//! `bosun pr <issue> [--auto-merge]`: open (or find) the PR of the current branch and, with
//! `--auto-merge`, turn on auto-merge at once; the ruleset's required checks gate the merge.
//! bosun never merges (spec D13).

use std::path::Path;
use std::process::Command;

use serde_json::json;

use super::PrArgs;
use super::reap::{EpicScope, close_done_epics};
use crate::app::{App, default_branch, kill_switch, notes, set_state, sync_merged};
use crate::core::approve::TRUSTED_CREATOR;
use crate::core::checks;
use crate::core::lease::{leases_of, orchestrator_of};
use crate::core::meta::{closing_issues, linked_issues, parse_task_meta};
use crate::core::records::Record;
use crate::core::state::{State, state_of};
use crate::core::title::is_conventional;
use crate::core::touch::{Changed, outside};
use crate::github::{
    CheckRun, Github, IssueState, Live, PrFile, Pull, Status, enable_auto_merge_any,
    is_already_clean, is_unstable,
};
use crate::output::{BosunError, CmdResult, Outcome};

/// The default checks the ruleset requires before a merge (spec D15, D32), reported as
/// `pending_checks` while they are not green. `required` lets `BOSUN_REQUIRED_CHECKS` replace
/// them, and `hoist.toml` `[github] required_checks` add to them; `bosun pr` does not need it.
pub const REQUIRED: [&str; 4] = checks::DEFAULT_REQUIRED;

/// Read from the default branch, so a PR cannot change the checks that judge it.
fn required(gh: &dyn Github, base: &str) -> Result<Vec<String>, BosunError> {
    let hoist = gh.file_at("hoist.toml", base)?;
    Ok(checks::required_checks(
        std::env::var("BOSUN_REQUIRED_CHECKS").ok().as_deref(),
        hoist.as_deref(),
    ))
}

/// Whether `user` has write or admin permission on the repo.
fn is_writer(gh: &dyn Github, user: &str) -> Result<bool, BosunError> {
    Ok(matches!(
        gh.permission(user)?.unwrap_or_default().as_str(),
        "admin" | "write"
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

/// The subject line of the head commit; `None` when git cannot say.
fn head_subject(dir: &Path) -> Option<String> {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(["log", "-1", "--format=%s"])
        .output()
        .ok()?;
    let subject = String::from_utf8_lossy(&out.stdout).trim().to_string();
    (out.status.success() && !subject.is_empty()).then_some(subject)
}

/// The state of check `ctx` on a head: a commit status, else a check run, else `missing`.
fn check_state(statuses: &[Status], runs: &[CheckRun], ctx: &str) -> String {
    let status = statuses.iter().find(|s| s.context == ctx);
    let run = runs.iter().find(|r| r.name == ctx);
    if status.is_some_and(|s| s.state == "success")
        || run.is_some_and(|r| r.conclusion.as_deref() == Some("success"))
    {
        return "success".into();
    }
    status
        .map(|s| s.state.clone())
        .or_else(|| run.map(|r| r.conclusion.clone().unwrap_or_else(|| r.status.clone())))
        .unwrap_or_else(|| "missing".into())
}

/// `(context, state)` of every required check that is not green on `sha` (information only).
fn not_green(gh: &dyn Github, base: &str, sha: &str) -> Result<Vec<(String, String)>, BosunError> {
    let statuses = gh.statuses(sha)?;
    let runs = gh.check_runs(sha)?;
    Ok(required(gh, base)?
        .into_iter()
        .map(|ctx| {
            let state = check_state(&statuses, &runs, &ctx);
            (ctx, state)
        })
        .filter(|(_, state)| state != "success")
        .collect())
}

/// The gates a merge relies on: `(context, state)` of `risk-check` and `review-verdict` on `sha`.
/// Only a commit status posted by `github-actions[bot]` counts; a check run or a status from
/// anyone else with the same name is ignored (`missing`), as in `risk-check` itself.
fn merge_gates(gh: &dyn Github, sha: &str) -> Result<Vec<(String, String)>, BosunError> {
    let statuses = gh.statuses(sha)?;
    Ok(["risk-check", "review-verdict"]
        .into_iter()
        .map(|ctx| {
            let state = statuses
                .iter()
                .find(|s| s.context == ctx && s.creator == TRUSTED_CREATOR)
                .map_or("missing", |s| s.state.as_str());
            (ctx.to_string(), state.to_string())
        })
        .collect())
}

/// The compare API lists at most 300 files (as in review_ci and approve).
const COMPARE_CAP: usize = 300;

/// Design C3: the PR may change only files in this session's live lease on `n` (plus lockfile
/// companions, added files and the fragments). `fetch` reads the PR's own file list, or the
/// comparison of the default branch with the pushed branch when no PR exists (with the number
/// of files the PR has, when GitHub says); it runs only once a lease is found. Returns the
/// `touch_check` value; exit 3 when files are outside.
fn touch_check(
    app: &App,
    n: u64,
    fetch: impl FnOnce() -> Result<(Vec<PrFile>, Option<u64>), BosunError>,
) -> Result<&'static str, BosunError> {
    let Some(lease) = leases_of(n, &notes(app.gh.as_ref(), n)?)
        .into_iter()
        // An empty touch (a spawn's epic lease) restricts nothing, as in the edit hook.
        .filter(|l| l.is_live(app.ctx.now) && !l.touch.is_empty())
        // Own lease first, else one held by a per-task id of this orchestrator.
        .filter(|l| l.session == app.ctx.session || orchestrator_of(&l.session) == app.ctx.session)
        .min_by_key(|l| l.session != app.ctx.session)
    else {
        return Ok("skipped (no lease)");
    };
    // Like the pushed-branch check: an unreadable diff does not stop the PR.
    let (files, total) = match fetch() {
        Ok(found) => found,
        Err(e) => {
            eprintln!(
                "bosun: cannot read the changed files of #{n}: {}",
                e.message
            );
            return Ok("skipped (diff unavailable)");
        }
    };
    let cut = match total {
        Some(total) => (files.len() as u64) < total,
        None => files.len() >= COMPARE_CAP,
    };
    if cut {
        return Ok("skipped (diff too large)");
    }
    let files: Vec<Changed> = files
        .into_iter()
        .map(|f| Changed {
            path: f.filename,
            status: f.status,
            previous: f.previous_filename,
        })
        .collect();
    let out = outside(&files, &lease.touch);
    if out.is_empty() {
        return Ok("ok");
    }
    Err(BosunError::conflict(
        format!("#{n} edits files outside its lease: {}", out.join(", ")),
        format!(
            "add them with `bosun claim {n} --touch <path>` (exit 3 if another session holds one), or drop the changes; then run `bosun pr {n}` again"
        ),
    )
    .with_data(json!({"outside": out})))
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
    // On the default branch or a detached HEAD there is no task branch: the PR is found by its
    // marker (below), so a session can queue its PR from any checkout.
    let on_base = branch == base || ["main", "master", "HEAD"].contains(&branch.as_str());
    let head = format!("{}:{branch}", app.ctx.repo.owner);
    let mut found = if on_base {
        Vec::new()
    } else {
        gh.list_pulls(Some(&head), IssueState::All)?
    };
    found.sort_by_key(|p| p.state != "open");
    let open = found.iter().position(|p| p.state == "open");
    if open.is_none() {
        // Lists do not say `merged`; read each closed PR.
        for p in found
            .iter()
            .filter(|p| p.state == "closed" && linked_issues(&p.body).contains(&n))
        {
            if gh
                .get_pull(p.number)?
                .is_some_and(|f| f.merged && f.base_ref == base)
            {
                let moved = sync_merged(gh, n, &issue, Some(&app.ctx.session))?;
                let task_state = if moved {
                    Some(State::Merged)
                } else {
                    state_of(&issue.labels)
                };
                let mut data = json!({
                    "pr": p.number, "head_sha": p.head_sha, "created": false,
                    "auto_merge": false, "merged": true, "task_state": task_state.map(State::name),
                });
                // The last merged task closes its epic (a root session working it directly never
                // runs `result`; a spawned child's lease leaves it to its `result`); housekeeping after the move: a failure is reported, not fatal.
                if moved && let Some(epic) = parse_task_meta(issue.body_text()).and_then(|m| m.epic)
                {
                    let mut closed = Vec::new();
                    let scope = EpicScope {
                        only: Some(epic),
                        quiet: false,
                        // A child's own epic lease blocks: its `result` closes the epic.
                        own_session: Some(&app.ctx.session),
                        own_spawned_blocks: true,
                        done: &[n],
                    };
                    let err = close_done_epics(gh, app.ctx.now, scope, &mut closed).err();
                    data["epic_closed"] = json!(!closed.is_empty());
                    if let Some(e) = err {
                        eprintln!("pr: closing the epic failed: {e}");
                        data["epic_error"] = json!(e.to_string());
                    }
                }
                return Ok(Outcome::data(data));
            }
        }
    }
    // No open PR on this branch: the open PR that carries this task's `bosun:task-pr` marker
    // is the task's PR, whatever branch it is on.
    // The marker is text the PR author controls, so it names a PR only together with a
    // `Closes #N`, a PR into the default branch from this repo (not a fork), and a writer as
    // author. Two such PRs are ambiguous: refuse.
    let marker = format!("<!-- bosun:task-pr {n} -->");
    let repo = app.ctx.repo.to_string();
    let mut by_marker = None;
    if open.is_none() {
        let mut ours: Vec<Pull> = Vec::new();
        for p in gh.list_pulls(None, IssueState::Open)? {
            if p.body.contains(&marker)
                && closing_issues(&p.body).contains(&n)
                && p.base_ref == base
                && p.head_repo.eq_ignore_ascii_case(&repo)
                && is_writer(gh, &p.user)?
            {
                ours.push(p);
            }
        }
        if ours.len() > 1 {
            let prs: Vec<u64> = ours.iter().map(|p| p.number).collect();
            return Err(BosunError::conflict(
                format!(
                    "{} open PRs carry the marker of task #{n}: {prs:?}",
                    prs.len()
                ),
                "close the extra PRs, or run `bosun pr` from the branch of the right one",
            )
            .with_data(json!({"prs": prs})));
        }
        by_marker = ours.pop();
    }
    if open.is_none() && by_marker.is_none() && on_base {
        return Err(BosunError::usage(
            format!(
                "the current branch is {branch} and no open PR of a writer into {base} carries the marker and `Closes #{n}`"
            ),
            "check out your task branch (for example `git switch -c claude/<task>`) and push it, or run `bosun pr` from the branch of the task's PR",
        ));
    }
    let branch = by_marker.as_ref().map_or(branch, |p| p.head_ref.clone());
    let touch;
    let (pull, created) = match open.map(|i| found.swap_remove(i)).or(by_marker) {
        Some(p) if !linked_issues(&p.body).contains(&n) => {
            // Before any state change, record or auto-merge: this PR belongs to something else.
            return Err(BosunError::conflict(
                format!(
                    "PR #{} on branch {branch} does not link task #{n}",
                    p.number
                ),
                format!(
                    "switch to the task's branch, or add `Closes #{n}` to the PR body and rerun"
                ),
            )
            .with_data(json!({"pr": p.number})));
        }
        Some(p) => {
            super::pr_bundle::refuse_gated_bundle(gh, &p)?;
            touch = touch_check(&app, n, || {
                let total = gh
                    .get_pull(p.number)?
                    .map_or(p.changed_files, |f| f.changed_files);
                Ok((gh.pull_files(p.number)?, (total > 0).then_some(total)))
            })?;
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
                        "push the branch first: git push -u origin {branch}, then run `bosun pr {n}` again"
                    ),
                ));
            }
            touch = touch_check(&app, n, || Ok((gh.compare_files(&base, &branch)?, None)))?;
            let title = args
                .title
                .clone()
                .or_else(|| head_subject(&app.ctx.dir).filter(|s| is_conventional(s)))
                .unwrap_or_else(|| issue.title.clone());
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
    let mut pending = Vec::new();
    let mut gates = Vec::new();
    let mut enforced = None;
    if args.auto_merge {
        if state != Some(State::Review) {
            let now = state.map_or("without a state".to_string(), |s| s.name().to_string());
            return Err(BosunError::conflict(
                format!("#{n} is {now}; auto-merge needs the task in review"),
                "an escalated or blocked task needs the owner: wait for the owner's answer, then rerun",
            ));
        }
        pending = not_green(gh, &base, &pull.head_sha)?;
        gates = merge_gates(gh, &pull.head_sha)?;
        // Auto-merge waits only for what the branch enforces: never turn it on for a branch
        // that would merge without the gates. A 403 proves nothing, so it does not block.
        enforced = Some(match gh.required_status_checks(&base)? {
            Live::Checks(on) => {
                let missing: Vec<&str> = (REQUIRED.iter().copied())
                    .filter(|c| !on.iter().any(|o| o == c))
                    .collect();
                if !missing.is_empty() {
                    return Err(BosunError::needs_human(
                        format!(
                            "{base} does not require {}, so auto-merge could merge PR #{} ungated",
                            missing.join(", "),
                            pull.number
                        ),
                        format!(
                            "an admin runs `hoist apply --area github` so the ruleset requires them, then rerun `bosun pr {n} --auto-merge`"
                        ),
                    )
                    .with_data(json!({
                        "pr": pull.number, "head_sha": pull.head_sha, "unenforced": missing,
                    })));
                }
                "verified"
            }
            Live::Partial(..) | Live::Forbidden => "unknown",
        });
        // Said before anything is turned on, and only once enforcement is known.
        let line: Vec<String> = gates.iter().map(|(c, s)| format!("{c}={s}")).collect();
        let why = if enforced == Some("verified") {
            "the branch requires them, so it merges only when both are success"
        } else {
            "could not verify that the branch requires them"
        };
        eprintln!(
            "bosun: auto-merge on PR #{} relies on the gates {}; {why}",
            pull.number,
            line.join(", ")
        );
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
        route = match enable_auto_merge_any(gh, &pull) {
            Ok(r) => Some(r),
            // Already mergeable: bosun never merges (D13), the task stays in review.
            Err(e) if is_already_clean(&e) => {
                return Err(BosunError::needs_human(
                    format!(
                        "PR #{} is already mergeable with every required check green, so GitHub cannot turn on auto-merge",
                        pull.number
                    ),
                    format!(
                        "the orchestrator merges PR #{0} (hand it over; you never merge), or its next push makes auto-merge possible again: push, then rerun `bosun pr {n} --auto-merge`",
                        pull.number
                    ),
                )
                .with_data(json!({
                    "pr": pull.number, "head_sha": pull.head_sha, "already_clean": true,
                })));
            }
            Err(e) if is_unstable(&e) => {
                return Err(BosunError::error(
                    format!("PR #{} has a failing check, so GitHub cannot turn on auto-merge", pull.number),
                    format!(
                        "`bosun wait --pr {0}` names the failing checks: fix them, push, then rerun `bosun pr {n} --auto-merge`",
                        pull.number
                    ),
                )
                .with_data(json!({"pr": pull.number, "head_sha": pull.head_sha, "unstable": true})));
            }
            Err(e) => return Err(e),
        };
        let fresh = gh.get_issue(n)?.unwrap_or(issue);
        set_state(gh, n, &fresh, State::Queued)?;
        state = Some(State::Queued);
    }
    Ok(Outcome::data(json!({
        "pr": pull.number, "head_sha": pull.head_sha, "created": created,
        "auto_merge": args.auto_merge, "auto_merge_route": route, "task_state": state.map(State::name),
        "touch_check": touch, "enforced_checks": enforced,
        "gates": gates.iter().map(|(c, s)| json!({"context": c, "state": s})).collect::<Vec<_>>(),
        "pending_checks": pending.iter().map(|(c, s)| json!({"context": c, "state": s}))
            .collect::<Vec<_>>(),
    })))
}
