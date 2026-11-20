//! The `gh` CLI as a fallback for a token, the login and opening a PR. `BOSUN_GH` names the
//! binary (default `gh` on PATH); `BOSUN_NO_GH=1` turns every use off (tests). A missing or
//! logged-out `gh` is never an error: callers get `None`.

use std::process::{Command, Stdio};

fn bin() -> Option<String> {
    if std::env::var("BOSUN_NO_GH").is_ok_and(|v| !v.trim().is_empty() && v.trim() != "0") {
        return None;
    }
    Some(
        std::env::var("BOSUN_GH")
            .ok()
            .filter(|v| !v.trim().is_empty())
            .unwrap_or_else(|| "gh".into()),
    )
}

/// Run `gh args`; `Some((success, trimmed stdout))`, `None` when gh cannot be started.
pub fn run(args: &[&str]) -> Option<(bool, String)> {
    let out = Command::new(bin()?)
        .args(args)
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    Some((
        out.status.success(),
        String::from_utf8_lossy(&out.stdout).trim().to_string(),
    ))
}

/// `gh auth token` (stdout trimmed); `None` when gh is missing, logged out or prints nothing.
pub fn auth_token() -> Option<String> {
    run(&["auth", "token"])
        .filter(|(ok, t)| *ok && !t.is_empty())
        .map(|(_, t)| t)
}

/// Whether `gh` is on PATH and logged in.
pub fn authenticated() -> bool {
    run(&["auth", "status"]).is_some_and(|(ok, _)| ok)
}

/// `gh api user --jq .login`.
pub fn login() -> Option<String> {
    run(&["api", "user", "--jq", ".login"])
        .filter(|(ok, t)| *ok && !t.is_empty())
        .map(|(_, t)| t)
}
