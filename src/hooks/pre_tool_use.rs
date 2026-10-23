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

pub fn run(cwd: Option<&Path>, input: &Value) -> Option<String> {
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
