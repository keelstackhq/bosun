//! `bosun status`: sessions, leases, tasks by state, halt, main-red, process-red and budget.

use std::collections::BTreeMap;
use std::path::Path;

use serde_json::json;

use super::budget::{feature_levels, knob_from_default};
use crate::app::{
    App, Result, active_issues, halt_reason, limits_from_default, main_red, trusted_notes,
};
use crate::core::handoff::{children_of, finished_children};
use crate::core::lease::{live_leases, members, orchestrator_of};
use crate::core::mode::BudgetFeature;
use crate::core::records::{Note, spend_total};
use crate::core::state::State;
use crate::gates::integrity::reduced_reason;
use crate::github::{Github, IssueState};
use crate::output::{CmdResult, Outcome};
use crate::registry;
use crate::time::Timestamp;

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
        spend = spend_total(&registry::trusted_spend_notes(gh)?, Some(now.day_start()));
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

    let daily = limits_from_default(gh, ctx)?.budget.daily_usd;
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
        "daily_usd": daily,
        "budget_pct": pct,
        "integrity": if reduced.is_some() { "reduced" } else { "full" },
    });
    if let Some(why) = reduced {
        data["integrity_reason"] = json!(why);
    }
    // Status is the recovery path: the budget part is best effort.
    let budget = knob_from_default(gh).and_then(|k| match k {
        BudgetFeature::Off => Ok(None),
        _ => feature_levels(&app).map(Some),
    });
    match budget {
        Ok(Some((levels, skipped))) => {
            data["budget_feature"] = json!(levels);
            if !skipped.is_empty() {
                data["budget_skipped"] = json!(skipped);
            }
        }
        Ok(None) => {}
        Err(e) => data["budget_skipped"] = json!([{"epic": null, "error": e.message}]),
    }
    with_archive_children(gh, &ctx.session, &[], now, Outcome::data(data))
}

/// Adds `archive_session` to `next` for each child of the caller's orchestrator whose newest
/// result is final and that is not working again (spec §5). `extra_issues` are epics closed in
/// this run. A failure is reported as `data.archive_error`, never as an error. bosun only
/// returns the call (D29).
pub fn with_archive_children(
    gh: &dyn Github,
    me: &str,
    extra_issues: &[u64],
    now: Timestamp,
    mut out: Outcome,
) -> CmdResult {
    match archive_children(gh, me, extra_issues, now) {
        Ok(children) => {
            for c in children {
                out = out.with_next("archive_session", json!({"session_id": c}));
            }
        }
        Err(e) => {
            eprintln!("bosun: archive check failed: {e}");
            out.data["archive_error"] = json!(e.to_string());
        }
    }
    Ok(out)
}

fn archive_children(
    gh: &dyn Github,
    me: &str,
    extra_issues: &[u64],
    now: Timestamp,
) -> Result<Vec<String>> {
    let Some((reg_issue, reg)) = registry::load(gh)? else {
        return Ok(Vec::new());
    };
    // No child, no GitHub reads: the common case costs only the registry.
    let parent = orchestrator_of(me);
    if children_of(&reg, parent).is_empty() {
        return Ok(Vec::new());
    }
    let mut numbers: Vec<u64> = gh
        .list_issues(&[], IssueState::Open)?
        .iter()
        .filter(|i| i.number != reg_issue)
        .map(|i| i.number)
        .collect();
    for n in extra_issues {
        if !numbers.contains(n) {
            numbers.push(*n);
        }
    }
    let mut results: Vec<Note> = Vec::new();
    for number in numbers {
        results.extend(trusted_notes(gh, number)?);
    }
    // Leases are read only when some child has finished.
    if finished_children(&reg, parent, &results, &[]).is_empty() {
        return Ok(Vec::new());
    }
    let held: Vec<_> = active_issues(gh)?
        .into_iter()
        .map(|(i, notes)| (i.number, notes))
        .collect();
    Ok(finished_children(
        &reg,
        parent,
        &results,
        &live_leases(&held, now),
    ))
}
