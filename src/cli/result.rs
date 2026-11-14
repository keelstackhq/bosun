//! `bosun result`: a child hands its result to its parent (spec D14).

use std::path::Path;

use serde_json::json;

use super::ResultArgs;
use super::reap::{EpicScope, close_done_epics};
use crate::app::{
    App, default_branch, end_lease, halt_reason, merge_counts, set_state, sync_merged,
    task_pr_merged, trusted_notes,
};
use crate::core::lease::{leases_of, members};
use crate::core::meta::{linked_issues, parse_task_meta};
use crate::core::records::Record;
use crate::core::state::{State, state_of, transition_allowed};
use crate::github::IssueState;
use crate::output::{BosunError, CmdResult, Outcome};
use crate::registry;

pub fn run(dir: Option<&Path>, args: &ResultArgs) -> CmdResult {
    // No kill switch check: reporting a result is how a halted child hands back.
    let app = App::load(dir)?;
    let (gh, n, session) = (app.gh.as_ref(), args.epic, &app.ctx.session);
    let Some(epic) = gh.get_issue(n)? else {
        return Err(BosunError::error(
            format!("epic #{n} does not exist"),
            "check the epic number; it is the issue in your prompt",
        ));
    };

    let pr_text = args.pr.map_or(String::new(), |p| format!(", PR #{p}"));
    let record = Record::Result {
        session: session.clone(),
        outcome: args.outcome.clone(),
        pr: args.pr,
    };
    let body = format!(
        "Result of {session}: {}{pr_text}\n\n{}",
        args.outcome,
        record.to_line()
    );
    let comment_id = gh.add_comment(n, &body)?.id;

    // `merged` is only a report: the merge itself moves the state (D13). The other outcomes set
    // their state, as this ends the lease and nothing else would move the epic (spec §5);
    // `partial` hands the epic back as ready, so the parent can spawn again for the rest.
    let mut state = state_of(&epic.labels);
    let mut partial_skipped = None;
    let to = match args.outcome.as_str() {
        "blocked" => Some(State::Blocked),
        "needs-human" => Some(State::NeedsHuman),
        "partial" => match partial_refusal(&app, n, state)? {
            None => Some(State::Ready),
            Some(why) => {
                partial_skipped = Some(why);
                None
            }
        },
        _ => None,
    };
    // A late outcome on an epic that already closed (or any refused move) is never dropped
    // silently: the comment above records it and `data.state_skipped` says why it did not apply.
    let mut state_skipped = None;
    if let Some(to) = to
        && state != Some(to)
    {
        if state.is_none_or(|from| transition_allowed(from, to)) {
            set_state(gh, n, &epic, to)?;
            state = Some(to);
        } else {
            let from = state.map_or("none".into(), |s| s.to_string());
            let why = format!("#{n} is {from}; {} cannot move it to {to}", args.outcome);
            eprintln!("result: {why}");
            state_skipped = Some(why);
        }
    }
    let epic_already_closed = epic.state == "closed";

    // A merged PR moves the tasks it closes (the merge moves state, D13); an unmerged one is a report.
    let mut tasks_merged = Vec::new();
    if let Some(pr) = args.pr
        && let Some(pull) = gh.get_pull(pr)?
        && pull.merged
        && pull.base_ref == default_branch(gh)?
    {
        for task in linked_issues(&pull.body) {
            if let Some(issue) = gh.get_issue(task)?
                && merge_counts(&issue)
                && sync_merged(gh, task, &issue, Some(session))?
            {
                tasks_merged.push(task);
            }
        }
    }
    // Every unfinished task of the epic whose own PR (task-pr record, else bosun marker) merged
    // moves to merged, so the parent sees the merge without waiting for `reap`.
    let base = default_branch(gh)?;
    let mut pulls = None;
    for st in [State::Building, State::Review, State::Fixing, State::Queued] {
        for task in gh.list_issues(&[&st.label()], IssueState::All)? {
            if task.pull_request
                || task.number == n
                || !parse_task_meta(task.body_text()).is_some_and(|m| m.epic == Some(n))
                || tasks_merged.contains(&task.number)
            {
                continue;
            }
            if task_pr_merged(
                gh,
                &app.ctx.repo.to_string(),
                &base,
                task.number,
                &mut pulls,
                true,
            )? && sync_merged(gh, task.number, &task, None)?
            {
                tasks_merged.push(task.number);
            }
        }
    }
    // Plus every merged task of the epic, whatever the outcome: the parent sees what is done.
    tasks_merged.extend(
        gh.list_issues(&[&State::Merged.label()], IssueState::All)?
            .iter()
            .filter(|i| !i.pull_request && i.number != n)
            .filter(|i| parse_task_meta(i.body_text()).is_some_and(|m| m.epic == Some(n)))
            .map(|i| i.number),
    );
    tasks_merged.sort_unstable();
    tasks_merged.dedup();

    let lease_ended = leases_of(n, &trusted_notes(gh, n)?)
        .iter()
        .any(|l| &l.session == session);
    if lease_ended {
        end_lease(gh, n, session, "result")?;
    }

    let mut left = false;
    if let Some((number, reg)) = registry::load(gh)?
        && members(&reg).iter().any(|m| &m.session == session)
    {
        let leave = Record::Leave {
            session: session.clone(),
            reason: "done".into(),
        };
        gh.add_comment(number, &leave.to_line())?;
        left = true;
    }

    // Closing the epic is housekeeping after the comment: a failure is reported, never fatal.
    let mut closed = Vec::new();
    let scope = EpicScope {
        only: Some(n),
        quiet: false,
        own_session: Some(session),
        own_spawned_blocks: false,
        done: &tasks_merged,
    };
    let epic_error = close_done_epics(gh, app.ctx.now, scope, &mut closed).err();
    let mut data = json!({
        "epic": n, "outcome": args.outcome, "pr": args.pr,
        "comment_id": comment_id, "lease_ended": lease_ended, "left": left,
        "task_state": state.map(State::name), "tasks_merged": tasks_merged,
        "partial_skipped": partial_skipped, "state_skipped": state_skipped,
        "epic_already_closed": epic_already_closed,
        "epic_closed": !closed.is_empty(),
    });
    if let Some(e) = epic_error {
        eprintln!("result: closing the epic failed: {e}");
        data["epic_error"] = json!(e.to_string());
    }
    // Green PRs of this work wait for the parent: bosun never merges (D13) and a child cannot
    // (D-008), so the result names them and hands the parent a message to finish them.
    let green = green_prs(&app, n, args.pr)?;
    data["ready_to_merge"] = json!(
        green
            .iter()
            .map(|(p, sha)| json!({"pr": p, "head_sha": sha}))
            .collect::<Vec<_>>()
    );
    let parent = parent_session(&app)?;
    data["parent"] = json!(parent);
    let mut out = Outcome::data(data);
    if let Some(parent) = parent {
        let message = parent_message(session, n, &args.outcome, args.pr, &green);
        out = out.with_next(
            "send_message",
            json!({"session_id": parent, "message": message}),
        );
    }
    Ok(out)
}

/// Open, non-draft PRs of epic `n` (its own, or linking one of its tasks, or `pr`) that GitHub
/// reports `clean`: every required check is green, so nothing but a merge is left.
fn green_prs(app: &App, n: u64, pr: Option<u64>) -> crate::app::Result<Vec<(u64, String)>> {
    let gh = app.gh.as_ref();
    let repo = app.ctx.repo.to_string();
    let mut out = Vec::new();
    for p in gh.list_pulls(None, IssueState::Open)?.into_iter().take(100) {
        let mut mine = Some(p.number) == pr;
        for i in crate::core::meta::linked_issues_in(&p.body, &repo) {
            if i == n {
                mine = true;
            } else if let Some(t) = gh.get_issue(i)?
                && parse_task_meta(t.body_text()).is_some_and(|m| m.epic == Some(n))
            {
                mine = true;
            }
        }
        if !mine || p.draft {
            continue;
        }
        if let Some(d) = gh.get_pull(p.number)?
            && d.state == "open"
            && !d.draft
            && d.mergeable_state.as_deref() == Some("clean")
        {
            out.push((d.number, d.head_sha));
        }
    }
    out.sort();
    Ok(out)
}

/// The session that spawned this one (registry record); `None` for a root or a wave agent
/// (which runs inside its orchestrator, so a message would reach itself).
fn parent_session(app: &App) -> crate::app::Result<Option<String>> {
    let (gh, session) = (app.gh.as_ref(), &app.ctx.session);
    if session.contains('/') {
        return Ok(None);
    }
    Ok(match registry::load(gh)? {
        Some((number, _)) => crate::core::handoff::parent_of(&trusted_notes(gh, number)?, session),
        None => None,
    })
}

/// At most 500 characters, as `bosun msg`.
fn parent_message(
    session: &str,
    epic: u64,
    outcome: &str,
    pr: Option<u64>,
    green: &[(u64, String)],
) -> String {
    let pr = pr.map_or(String::new(), |p| format!(", PR #{p}"));
    let mut m = format!("bosun result from {session}: epic #{epic} {outcome}{pr}.");
    if green.is_empty() {
        m.push_str(" No PR is waiting for a merge.");
    } else {
        let list: Vec<String> = green
            .iter()
            .map(|(p, sha)| format!("#{p} ({})", sha.get(..7).unwrap_or(sha)))
            .collect();
        m.push_str(&format!(
            " Green and mergeable, waiting for you (I never merge; GitHub cannot auto-merge a clean PR): {}. Merge them, then run `bosun reap`.",
            list.join(", ")
        ));
    }
    crate::core::text::inert(&m, 500)
}

/// Why `partial` must not move epic `n` back to ready (the result is still recorded): only the
/// session working on it may hand it back, so it needs this session's own live lease on the epic
/// (a spawned child's lease is written by `spawn --record`), a working state, and no halt.
fn partial_refusal(app: &App, n: u64, state: Option<State>) -> crate::app::Result<Option<String>> {
    let (gh, me, now) = (app.gh.as_ref(), &app.ctx.session, app.ctx.now);
    if !matches!(
        state,
        Some(State::Claimed | State::Building | State::Review | State::Fixing)
    ) {
        let st = state.map_or("not a task".into(), |s| s.to_string());
        return Ok(Some(format!(
            "#{n} is {st}; partial moves only claimed, building, review or fixing"
        )));
    }
    if !leases_of(n, &trusted_notes(gh, n)?)
        .iter()
        .any(|l| &l.session == me && l.is_live(now))
    {
        return Ok(Some(format!("{me} holds no live lease on #{n}")));
    }
    if let Some((why, _)) = halt_reason(gh)? {
        return Ok(Some(format!("halted: {why}")));
    }
    Ok(None)
}
