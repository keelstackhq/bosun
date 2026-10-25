//! `bosun reap`: move tasks whose PR merged to merged, end expired leases (task back to ready)
//! and expired registry members.

use std::path::Path;

use serde_json::json;

use crate::app::{App, active_issues, default_branch, end_lease, notes, set_state, sync_merged};
use crate::core::lease::{leases_of, members};
use crate::core::meta::linked_issues;
use crate::core::records::{Record, parse_record};
use crate::core::state::{State, state_of};
use crate::github::{Issue, IssueState};
use crate::output::{CmdResult, Outcome};
use crate::registry;

pub fn run(dir: Option<&Path>) -> CmdResult {
    let app = App::load(dir)?;
    let (gh, now) = (app.gh.as_ref(), app.ctx.now);

    // A merge closes the issue, so look at tasks in any issue state. This runs before the expiry
    // loop: a merged task ends its leases as `merged` and is never reaped back to `ready`.
    let mut merged = Vec::new();
    let mut open_work: Vec<Issue> = Vec::new();
    for st in [State::Building, State::Review, State::Fixing, State::Queued] {
        for issue in gh.list_issues(&[&st.label()], IssueState::All)? {
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
    Ok(Outcome::data(
        json!({"reaped": reaped, "left": left, "merged": merged}),
    ))
}
