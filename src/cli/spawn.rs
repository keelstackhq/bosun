//! `bosun spawn`: the 7 checks of spec §5, claim the epic, return the `create_session` call;
//! `--record` registers the child once the session made the call.

use std::io::Read;
use std::path::Path;

use serde_json::{Value, json};

use super::SpawnArgs;
use crate::app::{
    App, end_lease, kill_switch, limits_from_default, main_red, notes, set_state, task_meta,
};
use crate::core::lease::{Lease, Member, TTL_SECS, leases_of, members};
use crate::core::meta::area_of;
use crate::core::records::{Note, Record, spend_total};
use crate::core::spawn::{SpawnInput, check};
use crate::core::state::{State, state_of};
use crate::github::Issue;
use crate::output::{BosunError, CmdResult, Outcome};
use crate::registry;
use crate::time::Timestamp;

pub fn run(dir: Option<&Path>, args: &SpawnArgs) -> CmdResult {
    let app = App::load(dir)?;
    kill_switch(app.gh.as_ref())?;
    let n = args.epic;
    let epic = app.gh.get_issue(n)?.ok_or_else(|| {
        BosunError::error(
            format!("issue #{n} does not exist"),
            "check the epic number; `bosun status` lists the epics",
        )
    })?;
    if args.record {
        return record(&app, &epic, args);
    }
    let limits = limits_from_default(app.gh.as_ref(), &app.ctx)?;
    let model = args
        .model
        .clone()
        .unwrap_or_else(|| limits.models.build.clone());
    if model.contains("[1m]") {
        return Err(BosunError::usage(
            format!("model {model:?} is a [1m] model"),
            "pass --model without the [1m] suffix, such as sonnet or opus",
        ));
    }
    let (me, now) = (&app.ctx.session, app.ctx.now);
    let reg = registry::load(app.gh.as_ref())?;
    let reg_notes: &[Note] = reg.as_ref().map_or(&[], |r| &r.1);
    let all = members(reg_notes);
    let live: Vec<&Member> = all.iter().filter(|m| m.expires > now).collect();
    // The depth is the registry's, never the spawner's own word: no live record, no spawn.
    let parent_depth = live
        .iter()
        .find(|m| &m.session == me)
        .map(|m| m.depth)
        .ok_or_else(unregistered)?;
    let meta = task_meta(&epic);
    let area = area_of(&epic.labels);
    let leases = leases_of(n, &notes(app.gh.as_ref(), n)?);
    let input = SpawnInput {
        main_red: main_red(app.gh.as_ref())?.is_some(),
        mainfix: meta.as_ref().is_some_and(|m| m.kind == "mainfix")
            || epic.labels.iter().any(|l| l == "bosun:kind/mainfix"),
        spent_today_usd: spend_total(reg_notes, Some(now.day_start())),
        daily_usd: limits.budget.daily_usd,
        parent_depth,
        max_depth: limits.sessions.max_depth,
        live_sessions: live.len() as u32,
        max_live_sessions: limits.sessions.max_live_sessions,
        children: live
            .iter()
            .filter(|m| m.parent.as_deref() == Some(me))
            .count() as u32,
        max_children: limits.sessions.max_children,
        conflict: conflict(&leases, &live, me, area.as_deref(), now),
    };
    check(&input).map_err(|r| {
        BosunError::new(r.exit, r.message, r.fix)
            .with_data(json!({"check": r.check, "name": r.name}))
    })?;
    let depth = input.parent_depth + 1;
    let prompt = match &args.prompt_file {
        Some(f) => std::fs::read_to_string(f).map_err(|e| {
            BosunError::error(
                format!("cannot read --prompt-file {}: {e}", f.display()),
                "pass a readable file, or omit --prompt-file for the default prompt",
            )
        })?,
        None => default_prompt(n, &epic.title, area.as_deref(), me, depth),
    };

    if !leases.iter().any(|l| &l.session == me && l.is_live(now)) {
        let record = Record::Lease {
            session: me.clone(),
            origin: app.ctx.origin.as_str().into(),
            expires: now.plus_secs(TTL_SECS).to_string(),
            touch: vec![],
        };
        let body = format!(
            "Claimed by {me} to spawn an orchestrator.\n\n{}",
            record.to_line()
        );
        app.gh.add_comment(n, &body)?;
    }
    if state_of(&epic.labels) == Some(State::Ready) {
        set_state(app.gh.as_ref(), n, &epic, State::Claimed)?;
    }
    let project = &app.ctx.config.project;
    let tags = [
        format!("bosun:{project}"),
        format!("bosun:issue/{n}"),
        format!("bosun:parent/{me}"),
    ];
    Ok(
        Outcome::data(json!({"epic": n, "checks_passed": 6, "model": model})).with_next(
            "create_session",
            json!({
                "title": format!("{project}: #{n} {}", epic.title),
                "prompt": prompt,
                "model": model,
                "tags": tags,
            }),
        ),
    )
}

/// Refusal of a spawner without a live registry record (check 3, depth).
fn unregistered() -> BosunError {
    BosunError::limit(
        "this session has no live record in the registry, so its spawn depth is unknown",
        "run `bosun join` first",
    )
    .with_data(json!({"check": 3, "name": "depth"}))
}

/// Another session's live lease on the epic, or another live member owning its area (the
/// parent of this session does not count, as in `claim`).
fn conflict(
    leases: &[Lease],
    live: &[&Member],
    me: &str,
    area: Option<&str>,
    now: Timestamp,
) -> Option<String> {
    if let Some(l) = leases.iter().find(|l| l.session != me && l.is_live(now)) {
        return Some(format!(
            "the lease of {} on the epic until {}",
            l.session, l.expires
        ));
    }
    let area = area?;
    let parent = live
        .iter()
        .find(|m| m.session == me)
        .and_then(|m| m.parent.as_deref());
    live.iter()
        .find(|m| {
            m.area.as_deref() == Some(area) && m.session != me && Some(m.session.as_str()) != parent
        })
        .map(|m| format!("area `{area}` owned by {} until {}", m.session, m.expires))
}

fn default_prompt(epic: u64, title: &str, area: Option<&str>, me: &str, depth: u32) -> String {
    let area_flag = area.map_or(String::new(), |a| format!("--area {a} "));
    format!(
        "You orchestrate epic #{epic} ({title}).\n\
         1. Run `bosun join {area_flag}--parent {me} --depth {depth}`.\n\
         2. Run `bosun wave plan {epic}` and work the tasks it returns as wave agents, each in its own workspace.\n\
         3. When done, post a comment on #{epic} with this line (outcome is merged, blocked or needs-human):\n\
         {{\"bosun\":\"result\",\"session\":\"<your session id>\",\"outcome\":\"merged\"}}\n"
    )
}

/// The session id in a `create_session` result.
fn child_id(v: &Value) -> Option<String> {
    [&v["id"], &v["session_id"], &v["session"]["id"]]
        .into_iter()
        .find_map(|x| x.as_str().filter(|s| !s.is_empty()).map(String::from))
}

fn record(app: &App, epic: &Issue, args: &SpawnArgs) -> CmdResult {
    let read_err = |what: &str, e: std::io::Error| {
        BosunError::error(
            format!("cannot read {what}: {e}"),
            "pass a readable file, or `-` with the result piped in",
        )
    };
    let text = match args.from.as_deref() {
        Some("-") => {
            let mut t = String::new();
            std::io::stdin()
                .read_to_string(&mut t)
                .map_err(|e| read_err("stdin", e))?;
            t
        }
        Some(f) => std::fs::read_to_string(f).map_err(|e| read_err(f, e))?,
        None => {
            return Err(BosunError::usage(
                "--record needs --from",
                "run `bosun spawn <epic> --record --from -` with the create_session result piped in",
            ));
        }
    };
    let child = serde_json::from_str::<Value>(&text)
        .ok()
        .and_then(|v| child_id(&v))
        .ok_or_else(|| {
            BosunError::error(
                "the create_session result has no session id",
                "pass the JSON the create_session call returned, with an `id` field, on --from -",
            )
        })?;
    let (gh, me, now, n) = (app.gh.as_ref(), &app.ctx.session, app.ctx.now, epic.number);
    let (reg, reg_notes) = registry::ensure(gh)?;
    let all = members(&reg_notes);
    let depth = all
        .iter()
        .find(|m| &m.session == me && m.expires > now)
        .map(|m| m.depth)
        .ok_or_else(unregistered)?
        + 1;
    let expires = now.plus_secs(TTL_SECS);
    let prior = all.iter().find(|m| m.session == child);
    let member = Member {
        session: child.clone(),
        origin: "spawned".into(),
        area: area_of(&epic.labels),
        parent: Some(me.clone()),
        depth,
        expires,
        comment_id: prior.map_or(0, |m| m.comment_id),
    };
    registry::write(gh, reg, &reg_notes, prior, &member)?;
    if leases_of(n, &notes(gh, n)?)
        .iter()
        .any(|l| &l.session == me)
    {
        end_lease(gh, n, me, "spawned")?;
    }
    let lease = Record::Lease {
        session: child.clone(),
        origin: "spawned".into(),
        expires: expires.to_string(),
        touch: vec![],
    };
    gh.add_comment(n, &format!("Handed to {child}.\n\n{}", lease.to_line()))?;
    Ok(Outcome::data(json!({"epic": n, "child": child})))
}
