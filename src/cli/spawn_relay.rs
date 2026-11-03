//! `bosun spawn --root`: a root relays to a fresh root session (spec §5.2). bosun returns the
//! `create_session` and `archive_session` calls and never makes them (D29). A relay writes a
//! `join` and a `relay` record, never a lease.

use serde_json::json;

use super::SpawnArgs;
use super::spawn::read_child;
use crate::app::{
    App, active_issues, default_branch, end_lease, limits_from_default, trusted_notes,
};
use crate::config::{CONFIG_FILE, Config};
use crate::core::handoff::{is_root, newest_handoff};
use crate::core::lease::{Member, TTL_SECS, leases_of};
use crate::core::records::{Note, Record, parse_record};
use crate::output::{BosunError, CmdResult, Outcome};
use crate::registry;

pub fn run(app: &App, args: &SpawnArgs) -> CmdResult {
    if !args.root {
        return Err(BosunError::usage(
            "--handoff belongs to --root",
            "run `bosun spawn --root --handoff`",
        ));
    }
    if args.epic.is_some()
        || args.slack_thread.is_some()
        || args.slack_user.is_some()
        || args.request.is_some()
        || args.prompt_file.is_some()
    {
        return Err(BosunError::usage(
            "--root takes no epic, Slack flag, --request or --prompt-file",
            "run `bosun spawn --root --handoff`, then `bosun spawn --root --record --from -`",
        ));
    }
    if args.record {
        return record(app, args);
    }
    if !args.handoff {
        return Err(BosunError::usage(
            "--root needs --handoff or --record",
            "pass --handoff to get the calls, or --record --from - to register the new root",
        ));
    }
    let (gh, me) = (app.gh.as_ref(), &app.ctx.session);
    let trusted = trusted_root_notes(app)?;
    let comment = own_handoff(&trusted, me)?;
    let limits = limits_from_default(gh, &app.ctx)?;
    let model = args
        .model
        .clone()
        .unwrap_or_else(|| limits.models.hard.clone());
    if model.contains("[1m]") {
        return Err(BosunError::usage(
            format!("model {model:?} is a [1m] model"),
            "pass --model without the [1m] suffix, such as sonnet or opus",
        ));
    }
    let branch = default_branch(gh)?;
    let env = match gh.file_at(CONFIG_FILE, &branch)? {
        Some(t) => {
            Config::parse(&t, &format!("{CONFIG_FILE} on {branch}"))?
                .cloud
                .environment_id
        }
        None => None,
    };
    let project = &app.ctx.config.project;
    let mut call = json!({"title": format!("{project}: root relay"), "prompt": "bosun resume --handoff",
        "model": model, "tags": [format!("bosun:{project}"), format!("bosun:relay/{me}")]});
    // Without `[cloud] environment_id` the new session inherits the caller's environment.
    if let Some(id) = &env {
        call["environment_id"] = json!(id);
    }
    Ok(Outcome::data(json!({"from": me, "handoff": comment, "model": model,
        "relay_waves": limits.waves.relay_waves, "environment_id": env,
        "then": "do not call archive_session until `bosun spawn --root --record --from -` has run with the create_session result"}))
    .with_next("create_session", call)
    .with_next("archive_session", json!({"session_id": me})))
}

/// The comment id of the caller's own newest handoff, or the refusal.
fn own_handoff(trusted: &[Note], me: &str) -> Result<u64, BosunError> {
    match newest_handoff(trusted, &[me]) {
        Some((id, r)) if r.session() == me => Ok(id),
        _ => Err(BosunError::error(
            format!("{me} has written no handoff"),
            "run `bosun handoff` first, then `bosun spawn --root --handoff`",
        )),
    }
}

/// The trusted registry notes, when the caller is a root; else the refusal.
fn trusted_root_notes(app: &App) -> Result<Vec<Note>, BosunError> {
    let me = &app.ctx.session;
    if let Some((number, _)) = registry::load(app.gh.as_ref())? {
        let trusted = trusted_notes(app.gh.as_ref(), number)?;
        if is_root(&trusted, me) {
            return Ok(trusted);
        }
    }
    Err(BosunError::error(
        format!("{me} is not a joined root session; only a root relays"),
        "run `bosun join` as a root (no --parent, an id without `/`)",
    ))
}

fn record(app: &App, args: &SpawnArgs) -> CmdResult {
    let (gh, me, now) = (app.gh.as_ref(), &app.ctx.session, app.ctx.now);
    let trusted = trusted_root_notes(app)?;
    own_handoff(&trusted, me)?;
    let relayed = trusted.iter().any(
        |n| matches!(parse_record(&n.body), Some(Record::Relay { session, .. }) if &session == me),
    );
    if relayed {
        return Err(BosunError::error(
            format!("{me} already relayed to a root"),
            "a root relays once; the new root continues, so archive this session",
        ));
    }
    let child = read_child(args)?;
    let (reg, notes) = registry::ensure(gh)?;
    // The raw notes: a session that left or relayed earlier still has its join record.
    let known = notes.iter().any(|n| match parse_record(&n.body) {
        Some(Record::Join { session, .. }) => session == child,
        Some(Record::Relay { root, .. }) => root == child,
        _ => false,
    });
    if &child == me || child.contains('/') || known {
        return Err(BosunError::error(
            format!("the create_session result names {child:?}: not a new root session"),
            "pass the create_session result of the session you just started",
        ));
    }
    // Always a new join comment: an existing record is never edited into a root.
    let member = Member {
        session: child.clone(),
        origin: "relay".into(),
        area: None,
        parent: None,
        depth: 1,
        expires: now.plus_secs(TTL_SECS),
        comment_id: 0,
    };
    registry::write(gh, reg, &notes, None, &member)?;
    let relay = Record::Relay {
        session: me.clone(),
        root: child.clone(),
        at: now.to_string(),
    };
    gh.add_comment(
        reg,
        &format!("{me} relays to {child}.\n\n{}", relay.to_line()),
    )?;
    for (issue, inotes) in active_issues(gh)? {
        if leases_of(issue.number, &inotes)
            .iter()
            .any(|l| &l.session == me)
        {
            end_lease(gh, issue.number, me, "relayed")?;
        }
    }
    let leave = Record::Leave {
        session: me.clone(),
        reason: "relayed".into(),
    };
    gh.add_comment(reg, &leave.to_line())?;
    Ok(Outcome::data(json!({"from": me, "root": child}))
        .with_next("archive_session", json!({"session_id": me})))
}
