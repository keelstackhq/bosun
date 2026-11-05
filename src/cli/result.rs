//! `bosun result`: a child hands its result to its parent (spec D14).

use std::path::Path;

use serde_json::json;

use super::ResultArgs;
use crate::app::{
    App, default_branch, end_lease, halt_reason, merge_counts, notes, set_state, sync_merged,
    task_pr_merged,
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
    if let Some(to) = to
        && state != Some(to)
        && state.is_none_or(|from| transition_allowed(from, to))
    {
        set_state(gh, n, &epic, to)?;
        state = Some(to);
    }

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
            if task_pr_merged(gh, &base, task.number, &mut pulls, true)?
                && sync_merged(gh, task.number, &task, None)?
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

    let lease_ended = leases_of(n, &notes(gh, n)?)
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
    Ok(Outcome::data(json!({
        "epic": n, "outcome": args.outcome, "pr": args.pr,
        "comment_id": comment_id, "lease_ended": lease_ended, "left": left,
        "task_state": state.map(State::name), "tasks_merged": tasks_merged,
        "partial_skipped": partial_skipped,
    })))
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
    if !leases_of(n, &notes(gh, n)?)
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
