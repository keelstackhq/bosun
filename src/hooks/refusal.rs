//! Sticky refusal markers: `<git common dir>/bosun/refusals/<key>.json`, one per session and
//! command, next to the lease cache (spec §4). The CLI writes or clears them after a command;
//! the PreToolUse hook reads them. Any I/O failure is a stderr warning, never a failure.

use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use crate::core::refusal::{Refusal, clears, context, halted_deny, is_write, locks, mine, sticky};
use crate::core::repeat::repeat_key;
use crate::ctx::{hostname, session_id};
use crate::hooks::git_out;
use crate::output::CmdResult;

fn dir_of(dir: &Path) -> Option<PathBuf> {
    let common = git_out(dir, &["rev-parse", "--git-common-dir"])?;
    Some(dir.join(common).join("bosun").join("refusals"))
}

/// This process's session (D31), without loading the repo context.
pub(crate) fn caller() -> String {
    let env = |k: &str| std::env::var(k).ok();
    session_id(
        env("BOSUN_SESSION").as_deref(),
        env("CLAUDE_CODE_REMOTE_SESSION_ID").as_deref(),
        &hostname(),
    )
}

fn write(file: &Path, r: &Refusal) -> std::io::Result<()> {
    if let Some(parent) = file.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let tmp = file.with_extension(format!("json.tmp{}", std::process::id()));
    std::fs::write(&tmp, serde_json::to_vec(r)?)?;
    std::fs::rename(&tmp, file)
}

/// Every marker in the directory with its file.
fn all(dir: &Path) -> Vec<(PathBuf, Refusal)> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    entries
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x == "json"))
        .filter_map(|p| {
            let r = serde_json::from_slice(&std::fs::read(&p).ok()?).ok()?;
            Some((p, r))
        })
        .collect()
}

/// After a command (not a hook): a sticky refusal writes its marker; a success clears the
/// session's markers it answers.
pub fn record(dir: Option<&Path>, command: &str, result: &CmdResult) {
    let cwd = dir
        .map(Path::to_path_buf)
        .or_else(|| std::env::current_dir().ok());
    let Some(store) = cwd.as_deref().and_then(dir_of) else {
        return;
    };
    let session = caller();
    match result {
        Err(e) if sticky(command, e.exit.code()) => {
            let r = Refusal {
                session: session.clone(),
                command: command.into(),
                exit_code: e.exit.code(),
                fix: e.fix.clone(),
                at: now_text(),
                lock: locks(e.exit.code(), &e.message, &e.fix),
            };
            let file = store.join(format!("{}.json", repeat_key(&session, command, "")));
            let _ = std::fs::remove_file(shown_of(&file));
            if let Err(e) = write(&file, &r) {
                eprintln!("bosun: warning: cannot write the refusal marker: {e}");
            }
        }
        Ok(out) => {
            for (file, r) in all(&store) {
                if mine(&r.session, &session) && clears(&r, command, &out.data) {
                    let _ = std::fs::remove_file(&file);
                    let _ = std::fs::remove_file(shown_of(&file));
                }
            }
        }
        Err(_) => {}
    }
}

/// `<key>.shown`: holds the marker's bytes once the hook injected it. The hook never rewrites
/// the marker itself, so a marker a command removed cannot come back.
fn shown_of(file: &Path) -> PathBuf {
    file.with_extension("shown")
}

/// Whether the hook already injected this exact marker.
fn was_shown(file: &Path) -> bool {
    match (std::fs::read(file), std::fs::read(shown_of(file))) {
        (Ok(a), Ok(b)) => a == b,
        _ => false,
    }
}

fn mark_shown(file: &Path) -> std::io::Result<()> {
    let bytes = std::fs::read(file)?;
    let shown = shown_of(file);
    let tmp = shown.with_extension(format!("shown.tmp{}", std::process::id()));
    std::fs::write(&tmp, bytes)?;
    std::fs::rename(&tmp, shown)
}

fn now_text() -> String {
    let now = std::env::var("BOSUN_NOW").ok();
    crate::time::Timestamp::resolve(now.as_deref())
        .map_or_else(|_| String::new(), |t| t.to_string())
}

/// The hook's view: a deny for a write while one of the caller's markers is halted, else the
/// markers not yet shown as context (merged into `out`, which a deny keeps as it is).
pub fn apply(cwd: &Path, tool: &str, tool_input: &Value, out: Option<String>) -> Option<String> {
    let Some(store) = dir_of(cwd) else {
        return out;
    };
    let me = caller();
    let mut marks: Vec<_> = all(&store)
        .into_iter()
        .filter(|(_, r)| mine(&r.session, &me))
        .collect();
    if marks.is_empty() {
        return out;
    }
    marks.sort_by(|a, b| a.0.cmp(&b.0));
    let parsed: Option<Value> = out.as_deref().and_then(|t| serde_json::from_str(t).ok());
    let denied = parsed
        .as_ref()
        .is_some_and(|v| v["hookSpecificOutput"]["permissionDecision"] == "deny");
    if denied {
        return out;
    }
    if is_write(tool, tool_input)
        && let Some((_, r)) = marks.iter().find(|(_, r)| r.lock)
    {
        return Some(super::pre_tool_use::deny(&halted_deny(r)));
    }
    let mut lines: Vec<String> = Vec::new();
    for (file, r) in marks.iter().filter(|(f, _)| !was_shown(f)) {
        lines.push(context(r));
        if let Err(e) = mark_shown(file) {
            eprintln!("bosun: warning: cannot mark the refusal marker shown: {e}");
        }
    }
    if lines.is_empty() {
        return out;
    }
    let prior = parsed
        .as_ref()
        .and_then(|v| v["hookSpecificOutput"]["additionalContext"].as_str())
        .map(String::from);
    let text = prior
        .into_iter()
        .chain(lines)
        .collect::<Vec<_>>()
        .join("\n");
    Some(
        json!({"hookSpecificOutput": {"hookEventName": "PreToolUse", "additionalContext": text}})
            .to_string(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::output::BosunError;

    fn repo() -> tempfile::TempDir {
        let t = tempfile::tempdir().unwrap();
        let ok = std::process::Command::new("git")
            .args(["init", "-q"])
            .current_dir(t.path())
            .status()
            .unwrap();
        assert!(ok.success());
        t
    }

    #[test]
    fn needs_human_and_limit_write_markers_and_errors_do_not() {
        let t = repo();
        let store = dir_of(t.path()).unwrap();
        let err: CmdResult = Err(BosunError::error("x", "y"));
        record(Some(t.path()), "pr", &err);
        assert!(all(&store).is_empty());
        let five: CmdResult = Err(BosunError::needs_human("x", "ask the owner"));
        record(Some(t.path()), "pr", &five);
        let six: CmdResult = Err(BosunError::limit("x", "wait an hour"));
        record(Some(t.path()), "spawn", &six);
        let mut got: Vec<_> = all(&store)
            .into_iter()
            .map(|(_, r)| (r.command, r.exit_code))
            .collect();
        got.sort();
        assert_eq!(got, [("pr".into(), 5), ("spawn".into(), 6)]);
    }
}
