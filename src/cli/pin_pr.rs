//! `bosun pin --latest --pr`: the pin PR in one step (spec D35). Checks first (clean tree on the
//! default branch, no open pin PR), then, after pin wrote its files: branch, commit, push, draft PR.
//! Never merges and never forces a push (D13).

use std::path::Path;
use std::process::Command;

use serde_json::{Value, json};

use crate::app::{App, default_branch, kill_switch};
use crate::github::IssueState;
use crate::output::{BosunError, CmdResult, Outcome};

const BRANCH: &str = "bosun/pin";

/// What `preflight` found.
pub enum Pre {
    /// Open pin PR `n`: nothing to do but merge it.
    Open(u64),
    /// Clean tree on the default branch: the app, the base, the branch to push.
    Go(Box<App>, String, String),
}

fn git(root: &Path, args: &[&str]) -> Result<String, BosunError> {
    // A runner with no git identity still commits, as bosun.
    let id = [
        "-c",
        "user.name=bosun",
        "-c",
        "user.email=bosun@users.noreply.github.com",
    ];
    let has_id = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["config", "user.email"])
        .output()
        .is_ok_and(|o| o.status.success())
        && Command::new("git")
            .arg("-C")
            .arg(root)
            .args(["config", "user.name"])
            .output()
            .is_ok_and(|o| o.status.success());
    let mut c = Command::new("git");
    c.arg("-C").arg(root);
    if !has_id {
        c.args(id);
    }
    let out = c.args(args).output().map_err(|e| {
        BosunError::error(format!("cannot run git: {e}"), "install git, then rerun")
    })?;
    if !out.status.success() {
        return Err(BosunError::error(
            format!(
                "git {} failed: {}",
                args.join(" "),
                String::from_utf8_lossy(&out.stderr).trim()
            ),
            "fix the git error above, then rerun `bosun pin --latest --pr`",
        ));
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// Before anything is downloaded or written.
pub fn preflight(root: &Path) -> Result<Pre, BosunError> {
    let app = App::load(Some(root))?;
    kill_switch(app.gh.as_ref())?;
    let own = |h: &str| h == BRANCH || h.starts_with(&format!("{BRANCH}-"));
    if let Some(p) = app
        .gh
        .list_pulls(None, IssueState::Open)?
        .iter()
        .find(|p| own(&p.head_ref))
    {
        return Ok(Pre::Open(p.number));
    }
    let base = default_branch(app.gh.as_ref())?;
    let on = git(root, &["rev-parse", "--abbrev-ref", "HEAD"])?;
    if on != base {
        return Err(BosunError::conflict(
            format!("on {on}, not the default branch {base}"),
            format!("run `git switch {base}` (up to date), then rerun"),
        ));
    }
    // `bosun/pin` is free, else one dated name (an earlier pin's branch has no open PR here).
    let taken = |b: &str| {
        git(
            root,
            &["rev-parse", "--verify", "-q", &format!("refs/heads/{b}")],
        )
        .is_ok()
            || git(root, &["ls-remote", "--exit-code", "--heads", "origin", b]).is_ok()
    };
    let day = crate::time::Timestamp::now().to_string().replace('-', "");
    let dated = format!("{BRANCH}-{}", &day[..8]);
    let branch = if !taken(BRANCH) {
        BRANCH.to_string()
    } else if !taken(&dated) {
        dated
    } else {
        return Err(BosunError::conflict(
            format!("{BRANCH} and {dated} already exist: they are left from earlier pins"),
            format!(
                "run `git branch -D {BRANCH} {dated}` and ask the owner to delete both on GitHub (Branches page), then rerun"
            ),
        ));
    };
    if !git(root, &["status", "--porcelain"])?.is_empty() {
        return Err(BosunError::conflict(
            "the working tree has changes",
            "commit or stash them, then rerun",
        ));
    }
    Ok(Pre::Go(Box::new(app), base, branch))
}

/// What is left for a human to do (agents and the orchestrator turn on auto-merge; the cloud
/// environment needs nothing).
pub fn left(pr: u64, bosun_to: Option<&str>) -> Vec<String> {
    let mut v = vec![format!(
        "Approve pin PR #{pr} (owner: it is gated; it edits the setup-script pins)."
    )];
    if let Some(to) = bosun_to {
        v.push(format!(
            "After it merges, on your own machine run `npm i -g @keelstackhq/bosun@{to}`."
        ));
    }
    v
}

pub fn publish(
    root: &Path,
    pre: Pre,
    files: &[&str],
    title: &str,
    body: &str,
    mut data: Value,
    bosun_to: Option<&str>,
) -> CmdResult {
    let (app, base, branch) = match pre {
        Pre::Open(n) => {
            data["pr"] = json!({"number": n});
            data["left"] = json!([format!(
                "Approve pin PR #{n} (owner); once it merges, rerun to move further."
            )]);
            return Ok(Outcome::data(data));
        }
        Pre::Go(a, b, c) => (a, b, c),
    };
    git(root, &["switch", "-q", "-c", &branch])?;
    let mut add = vec!["add", "--"];
    add.extend(files);
    let pushed = git(root, &add)
        .and_then(|_| git(root, &["commit", "-q", "-m", title]))
        .and_then(|_| git(root, &["push", "-q", "-u", "origin", &branch]));
    if let Err(e) = pushed {
        // The tree was clean, so only the pin edits are dropped: back to the base branch.
        let back = git(root, &["switch", "-q", "-f", &base])
            .and_then(|_| git(root, &["branch", "-q", "-D", &branch]));
        let fix = if back.is_ok() {
            format!("rolled back to {base}; {}", e.fix)
        } else {
            format!(
                "could not roll back: run `git switch -f {base}` and `git branch -D {branch}`, then rerun"
            )
        };
        return Err(BosunError { fix, ..e });
    }
    let pull = app
        .gh
        .create_pull(title, &branch, &base, body, true)
        .map_err(|e| BosunError {
            fix: format!(
                "{branch} is pushed: open its PR by hand with `gh pr create --draft --head {branch} --base {base} --title \"{title}\"`"
            ),
            ..e
        })?;
    data["branch"] = json!(branch);
    data["pr"] = json!({"number": pull.number, "head_sha": pull.head_sha});
    data["left"] = json!(left(pull.number, bosun_to));
    Ok(Outcome::data(data))
}
