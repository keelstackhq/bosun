//! `bosun notify`: the Slack posts that are due (spec §10, plan M6 S1-S4, S8). bosun never calls
//! Slack and reads no Slack token: it returns each post in `next` as a `slack_post` (D29), and
//! `--ack KEY` records that the caller made it. It works while halted: it announces the halt.

use std::io::Read;
use std::path::Path;

use serde_json::{Value, json};

use super::NotifyArgs;
use crate::app::{App, HALT_LABEL, default_branch, limits_from_default, main_red, notes};
use crate::cli::risk_check::owner_logins;
use crate::core::budget::{Level, level};
use crate::core::records::{Note, Record, parse_record, spend_total};
use crate::core::state::State;
use crate::github::IssueState;
use crate::output::{BosunError, CmdResult, Outcome};
use crate::registry;
use crate::slack::{ChannelInfo, Pending, neutralise, valid_key};

pub fn run(dir: Option<&Path>, args: &NotifyArgs) -> CmdResult {
    match &args.ack {
        Some(key) => ack(dir, key),
        None => list(dir, args.channel_info.as_deref()),
    }
}

fn ack(dir: Option<&Path>, key: &str) -> CmdResult {
    if !valid_key(key) {
        return Err(BosunError::usage(
            format!("{key:?} is not a notification key"),
            "pass a key from `bosun notify`, such as halt:7 or ask:12:345 (lowercase letters, digits, - and :)",
        ));
    }
    let app = App::load(dir)?;
    let gh = app.gh.as_ref();
    let (issue, notes) = registry::ensure(gh)?;
    if notified_keys(&notes).iter().any(|k| k == key) {
        return Ok(Outcome::data(json!({"acked": key, "already": true})));
    }
    let record = Record::Notified {
        key: key.into(),
        at: app.ctx.now.to_string(),
    };
    gh.add_comment(issue, &record.to_line())?;
    Ok(Outcome::data(json!({"acked": key, "already": false})))
}

fn notified_keys(notes: &[Note]) -> Vec<String> {
    notes
        .iter()
        .filter_map(|n| match parse_record(&n.body) {
            Some(Record::Notified { key, .. }) => Some(key),
            _ => None,
        })
        .collect()
}

fn read_info(from: &str) -> Result<String, String> {
    let mut text = String::new();
    let read = if from == "-" {
        std::io::stdin().read_to_string(&mut text).map(|_| ())
    } else {
        std::fs::read_to_string(from).map(|t| text = t)
    };
    read.map(|()| text)
        .map_err(|e| format!("cannot read {from}: {e}"))
}

fn list(dir: Option<&Path>, channel_info: Option<&str>) -> CmdResult {
    let app = App::load(dir)?;
    let (gh, ctx) = (app.gh.as_ref(), &app.ctx);
    let slack = ctx.config.slack.clone().unwrap_or_default();
    let channel = slack.channel.trim().to_string();
    if channel.is_empty() {
        return Err(BosunError::error(
            "[slack] channel is not set",
            "set [slack] channel in .bosun/bosun.toml",
        ));
    }
    let wants = |kind: &str| slack.notify.is_empty() || slack.notify.iter().any(|k| k == kind);
    let url = |n: u64| {
        format!(
            "https://github.com/{}/{}/issues/{n}",
            ctx.repo.owner, ctx.repo.name
        )
    };

    let reg = registry::load(gh)?;
    let done = reg
        .as_ref()
        .map(|(_, n)| notified_keys(n))
        .unwrap_or_default();
    let mut pending: Vec<Pending> = Vec::new();
    let mut add = |key: String, kind: &'static str, text: String| {
        if !done.contains(&key) {
            pending.push(Pending { key, kind, text });
        }
    };

    // No kill switch here on purpose: notify announces the halt (spec §10).
    if wants("halt") {
        let halts = gh.list_issues(&[HALT_LABEL], IssueState::Open)?;
        if !halts.is_empty() {
            let owners = owner_logins(gh, &default_branch(gh)?)?;
            for i in halts
                .iter()
                .filter(|i| owners.iter().any(|o| o.eq_ignore_ascii_case(&i.user)))
            {
                add(
                    format!("halt:{}", i.number),
                    "halt",
                    format!(
                        "bosun is halted: the owner stopped all work. {}",
                        url(i.number)
                    ),
                );
            }
        }
    }
    if wants("main-red")
        && let Some(n) = main_red(gh)?
    {
        add(
            format!("main-red:{n}"),
            "main-red",
            format!("main is red; new work is paused. {}", url(n)),
        );
    }
    if wants("budget-halt")
        && let Some((reg_issue, reg_notes)) = &reg
    {
        let daily = limits_from_default(gh, ctx)?.budget.daily_usd;
        let spent = spend_total(reg_notes, Some(ctx.now.day_start()));
        if level(spent, daily) == Level::Stop {
            let day = &ctx.now.to_string()[..10];
            add(
                format!("budget-halt:{day}"),
                "budget-halt",
                format!(
                    "daily budget reached: ${spent:.2} of ${daily:.2} spent; live sessions are stopped. {}",
                    url(*reg_issue)
                ),
            );
        }
    }
    if wants("owner-question") {
        for i in gh.list_issues(&[&State::NeedsHuman.label()], IssueState::Open)? {
            for n in notes(gh, i.number)? {
                if let Some(Record::Ask { question, .. }) = parse_record(&n.body) {
                    add(
                        format!("ask:{}:{}", i.number, n.id),
                        "owner-question",
                        format!(
                            "question for the owner on #{}: {} {}",
                            i.number,
                            neutralise(&question),
                            url(i.number)
                        ),
                    );
                }
            }
        }
    }

    if !pending.is_empty() {
        let keys: Vec<&str> = pending.iter().map(|p| p.key.as_str()).collect();
        let refuse = |reason: &str, message: String, fix: &str| {
            Err(BosunError::needs_human(message, fix)
                .with_data(json!({"reason": reason, "pending": keys})))
        };
        let facts = r#"{"id","is_ext_shared","is_org_shared","is_shared","guests"}"#;
        let Some(from) = channel_info else {
            return refuse(
                "no channel info",
                format!(
                    "{} posts are due but the channel facts are missing",
                    keys.len()
                ),
                &format!(
                    "read the channel with the Slack tool (conversations.info and the guest count) and pass {facts} as JSON with --channel-info -"
                ),
            );
        };
        let info = match read_info(from).and_then(|t| ChannelInfo::parse(&t)) {
            Ok(i) => i,
            Err(e) => {
                return refuse(
                    "no channel info",
                    format!("the channel facts are not valid: {e}"),
                    &format!("pass exactly {facts} as JSON with --channel-info -"),
                );
            }
        };
        if let Err(reason) = info.check(&channel) {
            return refuse(
                reason,
                format!("will not post: {reason} ({})", info.id),
                "the owner must make [slack] channel a private, non-shared channel without guests; post nothing until then",
            );
        }
    }

    let mut out = Outcome::data(json!({
        "channel": channel,
        "pending": pending.iter().map(|p| json!({
            "key": p.key, "kind": p.kind, "text": p.text,
            "ack": format!("bosun notify --ack {}", p.key),
        })).collect::<Vec<Value>>(),
    }));
    for p in &pending {
        out = out.with_next("slack_post", json!({"channel": channel, "text": p.text}));
    }
    Ok(out)
}
