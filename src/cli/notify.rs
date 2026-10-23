//! `bosun notify`: the Slack posts that are due (spec §10, plan M6 S1-S4, S8). bosun never calls
//! Slack and reads no Slack token: it returns each post in `next` as a `slack_post` (D29), and
//! `--ack KEY` records that the caller made it. It works while halted: it announces the halt.

use std::collections::HashMap;
use std::io::Read;
use std::path::Path;

use serde_json::{Value, json};

use super::NotifyArgs;
use super::spawn::valid_ts;
use crate::app::{App, HALT_LABEL, default_branch, limits_from_default, main_red};
use crate::cli::risk_check::owner_logins;
use crate::core::budget::{Level, level};
use crate::core::lease::thread_of;
use crate::core::records::{Note, Record, parse_record, spend_total};
use crate::core::state::State;
use crate::github::{Github, IssueState, Variable};
use crate::output::{BosunError, CmdResult, Outcome};
use crate::registry;
use crate::slack::{ChannelInfo, Pending, neutralise, valid_key};
use crate::time::{Timestamp, parse_rfc3339};

/// At most this many owner questions per run.
const MAX_ASKS: usize = 5;
/// At most this many pending thread replies per run.
const MAX_RESULTS: usize = 10;
/// How many of the newest bosun issues are searched for results.
const MAX_EPICS: usize = 100;

pub fn run(dir: Option<&Path>, args: &NotifyArgs) -> CmdResult {
    match &args.ack {
        Some(key) => ack(dir, key),
        None => {
            if let Some(ts) = &args.thread
                && !valid_ts(ts)
            {
                return Err(BosunError::usage(
                    format!("--thread {ts:?} is not a Slack timestamp"),
                    "pass the thread ts as digits, a dot and digits, such as 1700000000.000200",
                ));
            }
            list(dir, args.channel_info.as_deref(), args.thread.as_deref())
        }
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
    let found = collect(&app, None)?;
    if found.done.iter().any(|k| k == key) {
        return Ok(Outcome::data(json!({"acked": key, "already": true})));
    }
    if !found.pending.iter().any(|p| p.key == key) {
        return Err(BosunError::conflict(
            format!("{key} is not a pending notification"),
            "ack only keys that `bosun notify` lists now; run it again",
        ));
    }
    let (issue, _) = registry::ensure(gh)?;
    let record = Record::Notified {
        key: key.into(),
        at: app.ctx.now.to_string(),
    };
    let comment = gh.add_comment(issue, &record.to_line())?;
    // The record counts only from a writer. Another login (a bot token, a read-only user)
    // would have every ack ignored and the post repeated forever; there is no way to delete the
    // comment, but an ignored record leaves the key pending.
    let p = gh.permission(&comment.user)?.unwrap_or_default();
    if !matches!(p.as_str(), "admin" | "write") {
        return Err(BosunError::needs_human(
            format!(
                "the ack was written by {} with {p} access, so it is ignored",
                comment.user
            ),
            "the token's user needs write access, or acks are ignored and posts repeat",
        ));
    }
    Ok(Outcome::data(json!({"acked": key, "already": false})))
}

/// Whether a login has write access, asked once per login (as `verdict apply` does).
struct Trust<'a> {
    gh: &'a dyn Github,
    seen: HashMap<String, bool>,
}

impl Trust<'_> {
    fn ok(&mut self, user: &str) -> Result<bool, BosunError> {
        if let Some(t) = self.seen.get(user) {
            return Ok(*t);
        }
        let p = self.gh.permission(user)?.unwrap_or_default();
        let t = matches!(p.as_str(), "admin" | "write");
        self.seen.insert(user.to_string(), t);
        Ok(t)
    }
}

struct Found {
    pending: Vec<Pending>,
    /// Keys with a `notified` record by a writer.
    done: Vec<String>,
    /// Sections that failed: `{kind, message}`.
    errors: Vec<Value>,
}

fn push(pending: &mut Vec<Pending>, key: String, kind: &'static str, text: String) {
    pending.push(Pending {
        key,
        kind,
        text,
        thread_ts: None,
    });
}

/// The pending posts. Halt and main-red come first; a later section that fails is reported in
/// `errors` and does not hide them. A `notified`, `ask`, `result` or thread-carrying lease record
/// counts only from a writer. With `only_thread`, only the thread-results of that thread.
fn collect(app: &App, only_thread: Option<&str>) -> Result<Found, BosunError> {
    let (gh, ctx) = (app.gh.as_ref(), &app.ctx);
    let slack = ctx.config.slack.clone().unwrap_or_default();
    let listed = |kind: &str| slack.notify.is_empty() || slack.notify.iter().any(|k| k == kind);
    let wants = |kind: &str| only_thread.is_none() && listed(kind);
    let url = |n: u64| {
        format!(
            "https://github.com/{}/{}/issues/{n}",
            ctx.repo.owner, ctx.repo.name
        )
    };
    let day = ctx.now.to_string()[..10].to_string();
    let mut trust = Trust {
        gh,
        seen: HashMap::new(),
    };
    let mut pending: Vec<Pending> = Vec::new();
    let mut errors: Vec<Value> = Vec::new();
    // The `notified` records come first: acked asks must not use up the cap. When they cannot
    // be read, halt and main-red stay pending (a repeat beats silence) and the rest is dropped.
    let mut done: Vec<String> = Vec::new();
    let mut reg = None;
    let read = (|| -> Result<_, BosunError> {
        let r = registry::load(gh)?;
        if let Some((n, _)) = &r {
            for c in gh.list_comments(*n)? {
                if let Some(Record::Notified { key, .. }) = parse_record(&c.body)
                    && trust.ok(&c.user)?
                {
                    done.push(key);
                }
            }
        }
        Ok(r)
    })();
    let notified_ok = read.is_ok();
    match read {
        Ok(r) => reg = r,
        Err(e) => {
            done.clear();
            errors.push(json!({"kind": "notified", "message": e.message}));
        }
    }

    // No kill switch here on purpose: notify announces the halt (spec §10). The owner logic is
    // that of `app::halt_reason`: the first halt issue by an owner, the first one when no
    // owner is listed, then the BOSUN_HALT variable.
    if wants("halt") {
        let r = (|| -> Result<(), BosunError> {
            let halts = gh.list_issues(&[HALT_LABEL], IssueState::Open)?;
            if !halts.is_empty() {
                let owners = owner_logins(gh, &default_branch(gh)?)?;
                let hit = if owners.is_empty() {
                    halts.first()
                } else {
                    halts
                        .iter()
                        .find(|i| owners.iter().any(|o| o.eq_ignore_ascii_case(&i.user)))
                };
                if let Some(i) = hit {
                    let text = if owners.is_empty() {
                        format!(
                            "bosun is halted: halt issue #{} is open and no owners are listed to judge it. {}",
                            i.number,
                            url(i.number)
                        )
                    } else {
                        format!(
                            "bosun is halted: the owner stopped all work. {}",
                            url(i.number)
                        )
                    };
                    push(&mut pending, format!("halt:{}", i.number), "halt", text);
                    return Ok(());
                }
            }
            if let Variable::Set(v) = gh.variable("BOSUN_HALT")?
                && v.trim() == "1"
            {
                let text = "bosun is halted: the owner set BOSUN_HALT=1.".to_string();
                push(&mut pending, format!("halt-var:{day}"), "halt", text);
            }
            Ok(())
        })();
        if let Err(e) = r {
            errors.push(json!({"kind": "halt", "message": e.message}));
        }
    }
    if wants("main-red") {
        match main_red(gh) {
            Ok(Some(n)) => {
                let text = format!("main is red; new work is paused. {}", url(n));
                push(&mut pending, format!("main-red:{n}"), "main-red", text);
            }
            Ok(None) => {}
            Err(e) => errors.push(json!({"kind": "main-red", "message": e.message})),
        }
    }
    if wants("budget-halt")
        && notified_ok
        && let Some((reg_issue, reg_notes)) = &reg
    {
        match limits_from_default(gh, ctx) {
            Ok(l) => {
                let daily = l.budget.daily_usd;
                let spent = spend_total(reg_notes, Some(ctx.now.day_start()));
                if level(spent, daily) == Level::Stop {
                    let text = format!(
                        "daily budget reached: ${spent:.2} of ${daily:.2} spent; live sessions are stopped. {}",
                        url(*reg_issue)
                    );
                    push(
                        &mut pending,
                        format!("budget-halt:{day}"),
                        "budget-halt",
                        text,
                    );
                }
            }
            Err(e) => errors.push(json!({"kind": "budget-halt", "message": e.message})),
        }
    }
    if wants("owner-question") && notified_ok {
        let r = (|| -> Result<(), BosunError> {
            let mut issues = gh.list_issues(&[&State::NeedsHuman.label()], IssueState::Open)?;
            issues.sort_by_key(|i| i.number);
            let mut count = 0;
            for i in issues {
                // Only asks still pending count toward the cap.
                if count >= MAX_ASKS {
                    break;
                }
                // The latest ask by a writer; anyone else's ask is not an owner question.
                for c in gh.list_comments(i.number)?.into_iter().rev() {
                    if let Some(Record::Ask { question, .. }) = parse_record(&c.body)
                        && trust.ok(&c.user)?
                    {
                        let key = format!("ask:{}:{}", i.number, c.id);
                        if done.contains(&key) {
                            break;
                        }
                        let text = format!(
                            "question for the owner on #{}: {} {}",
                            i.number,
                            neutralise(&question),
                            url(i.number)
                        );
                        push(&mut pending, key, "owner-question", text);
                        count += 1;
                        break;
                    }
                }
            }
            Ok(())
        })();
        if let Err(e) = r {
            errors.push(json!({"kind": "owner-question", "message": e.message}));
        }
    }

    if listed("thread-result") && notified_ok {
        let r = thread_results(gh, ctx, &mut trust, &done, only_thread, &mut pending);
        if let Err(e) = r {
            errors.push(json!({"kind": "thread-result", "message": e.message}));
        }
    }

    pending.retain(|p| !done.contains(&p.key));
    Ok(Found {
        pending,
        done,
        errors,
    })
}

/// The unacked `result` records of epics that came from a thread. The newest `MAX_EPICS` bosun
/// issues are searched (a result older than that is dropped). A lease's `thread_ts` and a result
/// count only from a writer; the cap counts only results still pending.
fn thread_results(
    gh: &dyn Github,
    ctx: &crate::ctx::Ctx,
    trust: &mut Trust,
    done: &[String],
    only_thread: Option<&str>,
    pending: &mut Vec<Pending>,
) -> Result<(), BosunError> {
    let mut issues: Vec<_> = gh
        .list_issues(&[], IssueState::All)?
        .into_iter()
        .filter(|i| i.labels.iter().any(|l| l.starts_with("bosun:state/")))
        .collect();
    issues.sort_by_key(|i| std::cmp::Reverse(i.number));
    let mut count = 0;
    for i in issues.into_iter().take(MAX_EPICS) {
        if count >= MAX_RESULTS {
            break;
        }
        let mut leases: Vec<Note> = Vec::new();
        let mut results: Vec<(u64, String, Option<u64>)> = Vec::new();
        for c in gh.list_comments(i.number)? {
            match parse_record(&c.body) {
                Some(Record::Lease {
                    thread_ts: Some(_), ..
                }) if trust.ok(&c.user)? => leases.push(Note {
                    id: c.id,
                    body: c.body,
                    created_at: parse_rfc3339(&c.created_at).unwrap_or(Timestamp(0)),
                }),
                Some(Record::Result { outcome, pr, .. }) if trust.ok(&c.user)? => {
                    results.push((c.id, outcome, pr));
                }
                _ => {}
            }
        }
        let Some(ts) = thread_of(&leases) else {
            continue;
        };
        if only_thread.is_some_and(|t| t != ts) {
            continue;
        }
        let base = format!("https://github.com/{}/{}", ctx.repo.owner, ctx.repo.name);
        for (id, outcome, pr) in results {
            let key = format!("result:{}:{id}", i.number);
            if done.contains(&key) {
                continue;
            }
            if count >= MAX_RESULTS {
                break;
            }
            let pr_text = pr.map_or(String::new(), |p| format!(" PR {base}/pull/{p}"));
            let text = format!(
                "result of #{}: {}. {base}/issues/{}{pr_text}",
                i.number,
                neutralise(&outcome),
                i.number
            );
            pending.push(Pending {
                key,
                kind: "thread-result",
                text,
                thread_ts: Some(ts.clone()),
            });
            count += 1;
        }
    }
    Ok(())
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

fn list(dir: Option<&Path>, channel_info: Option<&str>, thread: Option<&str>) -> CmdResult {
    let app = App::load(dir)?;
    let channel = app
        .ctx
        .config
        .slack
        .as_ref()
        .map(|s| s.channel.trim().to_string())
        .unwrap_or_default();
    if channel.is_empty() {
        return Err(BosunError::error(
            "[slack] channel is not set",
            "set [slack] channel in .bosun/bosun.toml",
        ));
    }
    let Found {
        pending, errors, ..
    } = collect(&app, thread)?;

    if !pending.is_empty() {
        let keys: Vec<&str> = pending.iter().map(|p| p.key.as_str()).collect();
        let refuse = |reason: &str, message: String, fix: &str| {
            Err(BosunError::needs_human(message, fix)
                .with_data(json!({"reason": reason, "pending": keys, "errors": errors})))
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
        "errors": errors,
        "pending": pending.iter().map(|p| json!({
            "key": p.key, "kind": p.kind, "text": p.text, "thread_ts": p.thread_ts,
            "ack": format!("bosun notify --ack {}", p.key),
        })).collect::<Vec<Value>>(),
    }));
    for p in &pending {
        let mut args = json!({"channel": channel, "text": p.text});
        if let Some(ts) = &p.thread_ts {
            args["thread_ts"] = json!(ts);
        }
        out = out.with_next("slack_post", args);
    }
    Ok(out)
}
