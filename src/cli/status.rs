//! `bosun status`: sessions, leases, tasks by state, halt, main-red, process-red and budget.

use std::collections::BTreeMap;
use std::path::Path;

use serde_json::json;

use crate::app::{App, Result, active_issues, halt_reason, main_red};
use crate::core::lease::{live_leases, members};
use crate::core::records::spend_total;
use crate::core::state::State;
use crate::gates::integrity::reduced_reason;
use crate::github::{Github, IssueState};
use crate::output::{CmdResult, Outcome};
use crate::registry;

pub const PROCESS_RED_LABEL: &str = "bosun:process-red";

/// The number of an open issue labelled `bosun:process-red` (a blocking lesson carries it).
pub fn process_red(gh: &dyn Github) -> Result<Option<u64>> {
    Ok(gh
        .list_issues(&[PROCESS_RED_LABEL], IssueState::Open)?
        .first()
        .map(|i| i.number))
}

pub fn run(dir: Option<&Path>) -> CmdResult {
    let app = App::load(dir)?;
    let (gh, ctx) = (app.gh.as_ref(), &app.ctx);
    let now = ctx.now;

    // An unreadable kill switch halts work commands, so it shows as halted.
    let halted = !matches!(halt_reason(gh), Ok(None));
    let process_red = process_red(gh)?;

    let (mut sessions, mut spend) = (Vec::new(), 0.0);
    if let Some((_, notes)) = registry::load(gh)? {
        for m in members(&notes).into_iter().filter(|m| m.expires > now) {
            sessions.push(
                json!({"session": m.session, "origin": m.origin, "area": m.area,
                "depth": m.depth, "expires": m.expires.to_string()}),
            );
        }
        spend = spend_total(&notes, Some(now.day_start()));
    }

    let held: Vec<_> = active_issues(gh)?
        .into_iter()
        .map(|(i, notes)| (i.number, notes))
        .collect();
    let leases: Vec<_> = live_leases(&held, now)
        .into_iter()
        .map(|l| {
            json!({"issue": l.issue, "session": l.session,
            "expires": l.expires.to_string(), "touch": l.touch})
        })
        .collect();

    let mut by_state = BTreeMap::new();
    for st in State::ALL {
        let n = gh.list_issues(&[&st.label()], IssueState::Open)?.len();
        if n > 0 {
            by_state.insert(st.name(), n);
        }
    }

    let daily = ctx.limits.budget.daily_usd;
    let pct = if daily > 0.0 {
        spend / daily * 100.0
    } else {
        0.0
    };
    // Reduced when a stack's profile uses a generic adapter (spec §7), read from the local files.
    let reduced = reduced_reason(&ctx.dir, &ctx.config.stack);
    let mut data = json!({
        "session": ctx.session,
        "origin": ctx.origin.as_str(),
        "repo": ctx.repo.to_string(),
        "halted": halted,
        "main_red": main_red(gh)?,
        "process_red": process_red,
        "live_sessions": sessions,
        "leases": leases,
        "issues_by_state": by_state,
        "spend_today_usd": spend,
        "budget_pct": pct,
        "integrity": if reduced.is_some() { "reduced" } else { "full" },
    });
    if let Some(why) = reduced {
        data["integrity_reason"] = json!(why);
    }
    Ok(Outcome::data(data))
}
