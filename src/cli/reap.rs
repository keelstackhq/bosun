//! `bosun reap`: end expired leases (task back to ready) and expired registry members.

use std::path::Path;

use serde_json::json;

use crate::app::{App, active_issues, end_lease, set_state, sync_merged};
use crate::core::lease::{leases_of, members};
use crate::core::meta::linked_issues;
use crate::core::records::Record;
use crate::core::state::{State, state_of};
use crate::github::IssueState;
use crate::output::{CmdResult, Outcome};
use crate::registry;

pub fn run(dir: Option<&Path>) -> CmdResult {
    let app = App::load(dir)?;
    let (gh, now) = (app.gh.as_ref(), app.ctx.now);

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

    // Auto-merge closes the issue, so look at queued tasks in any issue state.
    let mut merged = Vec::new();
    let queued = gh.list_issues(&[&State::Queued.label()], IssueState::All)?;
    if !queued.is_empty() {
        let pulls = gh.list_pulls(None, IssueState::All)?;
        for issue in queued {
            for p in pulls
                .iter()
                .filter(|p| linked_issues(&p.body).contains(&issue.number))
            {
                // Lists do not say `merged`; read the closed PR.
                if p.state == "closed" && gh.get_pull(p.number)?.is_some_and(|f| f.merged) {
                    sync_merged(gh, issue.number, &issue, None)?;
                    merged.push(issue.number);
                    break;
                }
            }
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
