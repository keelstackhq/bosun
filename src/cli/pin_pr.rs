//! `bosun pin --latest --pr`: the pin PR in one step (spec D35). Checks first (clean tree on the
//! default branch, no open pin PR), then, after pin wrote its files: branch, commit, push, draft PR.
//! Never merges and never forces a push (D13).

use std::path::Path;
use std::process::Command;

use serde_json::{Value, json};

use crate::app::{App, default_branch};
use crate::github::IssueState;
use crate::output::{BosunError, CmdResult, Outcome};

const BRANCH: &str = "bosun/pin";

/// What `preflight` found.
pub enum Pre {
    /// Open pin PR `n`: nothing to do but merge it.
    Open(u64),
    /// Clean tree on the default branch.
    Go(Box<App>, String),
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
            format!(
                "fix the git error above; if {BRANCH} is stale run `git branch -D {BRANCH}`; then rerun `bosun pin --latest --pr`"
            ),
        ));
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// Before anything is downloaded or written.
pub fn preflight(root: &Path) -> Result<Pre, BosunError> {
    let app = App::load(Some(root))?;
    let head = format!("{}:{BRANCH}", app.ctx.repo.owner);
    if let Some(p) = app.gh.list_pulls(Some(&head), IssueState::Open)?.first() {
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
    let stale = if git(
        root,
        &[
            "rev-parse",
            "--verify",
            "-q",
            &format!("refs/heads/{BRANCH}"),
        ],
    )
    .is_ok()
    {
        Some((
            "locally",
            format!("run `git branch -D {BRANCH}`, then rerun"),
        ))
    } else if git(
        root,
        &["ls-remote", "--exit-code", "--heads", "origin", BRANCH],
    )
    .is_ok()
    {
        Some((
            "on origin",
            format!("ask the owner to delete {BRANCH} on GitHub (Branches page), then rerun"),
        ))
    } else {
        None
    };
    if let Some((where_, fix)) = stale {
        return Err(BosunError::conflict(
            format!("{BRANCH} already exists {where_}: it is left from an earlier pin"),
            fix,
        ));
    }
    if !git(root, &["status", "--porcelain"])?.is_empty() {
        return Err(BosunError::conflict(
            "the working tree has changes",
            "commit or stash them, then rerun",
        ));
    }
    Ok(Pre::Go(Box::new(app), base))
}

/// What is left for the human, in order.
pub fn left(pr: u64, bosun_to: Option<&str>) -> Vec<String> {
    let mut v = vec![format!(
        "PR #{pr}: mark it ready for review and get the owner approval (it edits .bosun/ pins); GitHub merges on green."
    )];
    v.push("Cloud environment: nothing. The next session start runs the merged docs/setup-script.sh; the pasted snippet never changes.".into());
    if let Some(to) = bosun_to {
        v.push(format!(
            "This machine: after the merge run `npm i -g @keelstackhq/bosun@{to}`."
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
    let (app, base) = match pre {
        Pre::Open(n) => {
            data["pr"] = json!({"number": n});
            data["left"] = json!([format!(
                "PR #{n} is the open pin PR: merge it (owner approval, GitHub merges on green), then rerun to move further."
            )]);
            return Ok(Outcome::data(data));
        }
        Pre::Go(a, b) => (a, b),
    };
    git(root, &["switch", "-q", "-c", BRANCH])?;
    let mut add = vec!["add", "--"];
    add.extend(files);
    let pushed = git(root, &add)
        .and_then(|_| git(root, &["commit", "-q", "-m", title]))
        .and_then(|_| git(root, &["push", "-q", "-u", "origin", BRANCH]));
    if let Err(e) = pushed {
        // The tree was clean, so only the pin edits are dropped: back to the base branch.
        let back = git(root, &["switch", "-q", "-f", &base])
            .and_then(|_| git(root, &["branch", "-q", "-D", BRANCH]));
        let fix = if back.is_ok() {
            format!("rolled back to {base}; {}", e.fix)
        } else {
            format!(
                "could not roll back: run `git switch -f {base}` and `git branch -D {BRANCH}`, then rerun"
            )
        };
        return Err(BosunError { fix, ..e });
    }
    let pull = app
        .gh
        .create_pull(title, BRANCH, &base, body, true)
        .map_err(|e| BosunError {
            fix: format!(
                "{BRANCH} is pushed: open its PR by hand with `gh pr create --draft --head {BRANCH} --base {base} --title \"{title}\"`"
            ),
            ..e
        })?;
    data["pr"] = json!({"number": pull.number, "head_sha": pull.head_sha});
    data["left"] = json!(left(pull.number, bosun_to));
    Ok(Outcome::data(data))
}
