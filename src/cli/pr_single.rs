//! `bosun pr --single [--ready]`: one PR, no task issue. Finds or opens the PR of the current
//! branch as a draft (title from `--title`, else the last commit subject) and, with `--ready`, runs
//! the flow of a task-less PR (`pr_gov::queue`): every ready gate stamped on HEAD, the ruleset
//! requires the gates, then the PR is marked ready and auto-merge is turned on. bosun never merges
//! (D13). Without a GitHub token the calls go to `next` for a cloud session to make.

use std::path::Path;
use std::process::Command;

use serde_json::{Value, json};

use super::PrArgs;
use super::pr::{current_branch, head_subject, missing_gates};
use super::pr_gov::{queue, refuse_task_pr, require_queueable};
use crate::app::{App, default_branch, kill_switch};
use crate::core::stamps::gate_loop;
use crate::core::title::is_conventional;
use crate::github::IssueState;
use crate::github::http::Http;
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
    let mut out = queue(&app, pull, &base, RERUN, args.ready)?;
    out.data["mode"] = json!("single");
    out.data["created"] = json!(created);
    Ok(out)
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

/// No token: with `--ready` the stamps are still checked here, then the call that opens the PR
/// goes to `next` (a PR made ready from the start); the session turns auto-merge on with the new
/// PR's number.
fn offline(dir: Option<&Path>, args: &PrArgs) -> CmdResult {
    let ctx = crate::ctx::Ctx::load(dir)?;
    let branch = current_branch(&ctx.dir)?;
    let base = local_base(&ctx.dir);
    if on_base(&branch, &base) {
        return Err(base_usage(&branch, &base));
    }
    let title = title_of(&ctx.dir, args)?;
    if args.ready {
        let missing = missing_gates(&ctx.dir)?;
        if !missing.is_empty() {
            return Err(BosunError::error(
                format!(
                    "not ready: gates not passed locally on this tree: {}",
                    missing.join(", ")
                ),
                format!(
                    "commit your work, then run `{}`; fix what fails, then rerun `{RERUN}`",
                    gate_loop(&missing)
                ),
            )
            .with_data(json!({"missing_gates": missing})));
        }
    }
    let note = if args.ready {
        "no GitHub token: push the branch, open the PR with the call in `next` (it is made ready), then turn on auto-merge for it with mcp__github__enable_pr_auto_merge (bosun never merges)"
    } else {
        "no GitHub token: push the branch, then open the draft PR with the call in `next`"
    };
    eprintln!("bosun: {note}");
    let data: Value = json!({
        "pr": null, "task": null, "mode": "single", "created": false, "branch": branch,
        "draft": !args.ready, "ready": if args.ready { json!("pending") } else { json!(false) },
        "auto_merge": false, "note": note,
    });
    Ok(Outcome::data(data).with_next(
        "mcp__github__create_pull_request",
        json!({
            "owner": ctx.repo.owner, "repo": ctx.repo.name, "head": branch, "base": base,
            "title": title, "body": "", "draft": !args.ready,
        }),
    ))
}
