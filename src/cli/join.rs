//! `bosun join`: register this session in the registry, or refresh its record.

use std::path::Path;

use serde_json::json;

use super::JoinArgs;
use crate::app::{App, kill_switch};
use crate::core::records::SlackSeat;
use crate::ctx::Origin;
use crate::output::{BosunError, CmdResult, Outcome};
use crate::registry::{self, JoinSpec};

/// `^[CG][A-Z0-9]{2,}$`
fn channel_ok(c: &str) -> bool {
    let b = c.as_bytes();
    b.len() >= 3
        && matches!(b[0], b'C' | b'G')
        && b[1..]
            .iter()
            .all(|x| x.is_ascii_uppercase() || x.is_ascii_digit())
}

/// `^[0-9]+\.[0-9]+$`
fn ts_ok(t: &str) -> bool {
    t.split_once('.').is_some_and(|(a, b)| {
        [a, b]
            .iter()
            .all(|p| !p.is_empty() && p.bytes().all(|x| x.is_ascii_digit()))
    })
}

/// The pure checks of the seat flags (exit 2).
fn seat_of(args: &JoinArgs, origin: Option<Origin>) -> Result<Option<SlackSeat>, BosunError> {
    let (Some(channel), thread) = (&args.slack_channel, &args.slack_thread) else {
        if args.slack_thread.is_some() {
            return Err(BosunError::usage(
                "--slack-thread needs --slack-channel",
                "pass --slack-channel C... with --slack-thread, or drop both",
            ));
        }
        return Ok(None);
    };
    if origin != Some(Origin::Slack) {
        return Err(BosunError::usage(
            "--slack-channel needs --origin slack",
            "pass --origin slack, or drop the --slack-* flags",
        ));
    }
    if !channel_ok(channel) {
        return Err(BosunError::usage(
            format!("--slack-channel {channel:?} is not a channel id"),
            "pass the Slack channel id, like C0123ABCD (^[CG][A-Z0-9]{2,}$)",
        ));
    }
    if let Some(t) = thread
        && !ts_ok(t)
    {
        return Err(BosunError::usage(
            format!("--slack-thread {t:?} is not a thread ts"),
            "pass the thread ts, like 1700000000.000200",
        ));
    }
    Ok(Some(SlackSeat {
        role: if thread.is_some() {
            "thread"
        } else {
            "channel"
        }
        .into(),
        channel: channel.clone(),
        thread_ts: thread.clone(),
    }))
}

pub fn run(dir: Option<&Path>, args: &JoinArgs) -> CmdResult {
    let origin = match args.origin.as_deref() {
        Some(o) => Some(o.parse::<Origin>().map_err(|_| {
            BosunError::error(
                format!("--origin {o:?} is not an origin"),
                "use one of spawned, slack, local, routine",
            )
        })?),
        None => None,
    };
    let slack = seat_of(args, origin)?;
    let app = App::load(dir)?;
    kill_switch(app.gh.as_ref())?;
    if origin == Some(Origin::Slack) && matches!(app.ctx.origin, Origin::Spawned | Origin::Routine)
    {
        return Err(BosunError::needs_human(
            format!(
                "--origin slack, but this session resolves to origin {}",
                app.ctx.origin.as_str()
            ),
            "a spawned or routine session cannot relabel itself as Slack; join without --origin",
        ));
    }
    if let Some(seat) = &slack {
        let configured = app.ctx.config.slack.as_ref().map(|s| s.channel.as_str());
        if configured != Some(seat.channel.as_str()) {
            return Err(BosunError::needs_human(
                format!(
                    "--slack-channel {} is not the configured Slack channel",
                    seat.channel
                ),
                "set `[slack] channel` in .bosun/bosun.toml to this channel (an owner does this), or pass the configured channel",
            ));
        }
    }
    let (issue, m) = registry::join(
        &app,
        JoinSpec {
            origin,
            area: args.area.clone(),
            parent: args.parent.clone(),
            depth: args.depth,
            slack,
        },
    )?;
    Ok(Outcome::data(json!({
        "session": m.session,
        "origin": m.origin,
        "area": m.area,
        "depth": m.depth,
        "expires": m.expires.to_string(),
        "slack": m.slack,
        "registry_issue": issue,
    })))
}
