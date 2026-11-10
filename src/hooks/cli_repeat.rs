//! The CLI's own repeat counter (spec §4): `<git common dir>/bosun/cli-repeat/<key>.json`,
//! one per session. The hook's repeat-lock sees one tool call for a whole script, so every
//! bosun call bumps this counter itself: the same call in a row warns at the 3rd run and is
//! refused (exit 6) from the 5th. Only under Claude Code and outside CI. Polls are not
//! counted; the next hooked tool call that no lock denies resets it.

use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use crate::core::refusal::mine;
use crate::core::repeat::{
    Repeat, Verdict, cli_command, cli_counts, is_poll, repeat_key, repeat_step, repeat_verdict,
};
use crate::hooks::git_out;
use crate::hooks::refusal::caller;
use crate::output::BosunError;

fn dir_of(dir: &Path) -> Option<PathBuf> {
    let common = git_out(dir, &["rev-parse", "--git-common-dir"])?;
    Some(dir.join(common).join("bosun").join("cli-repeat"))
}

/// Count this call (`args` without the program name); the refusal when it ran too often in a
/// row. A warning goes to stderr. Any I/O failure skips the count.
pub fn check(args: &[String]) -> Option<BosunError> {
    let env = |k: &str| std::env::var(k).ok().filter(|v| !v.is_empty());
    let ci = ["CI", "GITHUB_ACTIONS", "CIRCLECI"]
        .iter()
        .any(|k| env(k).is_some());
    let command = cli_command(args);
    if !cli_counts(env("CLAUDECODE").as_deref(), ci) || is_poll(&command) {
        return None;
    }
    // The shell's directory, not `-C`: the hook resets from the repo the session runs in.
    let store = dir_of(&std::env::current_dir().ok()?)?;
    let session = caller();
    let file = store.join(format!("{}.json", repeat_key(&session, "", "")));
    let prev: Option<Repeat> = std::fs::read(&file)
        .ok()
        .and_then(|b| serde_json::from_slice::<Value>(&b).ok())
        .and_then(|v| serde_json::from_value(v.get("repeat")?.clone()).ok());
    let step = repeat_step(prev, &command);
    let body = json!({"session": session, "repeat": step});
    let tmp = file.with_extension(format!("json.tmp{}", std::process::id()));
    let wrote = std::fs::create_dir_all(&store)
        .and_then(|()| std::fs::write(&tmp, body.to_string()))
        .and_then(|()| std::fs::rename(&tmp, &file));
    if let Err(e) = wrote {
        eprintln!("bosun: warning: repeat counter skipped: {e}");
        return None;
    }
    match repeat_verdict(step.count)? {
        Verdict::Warn(why) => {
            eprintln!(
                "bosun: warning: {}",
                why.replacen("repeat-lock", "cli-repeat", 1)
            );
            None
        }
        Verdict::Deny(why) => Some(BosunError::limit(
            format!(
                "`{}` ran {} times in a row with no other tool call (a script loop counts too)",
                step.command, step.count
            ),
            why.replacen("repeat-lock", "cli-repeat", 1),
        )),
    }
}

/// The hook's part: a tool call that runs ends the caller's runs in a row (its own and its
/// task sessions'). A Bash poll leaves them as they are.
pub fn reset(dir: &Path, tool: &str, tool_input: &Value) {
    let command = tool_input.get("command").and_then(Value::as_str);
    if tool == "Bash" && command.is_some_and(is_poll) {
        return;
    }
    let Some(store) = dir_of(dir) else {
        return;
    };
    let Ok(entries) = std::fs::read_dir(&store) else {
        return;
    };
    let me = caller();
    for path in entries.filter_map(|e| e.ok().map(|e| e.path())) {
        let session = std::fs::read(&path)
            .ok()
            .and_then(|b| serde_json::from_slice::<Value>(&b).ok())
            .and_then(|v| v.get("session")?.as_str().map(String::from));
        if session.is_some_and(|s| mine(&s, &me)) {
            let _ = std::fs::remove_file(path);
        }
    }
}
