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
use crate::core::glob::{self, Glob};
use crate::core::lease::{Lease, TTL_SECS, conflicts, leases_of, members, race_winner};
use crate::core::meta::area_of;
use crate::core::records::{Note, Record, parse_record, spend_total};
use crate::core::state::{State, state_of};
use crate::github::Issue;
use crate::output::{BosunError, CmdResult, Outcome};
use crate::registry;
use crate::time::Timestamp;

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
        let cur = notes(gh, n)?;
        let mine = leases_of(n, &cur)
            .into_iter()
            .find(|l| &l.session == session && l.is_live(now));
        if let Some(l) = mine.filter(|_| state.is_some_and(State::is_active)) {
            let mut added: Vec<String> = Vec::new();
            for g in &args.touch {
                if !l.touch.contains(g) && !added.contains(g) {
                    added.push(g.clone());
                }
            }
            if !added.is_empty() {
                return grow(&app, n, &l, &added, &cur);
            }
            let exp = l.expires.to_string();
            let mut data = lease_data(n, session, &exp, &l.touch, l.comment_id);
            if !args.touch.is_empty() {
                data["added"] = json!([]);
            }
            return Ok(Outcome::data(data));
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
    check_overlap(&active, &touch, session, n, now)?;

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

        thread_ts: None,
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

/// Refuse (exit 3, naming the holder) when `touch` overlaps a live lease of another issue.
fn check_overlap(
    active: &[(Issue, Vec<Note>)],
    touch: &[Glob],
    session: &str,
    n: u64,
    now: Timestamp,
) -> Result<(), BosunError> {
    let live: Vec<_> = active
        .iter()
        .flat_map(|(i, notes)| leases_of(i.number, notes))
        .filter(|l| l.is_live(now))
        .collect();
    let found = conflicts(touch, &live, session, Some(n));
    if found.is_empty() {
        return Ok(());
    }
    let list: Vec<String> = found.iter().map(|c| c.describe()).collect();
    let data: Vec<Value> = found
        .iter()
        .map(|c| {
            json!({"issue": c.issue, "session": c.session, "mine": c.mine,
                "theirs": c.theirs, "expires": c.expires.to_string()})
        })
        .collect();
    Err(BosunError::conflict(
        format!(
            "touch paths of #{n} overlap a live lease: {}",
            list.join("; ")
        ),
        "wait until that lease ends, narrow --touch, or claim another task",
    )
    .with_data(json!({"conflicts": data})))
}

/// A re-claim of an own active task with new globs: the same overlap check on the added globs,
/// then a newer lease record with the union (the newest lease is the live one; no state change).
/// `mine` are the comments of the issue as read by the caller. On a lost race the grow comment is
/// rewritten with the old lease, so the lease keeps its previous touch and is never ended. (A
/// concurrent `bosun heartbeat` of the same plain session id can rewrite that comment with the
/// union touch before the restore; per-task ids `S/x` avoid it.)
fn grow(app: &App, n: u64, old: &Lease, added: &[String], mine: &[Note]) -> CmdResult {
    let (gh, session, now) = (app.gh.as_ref(), &app.ctx.session, app.ctx.now);
    let new_globs = glob::parse_all(added).map_err(|m| {
        BosunError::usage(
            m,
            "use repo-relative globs with * ? ** only, such as src/core/**",
        )
    })?;
    let active = active_issues(gh)?;
    check_overlap(&active, &new_globs, session, n, now)?;
    let touch: Vec<String> = old.touch.iter().chain(added).cloned().collect();
    let thread_ts = match mine
        .iter()
        .find(|c| c.id == old.comment_id)
        .and_then(|c| parse_record(&c.body))
    {
        Some(Record::Lease { thread_ts, .. }) => thread_ts,
        _ => None,
    };
    let body = |expires: String, touch: Vec<String>| {
        let r = Record::Lease {
            session: session.clone(),
            origin: old.origin.clone(),
            expires,
            touch,
            thread_ts: thread_ts.clone(),
        };
        format!("Claimed by {session}.\n\n{}", r.to_line())
    };
    let expires = now.plus_secs(TTL_SECS).to_string();
    let comment = gh.add_comment(n, &body(expires.clone(), touch.clone()))?;
    // Race against the active issues as they are after the write: a rival may have claimed an
    // issue that was still `ready` when the list was first read.
    let fresh = active_issues(gh)?;
    let restored = body(old.expires.to_string(), old.touch.clone());
    let won = fresh.iter().find(|(i, _)| i.number == n).is_some_and(|(_, c)| {
        // Still held: a lease-end of this session newer than the old record means a reap or
        // release ended the lease while the grow was written.
        !c.iter().any(|x| {
            x.id > old.comment_id
                && matches!(parse_record(&x.body), Some(Record::LeaseEnd { session: s, .. }) if &s == session)
        })
    });
    if !won {
        gh.update_comment(comment.id, &restored)?;
        return Err(BosunError::conflict(
            format!("#{n} is no longer held by this session"),
            format!("claim it again with `bosun claim {n}`"),
        ));
    }
    let ids: Vec<_> = fresh.into_iter().map(|(i, c)| (i.number, c)).collect();
    if let Some(w) = race_winner(n, comment.id, &new_globs, &ids, now) {
        let list = added.join(", ");
        let lost = format!(
            "#{n}: lost the race for {list} to #{} (session {}, lease until {})",
            w.issue, w.session, w.expires
        );
        let rival = json!({"issue": w.issue, "session": w.session,
            "expires": w.expires.to_string()});
        return Err(match gh.update_comment(comment.id, &restored) {
            Ok(_) => BosunError::conflict(
                format!("{lost}; your lease keeps its previous touch"),
                "wait until that lease ends, or claim with a narrower --touch",
            )
            .with_data(json!({"conflicts": [rival], "restored": true})),
            Err(e) => BosunError::error(
                format!(
                    "{lost}, and restoring the old lease failed ({e}); the lease still names {list}"
                ),
                format!(
                    "run `bosun release {n}`, or re-run `bosun claim {n}` with the same --touch once that lease ends"
                ),
            )
            .with_data(json!({"conflicts": [rival], "restored": false})),
        });
    }
    let mut data = lease_data(n, session, &expires, &touch, comment.id);
    data["added"] = json!(added);
    Ok(Outcome::data(data))
}
