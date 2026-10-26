//! `bosun hook pre-tool-use`: label-lock, merge-lock and edit-lock (spec §4, D13).

use std::path::Path;

use serde_json::{Value, json};

use super::{cache, git_out};
use crate::app::App;
use crate::core::guard::{edit_lock, edited_path, tool_rules};

/// The deny output of the hook protocol.
pub fn deny(reason: &str) -> String {
    json!({"hookSpecificOutput": {"hookEventName": "PreToolUse",
        "permissionDecision": "deny", "permissionDecisionReason": reason}})
    .to_string()
}

/// The hook fails closed on a bosun bug: a panic would exit 101, which Claude Code treats as a
/// non-blocking error and lets the tool call run.
pub fn run(cwd: Option<&Path>, input: &Value) -> Option<String> {
    guarded(|| judge(cwd, input))
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
    let path = edited_path(tool, &tool_input)?;
    match edit_reason(&dir, &path) {
        Ok(why) => why.map(|w| deny(&w)),
        Err(e) => {
            eprintln!("bosun: warning: edit-lock skipped: {e}");
            None
        }
    }
}

fn edit_reason(dir: &Path, path: &str) -> Result<Option<String>, String> {
    let app = App::load(Some(dir)).map_err(|e| e.message)?;
    let Some(rel) = repo_relative(&app.ctx.dir, dir, path) else {
        return Ok(None); // outside the repo: not a claimable path
    };
    let leases = cache::leases(&app, cache::path(dir).as_deref())?;
    Ok(edit_lock(&rel, &leases, &app.ctx.session))
}

/// `path` relative to the repo root; `None` when it lies outside the repo.
fn repo_relative(root: &Path, cwd: &Path, path: &str) -> Option<String> {
    let p = Path::new(path);
    let abs = if p.is_absolute() {
        p.to_path_buf()
    } else {
        cwd.join(p)
    };
    let strip = |a: &Path, r: &Path| a.strip_prefix(r).ok().map(Path::to_path_buf);
    let rel = strip(&abs, root).or_else(|| {
        let (a, r) = (abs.canonicalize().ok()?, root.canonicalize().ok()?);
        strip(&a, &r)
    })?;
    let text = rel.to_str()?.replace('\\', "/");
    (!text.starts_with("..")).then_some(text)
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
