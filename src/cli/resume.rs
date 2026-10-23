//! `bosun resume`: after a container restart, renew this session's join record and leases and
//! list, for each held task, the PR to continue (spec §5, Janitor, resume, results).

use std::path::Path;

use serde_json::{Value, json};

use crate::app::{App, active_issues, kill_switch, renew_lease};
use crate::core::glob;
use crate::core::lease::{Lease, TTL_SECS, conflicts, leases_of, live_leases, members};
use crate::core::meta::linked_issues;
use crate::core::state::{State, state_of};
use crate::github::{IssueState, Pull};
use crate::output::{CmdResult, Outcome};
use crate::registry::{self, JoinSpec};

pub fn run(dir: Option<&Path>) -> CmdResult {
    let app = App::load(dir)?;
    let gh = app.gh.as_ref();
    kill_switch(gh)?;
    let (session, now) = (&app.ctx.session, app.ctx.now);
    let expires = now.plus_secs(TTL_SECS);

    // The join record: renew it, or join again when the registry has no record of us.
    let mut joined = false;
    let existing = match registry::load(gh)? {
        Some((_, notes)) => members(&notes)
            .into_iter()
            .find(|m| &m.session == session)
            .map(|m| (m, notes)),
        None => None,
    };
    match existing {
        Some((mut m, notes)) => {
            m.expires = expires;
            let body = notes
                .iter()
                .find(|n| n.id == m.comment_id)
                .map_or("", |n| n.body.as_str());
            registry::rewrite(gh, &m, body)?;
        }
        None => {
            registry::join(&app, JoinSpec::default())?;
            joined = true;
        }
    }

    let active = active_issues(gh)?;
    let by_issue: Vec<_> = active.iter().map(|(i, n)| (i.number, n.clone())).collect();
    let mut live = live_leases(&by_issue, now);
    let pulls = gh.list_pulls(None, IssueState::Open)?;

    let (mut tasks, mut lost) = (Vec::new(), Vec::new());
    for (issue, notes) in &active {
        for lease in leases_of(issue.number, notes) {
            if &lease.session != session {
                continue;
            }
            if !lease.is_live(now) {
                let Ok(touch) = glob::parse_all(&lease.touch) else {
                    // A bad glob covers everything (as in `conflicts`): fail closed.
                    lost.push(json!({"issue": issue.number, "conflicts": [],
                        "fix": "the touch paths of this lease do not parse; release the task and claim it again"}));
                    continue;
                };
                let found = overlaps(&touch, &lease, &live);
                if !found.is_empty() {
                    lost.push(json!({"issue": issue.number, "conflicts": found,
                        "fix": "another session leased these paths; pick another task with `bosun wave plan`"}));
                    continue;
                }
                live.push(Lease {
                    expires,
                    ..lease.clone()
                });
            }
            renew_lease(gh, &lease, notes, expires)?;
            let state = state_of(&issue.labels);
            let pull = pulls
                .iter()
                .find(|p| linked_issues(&p.body).contains(&issue.number));
            tasks.push(task_json(issue.number, &issue.title, state, pull));
        }
    }
    Ok(Outcome::data(json!({
        "session": session, "joined": joined,
        "tasks": tasks, "lost": lost,
    })))
}

/// Live leases that overlap the touch paths of an expired lease.
fn overlaps(touch: &[glob::Glob], lease: &Lease, live: &[Lease]) -> Vec<Value> {
    conflicts(touch, live, &lease.session, Some(lease.issue))
        .iter()
        .map(|c| {
            json!({"issue": c.issue, "session": c.session, "mine": c.mine,
                "theirs": c.theirs, "expires": c.expires.to_string()})
        })
        .collect()
}

fn task_json(issue: u64, title: &str, state: Option<State>, pull: Option<&Pull>) -> Value {
    let action = match (state, pull) {
        (Some(State::Review), Some(p)) => format!("`bosun wait --pr {}`", p.number),
        (Some(State::Queued), Some(p)) => {
            format!("`bosun wait --pr {}` until GitHub merges it", p.number)
        }
        (_, Some(p)) => format!(
            "`git fetch origin {b} && git switch {b}`, push the fix, then `bosun pr {issue}`",
            b = p.head_ref
        ),
        (_, None) => format!("continue building, then `bosun pr {issue}`"),
    };
    json!({"issue": issue, "title": title, "state": state.map(|s| s.to_string()),
        "pr": pull.map(|p| p.number), "branch": pull.map(|p| &p.head_ref),
        "head_sha": pull.map(|p| &p.head_sha), "action": action})
}
