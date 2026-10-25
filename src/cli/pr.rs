//! `bosun pr <issue> [--auto-merge]`: open (or find) the PR of the current branch and, once every
//! required check is green, turn on auto-merge. bosun never merges (spec D13).

use std::path::Path;
use std::process::Command;

use serde_json::json;

use super::PrArgs;
use crate::app::{App, default_branch, kill_switch, set_state, sync_merged};
use crate::core::checks;
use crate::core::meta::linked_issues;
use crate::core::records::Record;
use crate::core::state::{State, state_of};
use crate::github::{Github, IssueState, enable_auto_merge_any};
use crate::output::{BosunError, CmdResult, Outcome};

/// The default checks that must be green before auto-merge (spec D15, D32). `required` lets
/// `BOSUN_REQUIRED_CHECKS` replace them, and `hoist.toml` `[github] required_checks` add to them.
pub const REQUIRED: [&str; 4] = checks::DEFAULT_REQUIRED;

/// Read from the default branch, so a PR cannot change the checks that judge it.
fn required(gh: &dyn Github, base: &str) -> Result<Vec<String>, BosunError> {
    let hoist = gh.file_at("hoist.toml", base)?;
    Ok(checks::required_checks(
        std::env::var("BOSUN_REQUIRED_CHECKS").ok().as_deref(),
        hoist.as_deref(),
    ))
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
fn not_green(gh: &dyn Github, base: &str, sha: &str) -> Result<Vec<(String, String)>, BosunError> {
    let statuses = gh.statuses(sha)?;
    let runs = gh.check_runs(sha)?;
    let mut bad = Vec::new();
    for ctx in required(gh, base)? {
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
    let mut found = gh.list_pulls(Some(&head), IssueState::All)?;
    found.sort_by_key(|p| p.state != "open");
    let open = found.iter().position(|p| p.state == "open");
    if open.is_none() {
        // Lists do not say `merged`; read each closed PR.
        for p in found
            .iter()
            .filter(|p| p.state == "closed" && linked_issues(&p.body).contains(&n))
        {
            if gh.get_pull(p.number)?.is_some_and(|f| f.merged) {
                let moved = sync_merged(gh, n, &issue, Some(&app.ctx.session))?;
                let task_state = if moved {
                    Some(State::Merged)
                } else {
                    state_of(&issue.labels)
                };
                return Ok(Outcome::data(json!({
                    "pr": p.number, "head_sha": p.head_sha, "created": false,
                    "auto_merge": false, "merged": true, "task_state": task_state.map(State::name),
                })));
            }
        }
    }
    let (pull, created) = match open.map(|i| found.swap_remove(i)) {
        Some(p) => (p, false),
        None => {
            let title = args.title.clone().unwrap_or_else(|| issue.title.clone());
            let body = format!("Closes #{n}\n\n<!-- bosun:task-pr {n} -->");
            (gh.create_pull(&title, &branch, &base, &body)?, true)
        }
    };
    let mut state = state_of(&issue.labels);
    // A PR exists only after the branch is pushed: claimed -> building is "first commit pushed".
    let mut issue = issue;
    if state == Some(State::Claimed) {
        set_state(gh, n, &issue, State::Building)?;
        issue = gh.get_issue(n)?.unwrap_or(issue);
        state = Some(State::Building);
    }
    if matches!(state, Some(State::Building | State::Fixing)) {
        set_state(gh, n, &issue, State::Review)?;
        state = Some(State::Review);
    }
    let mut route = None;
    if args.auto_merge {
        if state != Some(State::Review) {
            let now = state.map_or("without a state".to_string(), |s| s.name().to_string());
            return Err(BosunError::conflict(
                format!("#{n} is {now}; auto-merge needs the task in review"),
                "an escalated or blocked task needs the owner: wait for the owner's answer, then rerun",
            ));
        }
        let bad = not_green(gh, &base, &pull.head_sha)?;
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
        // Before auto-merge: a failed write leaves nothing turned on, so a rerun is clean.
        let record = Record::TaskPr {
            session: app.ctx.session.clone(),
            pr: pull.number,
        };
        gh.add_comment(
            n,
            &format!(
                "PR #{} queued for auto-merge\n\n{}",
                pull.number,
                record.to_line()
            ),
        )?;
        route = Some(enable_auto_merge_any(gh, &pull)?);
        let fresh = gh.get_issue(n)?.unwrap_or(issue);
        set_state(gh, n, &fresh, State::Queued)?;
        state = Some(State::Queued);
    }
    Ok(Outcome::data(json!({
        "pr": pull.number, "head_sha": pull.head_sha, "created": created,
        "auto_merge": args.auto_merge, "auto_merge_route": route, "task_state": state.map(State::name),
    })))
}
