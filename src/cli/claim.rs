//! `bosun claim`: write a lease for a ready task, or refuse with exit 3 (spec §4).

use std::path::Path;

use serde_json::{Value, json};

use super::ClaimArgs;
use crate::app::{
    App, active_issues, kill_switch, limits_from_default, main_red, notes, set_state, settle_race,
    task_meta,
};
use crate::cli::status::process_red;
use crate::core::budget::{BUDGET_STOP, no_new_work};
use crate::core::glob;
use crate::core::lease::{TTL_SECS, conflicts, leases_of, members};
use crate::core::meta::area_of;
use crate::core::records::{Record, spend_total};
use crate::core::state::{State, state_of};
use crate::output::{BosunError, CmdResult, Outcome};
use crate::registry;

pub fn run(dir: Option<&Path>, args: &ClaimArgs) -> CmdResult {
    let app = App::load(dir)?;
    let gh = app.gh.as_ref();
    let (n, session, now) = (args.issue, &app.ctx.session, app.ctx.now);
    kill_switch(gh)?;
    let issue = gh.get_issue(n)?.ok_or_else(|| {
        BosunError::error(
            format!("issue #{n} does not exist"),
            "check the number; `bosun status` lists the tasks",
        )
    })?;
    let meta = task_meta(&issue);
    let mainfix = meta.as_ref().is_some_and(|m| m.kind == "mainfix")
        || issue.labels.iter().any(|l| l == "bosun:kind/mainfix");
    if let Some(red) = main_red(gh)?
        && !mainfix
    {
        return Err(BosunError::halted(
            format!("main is red (#{red}); only mainfix tasks can be claimed"),
            format!("wait for #{red} to close, or claim a mainfix task"),
        ));
    }
    if let Some(red) = process_red(gh)?
        && !mainfix
        && !meta.as_ref().is_some_and(|m| m.kind == "process-fix")
    {
        return Err(BosunError::halted(
            format!(
                "process is red (#{red}): a blocking lesson pauses feature work; only mainfix and process-fix tasks can be claimed"
            ),
            format!(
                "fix the process (a process-fix task) and close #{red}, or claim a process-fix task"
            ),
        ));
    }

    let state = state_of(&issue.labels);
    if state != Some(State::Ready) {
        let mine = leases_of(n, &notes(gh, n)?)
            .into_iter()
            .find(|l| &l.session == session && l.is_live(now));
        if let Some(l) = mine.filter(|_| state.is_some_and(State::is_active)) {
            return Ok(Outcome::data(lease_data(
                n,
                session,
                &l.expires.to_string(),
                &l.touch,
                l.comment_id,
            )));
        }
        return Err(BosunError::conflict(
            format!(
                "#{n} is {}",
                state.map_or("not a task".into(), |s| s.to_string())
            ),
            "claim a task in state ready; `bosun wave plan` lists them",
        ));
    }

    // Spend guard (spec §9, UC-M4-1): the same rule as spawn check 2. It sits after the
    // idempotent re-claim above, which is not new work.
    if !mainfix {
        let daily = limits_from_default(gh, &app.ctx)?.budget.daily_usd;
        let spent =
            registry::load(gh)?.map_or(0.0, |(_, reg)| spend_total(&reg, Some(now.day_start())));
        if no_new_work(spent, daily) {
            return Err(BosunError::limit(
                format!(
                    "spent ${spent:.2} of ${daily:.2} today (at or over {:.0} %); no new work",
                    BUDGET_STOP * 100.0
                ),
                "wait for the day to reset at 00:00 UTC, claim a mainfix task, or ask the owner to raise budget.daily_usd in .bosun/limits.toml",
            )
            .with_data(json!({"check": 2, "name": "budget", "spent_today_usd": spent,
                "daily_usd": daily, "pct": spent / daily * 100.0})));
        }
    }

    let touch_raw = if args.touch.is_empty() {
        meta.map(|m| m.touch).unwrap_or_default()
    } else {
        args.touch.clone()
    };
    if touch_raw.is_empty() {
        return Err(BosunError::usage(
            format!("no touch paths for #{n}"),
            "pass --touch GLOB (repeatable) or add `touch` to the task metadata",
        ));
    }
    let touch = glob::parse_all(&touch_raw).map_err(|m| {
        BosunError::usage(
            m,
            "use repo-relative globs with * ? ** only, such as src/core/**",
        )
    })?;

    let active = active_issues(gh)?;
    let live: Vec<_> = active
        .iter()
        .flat_map(|(i, notes)| leases_of(i.number, notes))
        .filter(|l| l.is_live(now))
        .collect();
    let found = conflicts(&touch, &live, session, Some(n));
    if !found.is_empty() {
        let list: Vec<String> = found.iter().map(|c| c.describe()).collect();
        let data: Vec<Value> = found
            .iter()
            .map(|c| {
                json!({"issue": c.issue, "session": c.session, "mine": c.mine,
                    "theirs": c.theirs, "expires": c.expires.to_string()})
            })
            .collect();
        return Err(BosunError::conflict(
            format!(
                "touch paths of #{n} overlap a live lease: {}",
                list.join("; ")
            ),
            "wait until that lease ends, narrow --touch, or claim another task",
        )
        .with_data(json!({"conflicts": data})));
    }

    if let (Some(area), Some((_, reg))) = (area_of(&issue.labels), registry::load(gh)?) {
        let all = members(&reg);
        let parent = all
            .iter()
            .find(|m| &m.session == session)
            .and_then(|m| m.parent.clone());
        let owner = all.iter().find(|m| {
            m.area.as_deref() == Some(&area)
                && &m.session != session
                && m.expires > now
                && Some(&m.session) != parent.as_ref()
        });
        if let Some(o) = owner {
            return Err(BosunError::conflict(
                format!(
                    "area `{area}` is owned by {} until {}",
                    o.session, o.expires
                ),
                format!("ask {} to take #{n}, or wait until it leaves", o.session),
            ));
        }
    }

    let expires = now.plus_secs(TTL_SECS).to_string();
    let record = Record::Lease {
        session: session.clone(),
        origin: app.ctx.origin.as_str().into(),
        expires: expires.clone(),
        touch: touch_raw.clone(),
    };
    let body = format!("Claimed by {session}.\n\n{}", record.to_line());
    let comment = gh.add_comment(n, &body)?;
    let others: Vec<u64> = active.iter().map(|(i, _)| i.number).collect();
    settle_race(gh, session, n, comment.id, &touch, &others, now)?;
    set_state(gh, n, &issue, State::Claimed)?;
    Ok(Outcome::data(lease_data(
        n, session, &expires, &touch_raw, comment.id,
    )))
}

fn lease_data(issue: u64, session: &str, expires: &str, touch: &[String], id: u64) -> Value {
    json!({"issue": issue, "comment_id": id,
        "lease": {"session": session, "expires": expires, "touch": touch}})
}
