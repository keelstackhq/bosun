//! `bosun pr <issue> [--auto-merge]`: open (or find) the PR of the current branch and, once every
//! required check is green, turn on auto-merge. bosun never merges (spec D13).

use std::path::Path;
use std::process::Command;

use serde_json::json;

use super::PrArgs;
use crate::app::{App, default_branch, kill_switch, set_state};
use crate::core::checks;
use crate::core::state::{State, state_of};
use crate::github::{Github, IssueState};
use crate::output::{BosunError, CmdResult, Outcome};

/// The default checks that must be green before auto-merge (spec D15, D32). `required` lets
/// `BOSUN_REQUIRED_CHECKS`, then `hoist.toml` `[github] required_checks`, override them.
pub const REQUIRED: [&str; 4] = checks::DEFAULT_REQUIRED;

fn required(root: &Path) -> Vec<String> {
    let hoist = std::fs::read_to_string(root.join("hoist.toml")).ok();
    checks::required_checks(
        std::env::var("BOSUN_REQUIRED_CHECKS").ok().as_deref(),
        hoist.as_deref(),
    )
}

fn current_branch(dir: &Path) -> Result<String, BosunError> {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(["rev-parse", "--abbrev-ref", "HEAD"])
        .output()
        .map_err(|e| BosunError::error(format!("cannot run git: {e}"), "install git"))?;
    let branch = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if !out.status.success() || branch.is_empty() {
        return Err(BosunError::error(
            "cannot read the current branch",
            "run bosun inside the repo checkout on your task branch",
        ));
    }
    Ok(branch)
}

/// `(context, state)` of every required check that is not green on `sha`.
fn not_green(gh: &dyn Github, root: &Path, sha: &str) -> Result<Vec<(String, String)>, BosunError> {
    let statuses = gh.statuses(sha)?;
    let runs = gh.check_runs(sha)?;
    let mut bad = Vec::new();
    for ctx in required(root) {
        let status = statuses.iter().find(|s| s.context == ctx);
        let run = runs.iter().find(|r| r.name == ctx);
        let green = status.is_some_and(|s| s.state == "success")
            || run.is_some_and(|r| r.conclusion.as_deref() == Some("success"));
        if !green {
            let state = status
                .map(|s| s.state.clone())
                .or_else(|| run.map(|r| r.conclusion.clone().unwrap_or_else(|| r.status.clone())))
                .unwrap_or_else(|| "missing".into());
            bad.push((ctx, state));
        }
    }
    Ok(bad)
}

pub fn run(dir: Option<&Path>, args: &PrArgs) -> CmdResult {
    let app = App::load(dir)?;
    let gh = app.gh.as_ref();
    let n = args.issue;
    kill_switch(gh)?;
    let issue = gh.get_issue(n)?.ok_or_else(|| {
        BosunError::error(
            format!("issue #{n} does not exist"),
            "check the number; `bosun status` lists the tasks",
        )
    })?;
    let branch = current_branch(&app.ctx.dir)?;
    let base = default_branch(gh)?;
    if branch == base || ["main", "master", "HEAD"].contains(&branch.as_str()) {
        return Err(BosunError::usage(
            format!("the current branch is {branch}; a PR needs a task branch"),
            "check out your task branch (for example `git switch -c claude/<task>`) and push it",
        ));
    }
    let head = format!("{}:{branch}", app.ctx.repo.owner);
    let existing = gh.list_pulls(Some(&head), IssueState::Open)?;
    let (pull, created) = match existing.into_iter().next() {
        Some(p) => (p, false),
        None => {
            let title = args.title.clone().unwrap_or_else(|| issue.title.clone());
            let body = format!("Closes #{n}\n\n<!-- bosun:task-pr {n} -->");
            (gh.create_pull(&title, &branch, &base, &body)?, true)
        }
    };
    let mut state = state_of(&issue.labels);
    if matches!(
        state,
        Some(State::Claimed | State::Building | State::Fixing)
    ) {
        set_state(gh, n, &issue, State::Review)?;
        state = Some(State::Review);
    }
    if args.auto_merge {
        if state != Some(State::Review) {
            let now = state.map_or("without a state".to_string(), |s| s.name().to_string());
            return Err(BosunError::conflict(
                format!("#{n} is {now}; auto-merge needs the task in review"),
                "an escalated or blocked task needs the owner: wait for the owner's answer, then rerun",
            ));
        }
        let bad = not_green(gh, &app.ctx.dir, &pull.head_sha)?;
        if !bad.is_empty() {
            let list: Vec<String> = bad.iter().map(|(c, s)| format!("{c}: {s}")).collect();
            return Err(BosunError::error(
                format!(
                    "PR #{} is not ready for auto-merge: {}",
                    pull.number,
                    list.join(", ")
                ),
                format!("`bosun wait --pr {}`, then rerun", pull.number),
            )
            .with_data(json!({
                "pr": pull.number, "head_sha": pull.head_sha,
                "not_green": bad.iter().map(|(c, s)| json!({"context": c, "state": s}))
                    .collect::<Vec<_>>(),
            })));
        }
        gh.enable_auto_merge(&pull.node_id, "SQUASH")?;
        let fresh = gh.get_issue(n)?.unwrap_or(issue);
        set_state(gh, n, &fresh, State::Queued)?;
        state = Some(State::Queued);
    }
    Ok(Outcome::data(json!({
        "pr": pull.number, "head_sha": pull.head_sha, "created": created,
        "auto_merge": args.auto_merge, "task_state": state.map(State::name),
    })))
}
