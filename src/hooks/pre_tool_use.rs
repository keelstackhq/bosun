//! `bosun hook pre-tool-use`: label-lock, merge-lock and edit-lock (spec §4, D13).

use std::path::Path;

use serde_json::{Value, json};

use super::{cache, git_out};
use crate::app::{App, trusted_notes};
use crate::core::guard::{
    ChildTask, Script, child_task_create, edit_lock, edited_path, foreign_lock, prereqs_lock,
    prereqs_lock_for, script_rules, tool_rules,
};
use crate::core::lease::leases_of;
use crate::core::repeat::{Verdict, is_poll, repeat_key, repeat_step, repeat_verdict};

/// The deny output of the hook protocol.
pub fn deny(reason: &str) -> String {
    json!({"hookSpecificOutput": {"hookEventName": "PreToolUse",
        "permissionDecision": "deny", "permissionDecisionReason": reason}})
    .to_string()
}

/// The hook fails closed on a bosun bug: a panic would exit 101, which Claude Code treats as a
/// non-blocking error and lets the tool call run.
pub fn run(cwd: Option<&Path>, input: &Value) -> Option<String> {
    guarded(|| {
        let out = judge(cwd, input);
        let dir = cwd
            .map(Path::to_path_buf)
            .or_else(|| std::env::current_dir().ok());
        let tool = input.get("tool_name").and_then(Value::as_str);
        match (dir, tool) {
            (Some(d), Some(t)) => {
                let tool_input = input.get("tool_input").unwrap_or(&Value::Null);
                super::refusal::apply(&d, t, tool_input, out)
            }
            _ => out,
        }
    })
}

/// Run `f`; a panic becomes a deny. While `f` runs, a panic prints one short line to stderr
/// (no payload, which may quote the command), so stdout keeps the hook protocol.
fn guarded(f: impl FnOnce() -> Option<String>) -> Option<String> {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(|info| {
        let at = info
            .location()
            .map(|l| format!(" at {l}"))
            .unwrap_or_default();
        eprintln!("bosun: internal error in the PreToolUse hook{at}; denying");
    }));
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(f));
    std::panic::set_hook(previous);
    result.unwrap_or_else(|_| {
        Some(deny(
            "the bosun hook could not judge this command (internal error); rerun it in a \
             simpler form or report a bosun bug",
        ))
    })
}

fn judge(cwd: Option<&Path>, input: &Value) -> Option<String> {
    let tool = input.get("tool_name").and_then(Value::as_str)?;
    let tool_input = input.get("tool_input").cloned().unwrap_or(Value::Null);
    let dir = cwd
        .map(Path::to_path_buf)
        .or_else(|| std::env::current_dir().ok())?;
    // The branch where a git command acts: the hook's directory moved by the command's own
    // `cd dir &&` and `git -C dir` (a worktree on a feature branch is not the default branch).
    let branch_at = |opts: &[String]| {
        let mut args: Vec<&str> = opts.iter().map(String::as_str).collect();
        args.extend(["rev-parse", "--abbrev-ref", "HEAD"]);
        git_out(&dir, &args)
    };
    let default = git_out(
        &dir,
        &["symbolic-ref", "--short", "refs/remotes/origin/HEAD"],
    )
    .map(|b| b.strip_prefix("origin/").unwrap_or(&b).to_string())
    .unwrap_or_else(|| "main".into());
    if let Some(why) = tool_rules(tool, &tool_input, &branch_at, &default) {
        return Some(deny(&why));
    }
    if let Some(why) = prereqs_lock(tool, &tool_input) {
        // A registered child may create a new issue with a task marker for its own epic, and
        // nothing else: not edits, comments, other markers, other repos or the allow and
        // capabilities keys.
        let child_ok = prereqs_lock_for(tool, &tool_input, true).is_none()
            && child_task_create(tool, &tool_input)
                .is_ok_and(|write| registered_child(&dir, &write));
        if !child_ok {
            return Some(deny(&why));
        }
    }
    if tool == "Bash" {
        // Script files are read last, so every other lock has judged the command first.
        let repeat = repeat_lock(&dir, input, &tool_input);
        let command = tool_input
            .get("command")
            .and_then(Value::as_str)
            .unwrap_or("");
        let read = |p: &str| read_script(&dir, p);
        if let Some(why) = script_rules(command, &read, &branch_at, &default) {
            return Some(deny(&why));
        }
        return repeat;
    }
    if let Some(file) = repeat_file(&dir, input) {
        // Another tool call (an edit between test runs) ends the run of repeats.
        let _ = std::fs::remove_file(file);
    }
    let path = edited_path(tool, &tool_input)?;
    match edit_reason(&dir, &path) {
        Ok(why) => why.map(|w| deny(&w)),
        Err(e) => {
            eprintln!("bosun: warning: edit-lock skipped: {e}");
            None
        }
    }
}

/// A script the merge-lock scans, relative to `dir`: at most 1 MiB is read (procfs files report
/// size 0, so the size is not trusted), as lossy UTF-8. A directory or a missing path is
/// `None`; a device, FIFO, socket, a file under /proc, /sys or /dev, or more than 1 MiB is
/// `Unreadable`. A denial may quote a script line, as for any command.
fn read_script(dir: &Path, path: &str) -> Option<Script> {
    use std::io::Read;
    const MAX: u64 = 1 << 20;
    let file = dir.join(path);
    let meta = std::fs::metadata(&file).ok()?;
    if meta.is_dir() {
        return None;
    }
    let real = std::fs::canonicalize(&file).unwrap_or(file.clone());
    if ["/proc", "/sys", "/dev"]
        .iter()
        .any(|r| real.starts_with(r))
    {
        return Some(Script::Unreadable("a /proc, /sys or /dev file"));
    }
    if !meta.is_file() {
        return Some(Script::Unreadable("not a regular file"));
    }
    let mut buf = Vec::new();
    let read = std::fs::File::open(&file).and_then(|f| f.take(MAX + 1).read_to_end(&mut buf));
    match read {
        Err(_) => Some(Script::Unreadable("it cannot be opened")),
        Ok(n) if n as u64 > MAX => Some(Script::Unreadable("larger than 1 MiB")),
        Ok(_) => Some(Script::Text(String::from_utf8_lossy(&buf).into_owned())),
    }
}

/// `<git common dir>/bosun/repeat/<key>.json` of the session, agent and directory; `None`
/// without a session or a git repo.
fn repeat_file(dir: &Path, input: &Value) -> Option<std::path::PathBuf> {
    let text = |k: &str| input.get(k).and_then(Value::as_str).map(String::from);
    let session = text("session_id").or_else(|| std::env::var("BOSUN_SESSION").ok());
    let session = session.filter(|s| !s.trim().is_empty())?;
    let agent = text("agent_id").unwrap_or_default();
    let key = repeat_key(&session, &agent, &dir.to_string_lossy());
    let common = git_out(dir, &["rev-parse", "--git-common-dir"])?;
    Some(dir.join(common).join(format!("bosun/repeat/{key}.json")))
}

/// repeat-lock: count a Bash call that is not a poll (written atomically); a warning at the
/// third run in a row, a deny from the fifth. Any I/O failure skips the lock.
fn repeat_lock(dir: &Path, input: &Value, tool_input: &Value) -> Option<String> {
    let command = tool_input.get("command").and_then(Value::as_str)?;
    let file = (!is_poll(command))
        .then(|| repeat_file(dir, input))
        .flatten()?;
    let prev = serde_json::from_slice(&std::fs::read(&file).unwrap_or_default()).ok();
    let step = repeat_step(prev, command);
    let tmp = file.with_extension(format!("json.tmp{}", std::process::id()));
    let wrote = (file.parent().map_or(Ok(()), std::fs::create_dir_all))
        .and_then(|()| std::fs::write(&tmp, serde_json::to_vec(&step)?))
        .and_then(|()| std::fs::rename(&tmp, &file));
    if let Err(e) = wrote {
        eprintln!("bosun: warning: repeat-lock skipped: {e}");
        return None;
    }
    match repeat_verdict(step.count)? {
        Verdict::Deny(why) => Some(deny(&why)),
        Verdict::Warn(why) => Some(
            json!({"hookSpecificOutput": {"hookEventName": "PreToolUse", "additionalContext": why}})
                .to_string(),
        ),
    }
}

/// Whether this session is a registered child for `write.epic`: the epic carries a live lease
/// for this session with origin `spawned` and an empty touch, which only `bosun spawn --record`
/// writes (a claim has touch paths), in a comment of a repo writer (`trusted_notes`, not any
/// comment). An MCP call must also target this repo. Any doubt (no repo, no GitHub) is no.
fn registered_child(dir: &Path, write: &ChildTask) -> bool {
    let Ok(app) = App::load(Some(dir)) else {
        return false;
    };
    if let Some((owner, name)) = &write.repo
        && !(owner.eq_ignore_ascii_case(&app.ctx.repo.owner)
            && name.eq_ignore_ascii_case(&app.ctx.repo.name))
    {
        return false;
    }
    let Ok(notes) = trusted_notes(app.gh.as_ref(), write.epic) else {
        return false;
    };
    leases_of(write.epic, &notes).iter().any(|l| {
        l.session == app.ctx.session
            && l.origin == "spawned"
            && l.touch.is_empty()
            && l.is_live(app.ctx.now)
    })
}

fn edit_reason(dir: &Path, path: &str) -> Result<Option<String>, String> {
    let app = App::load(Some(dir)).map_err(|e| e.message)?;
    let Some(rel) = repo_relative(&app.ctx.dir, dir, path) else {
        return Ok(None); // outside the repo: not a claimable path
    };
    let file = cache::path(dir);
    let leases = cache::leases(&app, file.as_deref())?;
    // Assume the file exists first: git is asked only when that would deny.
    let me = &app.ctx.session;
    let Some(why) = edit_lock(&rel, true, &leases, me) else {
        return Ok(None);
    };
    if foreign_lock(&rel, &leases, me).is_some() {
        return Ok(Some(why));
    }
    let exists = app.ctx.dir.join(&rel).is_file() && on_base(&app.ctx.dir, &rel);
    if !exists {
        return Ok(None);
    }
    // The own-touch rule may rest on a stale cache (a claim that grew the touch): look again
    // before denying.
    match cache::refresh(&app, file.as_deref()) {
        Ok(fresh) => Ok(edit_lock(&rel, exists, &fresh, me)),
        Err(e) => {
            eprintln!(
                "bosun: could not re-check leases: {}; using the cached view",
                e.message
            );
            Ok(Some(why))
        }
    }
}

/// Whether `rel` is on the default branch (`origin/HEAD`, else `origin/main`, else
/// `origin/master`): a file added on this branch counts as new, so Write then Edit works. With
/// no such ref the file counts as new (fail open). Known limit: case-insensitive file systems.
fn on_base(root: &Path, rel: &str) -> bool {
    let known = |r: &str| git_out(root, &["rev-parse", "--verify", "--quiet", r]).is_some();
    let head = git_out(
        root,
        &["symbolic-ref", "--short", "refs/remotes/origin/HEAD"],
    );
    let Some(base) = head
        .into_iter()
        .chain(["origin/main".to_string(), "origin/master".to_string()])
        .find(|r| known(r))
    else {
        return false;
    };
    std::process::Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["cat-file", "-e", &format!("{base}:{rel}")])
        .output()
        .is_ok_and(|o| o.status.success())
}

/// `path` relative to the repo root, resolved with `canonicalize` when the file or its parent
/// exists, else `.` and `..` lexically; `None` when it lies outside the repo.
fn repo_relative(root: &Path, cwd: &Path, path: &str) -> Option<String> {
    use std::path::Component;
    let p = Path::new(path);
    let joined = if p.is_absolute() {
        p.to_path_buf()
    } else {
        cwd.join(p)
    };
    // The real path first, so symlinks and `link/../x` are judged by their target: the file
    // itself, else (a new file) its existing parent. Outside the repo: not a claimable path.
    let real = joined.canonicalize().ok().or_else(|| {
        let parent = joined.parent()?.canonicalize().ok()?;
        Some(parent.join(joined.file_name()?))
    });
    if let Some(real) = real {
        let r = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
        let rel = real.strip_prefix(&r).ok()?;
        let text = rel.to_str()?.replace('\\', "/");
        return (!rel.components().any(|c| c == Component::ParentDir)).then_some(text);
    }
    let mut abs = std::path::PathBuf::new();
    for c in joined.components() {
        match c {
            Component::CurDir => {}
            Component::ParentDir => {
                if !abs.pop() {
                    return None;
                }
            }
            other => abs.push(other),
        }
    }
    let strip = |a: &Path, r: &Path| a.strip_prefix(r).ok().map(Path::to_path_buf);
    let rel = strip(&abs, root).or_else(|| {
        let (a, r) = (abs.canonicalize().ok()?, root.canonicalize().ok()?);
        strip(&a, &r)
    })?;
    let text = rel.to_str()?.replace('\\', "/");
    (!rel.components().any(|c| c == Component::ParentDir)).then_some(text)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_panic_in_the_guard_denies() {
        let out = guarded(|| panic!("boom in {}", "gh issue edit 1"));
        let v: Value = serde_json::from_str(&out.unwrap()).unwrap();
        let hso = &v["hookSpecificOutput"];
        assert_eq!(hso["permissionDecision"], "deny");
        let why = hso["permissionDecisionReason"].as_str().unwrap();
        assert!(why.contains("internal error"), "{why}");
        assert!(!why.contains("boom"), "{why}");
        // No panic: the verdict passes through, and a later panic is caught again.
        assert_eq!(guarded(|| None), None);
        assert_eq!(guarded(|| Some("x".into())), Some("x".into()));
        assert!(guarded(|| panic!("again")).is_some());
    }
}
