//! Claude Code hook entry points (spec §4, S8). Their stdout follows the hook protocol, not the
//! bosun envelope, and a hook never exits non-zero for a bosun-internal problem: it logs to
//! stderr and allows.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::Value;

use crate::cli::HookCmd;
use crate::output::{CmdResult, Outcome};

pub mod cache;
pub mod pre_tool_use;
pub mod session_start;

/// Run a hook with the process stdin; returns the text for stdout, if any.
pub fn run(dir: Option<&Path>, cmd: &HookCmd) -> Option<String> {
    let mut raw = String::new();
    if let Err(e) = std::io::stdin().read_to_string(&mut raw) {
        eprintln!("bosun: warning: cannot read the hook input: {e}");
    }
    let input: Value = match serde_json::from_str(&raw) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("bosun: warning: the hook input is not JSON ({e}); allowing");
            return None;
        }
    };
    // The hook's cwd is the session's directory; `-C` wins when given.
    let cwd = dir
        .map(Path::to_path_buf)
        .or_else(|| input.get("cwd").and_then(Value::as_str).map(PathBuf::from));
    match cmd {
        HookCmd::PreToolUse => pre_tool_use::run(cwd.as_deref(), &input),
        HookCmd::SessionStart => Some(session_start::run(cwd.as_deref())),
    }
}

/// The `lib::run` entry: print the hook's stdout (if any); `main` prints no envelope for hooks.
pub fn dispatch(dir: Option<&Path>, cmd: &HookCmd) -> CmdResult {
    if let Some(text) = run(dir, cmd) {
        println!("{text}");
    }
    Ok(Outcome::data(serde_json::json!({})))
}

/// Trimmed stdout of a successful `git -C dir args`.
pub(crate) fn git_out(dir: &Path, args: &[&str]) -> Option<String> {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .output()
        .ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).trim().to_string())
        .filter(|s| !s.is_empty())
}
