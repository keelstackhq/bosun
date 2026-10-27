//! `bosun reap`: move tasks whose PR merged to merged, end expired leases (task back to ready)
//! and expired registry members.

use std::path::Path;

use serde_json::json;

use crate::app::{
    App, active_issues, default_branch, end_lease, merge_counts, notes, set_state, sync_merged,
};
use crate::cli::plan::epic_item;
use crate::core::lease::{leases_of, members};
use crate::core::meta::{linked_issues, parse_task_meta};
use crate::core::records::{Record, parse_record};
use crate::core::state::{State, TaskEnd, epic_complete, labels_to_remove, state_of};
use crate::github::{Github, Issue, IssuePatch, IssueState, PatchState};
use crate::output::{CmdResult, Outcome};
use crate::registry;
use crate::time::{Timestamp, parse_rfc3339};

pub fn run(dir: Option<&Path>) -> CmdResult {
    let app = App::load(dir)?;
    let (gh, now) = (app.gh.as_ref(), app.ctx.now);

    // A merge closes the issue, so look at tasks in any issue state. This runs before the expiry
    // loop: a merged task ends its leases as `merged` and is never reaped back to `ready`.
    let mut merged = Vec::new();
    let mut open_work: Vec<Issue> = Vec::new();
    for st in [State::Building, State::Review, State::Fixing, State::Queued] {
        for issue in gh.list_issues(&[&st.label()], IssueState::All)? {
            if !merge_counts(&issue) {
                continue;
            }
            if !open_work.iter().any(|i| i.number == issue.number) {
                open_work.push(issue);
            }
        }
    }
    open_work.sort_by_key(|i| i.number);
    if !open_work.is_empty() {
        let base = default_branch(gh)?;
        // Listed only for tasks without a usable `task-pr` record (older ones).
        let mut all_pulls = None;
        for issue in open_work {
            let recorded =
                notes(gh, issue.number)?
                    .iter()
                    .rev()
                    .find_map(|n| match parse_record(&n.body) {
                        Some(Record::TaskPr { pr, .. }) => Some(pr),
                        _ => None,
                    });
            // The record is trusted like any other; the PR must still link the task.
            let pull = match recorded {
                Some(pr) => gh
                    .get_pull(pr)?
                    .filter(|p| linked_issues(&p.body).contains(&issue.number)),
                None => None,
            };
            match pull {
                Some(p) if p.state == "open" => continue,
                Some(p) if p.merged && p.base_ref == base => {
                    sync_merged(gh, issue.number, &issue, None)?;
                    merged.push(issue.number);
                    continue;
                }
                // Closed unmerged, or no usable record: the sweep decides.
                _ => {}
            }
            let pulls = match &all_pulls {
                Some(p) => p,
                None => all_pulls.insert(gh.list_pulls(None, IssueState::All)?),
            };
            let linked: Vec<_> = pulls
                .iter()
                .filter(|p| linked_issues(&p.body).contains(&issue.number))
                .collect();
            // A PR still open means the work is not done (an old merged PR may link the issue).
            if linked.iter().any(|p| p.state == "open") {
                continue;
            }
            for p in linked
                .iter()
                .filter(|p| p.state == "closed" && p.base_ref == base)
            {
                // Lists do not say `merged`; read the closed PR.
                if gh.get_pull(p.number)?.is_some_and(|f| f.merged) {
                    sync_merged(gh, issue.number, &issue, None)?;
                    merged.push(issue.number);
                    break;
                }
            }
        }
    }

    let mut reaped = Vec::new();
    for (issue, notes) in active_issues(gh)? {
        let leases = leases_of(issue.number, &notes);
        let expired: Vec<_> = leases.iter().filter(|l| !l.is_live(now)).collect();
        for lease in &expired {
            end_lease(gh, issue.number, &lease.session, "reaped")?;
            reaped.push(json!({"issue": issue.number, "session": lease.session}));
        }
        let held = leases.iter().any(|l| l.is_live(now));
        let early = matches!(
            state_of(&issue.labels),
            Some(State::Claimed | State::Building)
        );
        if !expired.is_empty() && early && !held {
            set_state(gh, issue.number, &issue, State::Ready)?;
        }
    }

    let mut left = Vec::new();
    if let Some((number, notes)) = registry::load(gh)? {
        for m in members(&notes).into_iter().filter(|m| m.expires <= now) {
            let leave = Record::Leave {
                session: m.session.clone(),
                reason: "reaped".into(),
            };
            gh.add_comment(number, &leave.to_line())?;
            left.push(m.session);
        }
    }
    // Closing epics is housekeeping: a failure there must not undo or hide the work above.
    let mut epics_closed = Vec::new();
    let epics_error = close_done_epics(gh, now, &mut epics_closed).err();
    epics_closed.sort_unstable();
    let mut data =
        json!({"reaped": reaped, "left": left, "merged": merged, "epics_closed": epics_closed});
    if let Some(e) = epics_error {
        eprintln!("reap: closing finished epics failed: {e}");
        data["epics_error"] = json!(e.to_string());
    }
    Ok(Outcome::data(data))
}

const QUIET_SECS: i64 = 600;

/// Close each open epic whose tasks are all merged or closed, one at least done (see
/// `epic_complete`): state `merged`, closed as completed. A live lease (an orchestrator's) blocks
/// it; a `claimed` epic without one is a finished spawned orchestrator's (`result` ends the
/// lease, not the state). The quiet period does not cover an amend adding a task right now; the
/// next import or amend reopens the epic. Open plan issues are listed first; all of them only
/// when an unleased epic has no open task (epics with only dropped tasks pay that every run).
/// Closed epics go into `closed` as they happen, so an error keeps the progress.
fn close_done_epics(
    gh: &dyn Github,
    now: Timestamp,
    closed: &mut Vec<u64>,
) -> crate::app::Result<()> {
    let epic_of = |i: &Issue| parse_task_meta(i.body_text()).and_then(|m| m.epic);
    let open = gh.list_issues(&["bosun:source/plan"], IssueState::Open)?;
    let mut candidates: Vec<&Issue> = Vec::new();
    for e in open
        .iter()
        .filter(|i| !i.pull_request && epic_item(i.body_text()).is_some())
        .filter(|i| {
            matches!(
                state_of(&i.labels),
                None | Some(State::Ready | State::Planned | State::Claimed)
            )
        })
        .filter(|e| {
            !open.iter().any(|t| {
                epic_of(t) == Some(e.number)
                    && TaskEnd::of(state_of(&t.labels), false, None) == TaskEnd::Open
            })
        })
    {
        if !leases_of(e.number, &notes(gh, e.number)?)
            .iter()
            .any(|l| l.is_live(now))
        {
            candidates.push(e);
        }
    }
    if candidates.is_empty() {
        return Ok(());
    }
    let all = gh.list_issues(&["bosun:source/plan"], IssueState::All)?;
    for epic in candidates {
        let tasks = all.iter().filter(|t| epic_of(t) == Some(epic.number));
        let end = |t: &Issue| {
            let reason = t.state_reason.as_deref();
            TaskEnd::of(state_of(&t.labels), t.state == "closed", reason)
        };
        if !epic_complete(&tasks.clone().map(end).collect::<Vec<_>>()) {
            continue;
        }
        // Quiet period: an amend may be adding a task right now (GitHub's list lags).
        let newest = tasks
            .filter_map(|t| parse_rfc3339(&t.updated_at))
            .max_by_key(|t| t.0);
        if newest.is_some_and(|n| n.0 > now.minus_secs(QUIET_SECS).0) {
            continue;
        }
        // Close first: an epic labelled merged but still open would never be a candidate again.
        gh.update_issue(
            epic.number,
            IssuePatch {
                state: Some(PatchState::Closed),
                state_reason: Some("completed".into()),
                ..IssuePatch::default()
            },
        )?;
        closed.push(epic.number);
        gh.ensure_label(&State::Merged.label(), "ededed", "bosun task state")?;
        for label in labels_to_remove(&epic.labels, State::Merged) {
            gh.remove_label(epic.number, &label)?;
        }
        gh.add_labels(epic.number, &[&State::Merged.label()])?;
    }
    Ok(())
}
