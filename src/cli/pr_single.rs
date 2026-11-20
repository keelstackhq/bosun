//! `bosun pr --single [--ready]`: one PR, no task issue. Finds or opens the PR of the current
//! branch as a draft (title from `--title`, else the last commit subject) and, with `--ready`, runs
//! the flow of a task-less PR (`pr_gov::queue`): every ready gate stamped on HEAD, the ruleset
//! requires the gates, then the PR is marked ready and auto-merge is turned on. bosun never merges
//! (D13). Without a GitHub token the calls go to `next` for a cloud session to make.

use std::path::Path;
use std::process::Command;

use serde_json::{Value, json};

use super::PrArgs;
use super::pr::{current_branch, head_subject};
use super::pr_gov::{queue, refuse_task_pr, require_queueable};
use crate::app::{App, default_branch, kill_switch};
use crate::core::risk::{autonomous_allow_globs, matches_globs};
use crate::core::shape::ONE_OFF_MAX;
use crate::core::title::is_conventional;
use crate::github::http::Http;
use crate::github::{IssueState, Pull};
use crate::output::{BosunError, CmdResult, Outcome};

const RERUN: &str = "bosun pr --single --ready";

fn on_base(branch: &str, base: &str) -> bool {
    branch == base || ["main", "master", "HEAD"].contains(&branch)
}

fn base_usage(branch: &str, base: &str) -> BosunError {
    BosunError::usage(
        format!(
            "the current branch is {branch}, the default branch ({base}): there is no change to open a PR for"
        ),
        "check out your branch (for example `git switch -c claude/<change>`), commit and push, then run `bosun pr --single`",
    )
}

/// The title: `--title`, else the last commit's subject when it is a conventional commit message.
fn title_of(dir: &Path, args: &PrArgs) -> Result<String, BosunError> {
    let title = args
        .title
        .clone()
        .or_else(|| head_subject(dir).filter(|s| is_conventional(s)))
        .ok_or_else(|| {
            BosunError::usage(
                "no PR title: the last commit subject is not a conventional commit message",
                "pass `--title \"type: summary\"`, or amend the commit subject",
            )
        })?;
    super::refuse_secret("the PR title", &title)?;
    Ok(title)
}

pub fn run(dir: Option<&Path>, args: &PrArgs) -> CmdResult {
    if !Http::has_token() {
        return offline(dir, args);
    }
    let app = App::load(dir)?;
    let gh = app.gh.as_ref();
    kill_switch(gh)?;
    let branch = current_branch(&app.ctx.dir)?;
    let base = default_branch(gh)?;
    if on_base(&branch, &base) {
        return Err(base_usage(&branch, &base));
    }
    let head = format!("{}:{branch}", app.ctx.repo.owner);
    let open = gh
        .list_pulls(Some(&head), IssueState::Open)?
        .into_iter()
        .next();
    let (pull, created) = match open {
        Some(p) => {
            require_queueable(&app, &p, RERUN)?;
            refuse_task_pr(&app, &p)?;
            (p, false)
        }
        None => {
            // Only a definite "not found" stops here; an unreadable check (a proxy's 403) does not.
            let pushed = gh.branch_exists(&branch).unwrap_or_else(|e| {
                eprintln!("bosun: cannot check that {branch} is pushed: {}", e.message);
                true
            });
            if !pushed {
                return Err(BosunError::error(
                    format!("branch {branch} is not on GitHub yet"),
                    format!(
                        "push the branch first: git push -u origin {branch}, then run `bosun pr --single` again"
                    ),
                ));
            }
            super::trailers::check_local(&app.ctx.dir, &base, app.ctx.config.trailers.require)?;
            let title = title_of(&app.ctx.dir, args)?;
            (gh.create_pull(&title, &branch, &base, "", true)?, true)
        }
    };
    let reason = if args.ready {
        Some(single_reason(&app, &pull, &base)?)
    } else {
        None
    };
    let mut out = queue(&app, pull, &base, RERUN, args.ready)?;
    if let Some(r) = reason {
        out.data["single_reason"] = json!(r);
    }
    out.data["mode"] = json!("single");
    out.data["created"] = json!(created);
    Ok(out)
}

/// Why `--single --ready` may go ahead: `bootstrap` when the base has no `.bosun/` directory (the
/// PR that adds bosun; nothing there can judge it), or `one-off` when every changed path lies in
/// the one-off allow set and the change is within the one-off cap. Anything else is refused: the
/// one-off guards (allow set, R-oneoff-size) must not be bypassed by a task-less PR.
fn single_reason(app: &App, pull: &Pull, base: &str) -> Result<&'static str, BosunError> {
    let gh = app.gh.as_ref();
    // An unreadable or truncated tree is not proof of a bootstrap: fail closed.
    let tree = gh.tree(base).unwrap_or(None).filter(|t| !t.truncated);
    if tree.is_some_and(|t| {
        !t.entries
            .iter()
            .any(|e| e.path == ".bosun" || e.path.starts_with(".bosun/"))
    }) {
        return Ok("bootstrap");
    }
    let allow = crate::cli::risk_check::autonomous_allow(gh, base)?;
    let allow = autonomous_allow_globs(allow.as_deref());
    let extra = crate::cli::risk_check::oneoff_allow(gh, base)?;
    let ok = |f: &str| matches_globs(&allow, f) || matches_globs(&extra, f);
    let files = gh.pull_files(pull.number)?;
    let lines: u64 = files.iter().map(|f| f.additions + f.deletions).sum();
    let inside = files
        .iter()
        .all(|f| ok(&f.filename) && f.previous_filename.as_deref().is_none_or(ok));
    if inside && lines <= ONE_OFF_MAX {
        return Ok("one-off");
    }
    Err(BosunError::conflict(
        format!(
            "`--single --ready` is not allowed for PR #{}: the base has bosun, and the PR changes {lines} lines{}",
            pull.number,
            if inside {
                ""
            } else {
                " outside the one-off allow set"
            }
        ),
        "`--single --ready` is for the bootstrap PR or a one-off-sized change: use `bosun pr --one-off --title ... --touch ...`, or a plan task, or `bosun pr --pr <n> --ready` for a governance PR",
    ))
}

/// The default branch as the checkout knows it (`origin/HEAD`), else `main`; no GitHub is read.
fn local_base(dir: &Path) -> String {
    Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(["symbolic-ref", "--short", "refs/remotes/origin/HEAD"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .and_then(|b| b.strip_prefix("origin/").map(str::to_string))
        .filter(|b| !b.is_empty())
        .unwrap_or_else(|| "main".into())
}

/// No token: only the draft PR can be opened this way. After the local checks that need no
/// GitHub (the trailers), the create call goes to `next` (always a draft). `--ready` needs GitHub
/// (stamps on the PR, the ruleset, auto-merge: all bosun's own calls, since the guard refuses the
/// MCP auto-merge tool, D13), so it refuses with the login fix.
fn offline(dir: Option<&Path>, args: &PrArgs) -> CmdResult {
    let ctx = crate::ctx::Ctx::load(dir)?;
    let branch = current_branch(&ctx.dir)?;
    let base = local_base(&ctx.dir);
    if on_base(&branch, &base) {
        return Err(base_usage(&branch, &base));
    }
    if args.ready {
        return Err(BosunError::error(
            "no GitHub token: `--ready` cannot run",
            format!(
                "`--ready` needs GitHub: run `gh auth login` or export GITHUB_TOKEN, then rerun `{RERUN}`"
            ),
        ));
    }
    let title = title_of(&ctx.dir, args)?;
    super::trailers::check_local(&ctx.dir, &base, ctx.config.trailers.require)?;
    let note =
        "no GitHub token: open the draft PR with the call in `next` (the branch must be on GitHub)";
    eprintln!("bosun: {note}");
    let data: Value = json!({
        "pr": null, "task": null, "mode": "single", "created": false, "branch": branch,
        "draft": true, "ready": false, "auto_merge": false, "note": note,
    });
    Ok(Outcome::data(data).with_next(
        "mcp__github__create_pull_request",
        json!({
            "owner": ctx.repo.owner, "repo": ctx.repo.name, "head": branch, "base": base,
            "title": title, "body": "", "draft": true,
        }),
    ))
}
