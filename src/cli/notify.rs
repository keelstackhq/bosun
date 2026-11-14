//! `bosun notify`: the Slack posts that are due (spec §10, plan M6 S1-S4, S8). bosun never calls
//! Slack and reads no Slack token: it returns each post in `next` as a `slack_post` (D29), and
//! `--ack KEY` records that the caller made it. It works while halted: it announces the halt.

use std::collections::HashMap;
use std::io::Read;
use std::path::Path;

use serde_json::{Value, json};

use super::NotifyArgs;
use super::spawn::valid_ts;
use crate::app::{App, HALT_LABEL, default_ref, limits_from_default, main_red};
use crate::cli::risk_check::owner_logins;
use crate::core::budget::{Level, level};
use crate::core::records::{Record, SlackSeat, parse_record, spend_total};
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
/// At most this many PRs in one digest post.
const MAX_DIGEST: usize = 20;
/// Issues updated within this many seconds (7 days) are searched for results.
const WINDOW_SECS: i64 = 7 * 86_400;
/// Without `--thread`, a result younger than this is left to the live thread (15 minutes).
const GRACE_SECS: i64 = 900;

/// Where `collect` looks for thread replies.
#[derive(Clone, Copy, PartialEq)]
enum Results {
    /// Not at all.
    Off,
    /// The recently updated issues.
    Scan,
    /// One issue (for `--ack` of a `result:<issue>:<comment>` key).
    Issue(u64),
    /// Only the owner question of one issue, uncapped (for `--ack` of an `ask:<issue>:<comment>` key).
    Ask(u64),
}

pub fn run(dir: Option<&Path>, args: &NotifyArgs) -> CmdResult {
    match &args.ack {
        Some(_) if args.thread.is_some() => Err(BosunError::usage(
            "--thread and --ack cannot be used together",
            "list with `bosun notify --thread TS`, then ack a key with `bosun notify --ack KEY`",
        )),
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
    // A result or ask key is checked on its own issue: the capped list may not hold it.
    let issue_of = |prefix: &str| {
        key.strip_prefix(prefix)
            .and_then(|r| r.split_once(':'))
            .and_then(|(n, _)| n.parse().ok())
    };
    let mode = match (issue_of("result:"), issue_of("ask:")) {
        (Some(n), _) => Results::Issue(n),
        (_, Some(n)) => Results::Ask(n),
        _ => Results::Off,
    };
    let found = collect(&app, None, false, mode)?;
    if found.done.iter().any(|k| k == key) {
        return Ok(Outcome::data(json!({"acked": key, "already": true})));
    }
    // Today's digest key is checked by its format and the config, not by rebuilding the board.
    let slack = app.ctx.config.slack.clone().unwrap_or_default();
    let digest_today = key == format!("digest:{}", &app.ctx.now.to_string()[..10])
        && slack.notify.iter().any(|k| k == "digest");
    if !digest_today && !found.pending.iter().any(|p| p.key == key) {
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
/// counts only from a writer. With `only_thread`, only the thread-results of that thread, and
/// the channel posts too when `keep_channel` is set (a thread seat).
fn collect(
    app: &App,
    only_thread: Option<&str>,
    keep_channel: bool,
    results: Results,
) -> Result<Found, BosunError> {
    let (gh, ctx) = (app.gh.as_ref(), &app.ctx);
    let slack = ctx.config.slack.clone().unwrap_or_default();
    let listed = |kind: &str| slack.notify.is_empty() || slack.notify.iter().any(|k| k == kind);
    let wants = |kind: &str| {
        (only_thread.is_none() || keep_channel)
            && !matches!(results, Results::Issue(_) | Results::Ask(_))
            && listed(kind)
    };
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
                let owners = owner_logins(gh, &default_ref(gh)?)?;
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
        && let Some((reg_issue, _)) = &reg
    {
        let limits = limits_from_default(gh, ctx).and_then(|l| {
            let n = registry::trusted_spend_notes(gh)?;
            Ok((l, spend_total(&n, Some(ctx.now.day_start()))))
        });
        match limits {
            Ok((l, spent)) => {
                let daily = l.budget.daily_usd;
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
    // Opt-in: an empty notify list does not include the digest. Never on an ack.
    if matches!(results, Results::Scan)
        && wants("digest")
        && slack.notify.iter().any(|k| k == "digest")
    {
        match digest_post(app, &mut errors) {
            Ok(Some(p)) => pending.push(p),
            Ok(None) => {}
            Err(e) => errors.push(json!({"kind": "digest", "message": e.message})),
        }
    }
    let ask_only = match results {
        Results::Ask(n) => Some(n),
        _ => None,
    };
    if (wants("owner-question") || (ask_only.is_some() && listed("owner-question"))) && notified_ok
    {
        let r = (|| -> Result<(), BosunError> {
            let mut issues = gh.list_issues(&[&State::NeedsHuman.label()], IssueState::Open)?;
            issues.sort_by_key(|i| i.number);
            issues.retain(|i| ask_only.is_none_or(|n| n == i.number));
            let mut count = 0;
            for i in issues {
                // Only asks still pending count toward the cap.
                if ask_only.is_none() && count >= MAX_ASKS {
                    break;
                }
                // The latest ask by a writer; anyone else's ask is not an owner question. Its
                // thread is that of the newest writer lease before it.
                let mut lease: Option<String> = None;
                let mut ask: Option<(u64, String, Option<String>)> = None;
                for c in gh.list_comments(i.number)? {
                    match parse_record(&c.body) {
                        Some(Record::Lease { thread_ts, .. }) if trust.ok(&c.user)? => {
                            lease = thread_ts.filter(|t| valid_ts(t));
                        }
                        Some(Record::Ask { question, .. }) if trust.ok(&c.user)? => {
                            ask = Some((c.id, question, lease.clone()));
                        }
                        _ => {}
                    }
                }
                if let Some((id, question, thread_ts)) = ask {
                    // A thread seat takes only its own thread's asks, before the cap.
                    if keep_channel
                        && let Some(t) = only_thread
                        && thread_ts.as_deref().is_some_and(|x| x != t)
                    {
                        continue;
                    }
                    let key = format!("ask:{}:{id}", i.number);
                    if done.contains(&key) {
                        continue;
                    }
                    let text = format!(
                        "question for the owner on #{}: {} {}",
                        i.number,
                        neutralise(&question),
                        url(i.number)
                    );
                    pending.push(Pending {
                        key,
                        kind: "owner-question",
                        text,
                        thread_ts,
                    });
                    count += 1;
                }
            }
            Ok(())
        })();
        if let Err(e) = r {
            errors.push(json!({"kind": "owner-question", "message": e.message}));
        }
    }

    if listed("thread-result") && notified_ok {
        let r = thread_results(
            app,
            &mut trust,
            &done,
            only_thread,
            results,
            &mut pending,
            &mut errors,
        );
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

/// The daily post with one approve-by-GitHub-comment line per PR that waits for the owner.
/// Slack approves nothing: bosun cannot verify a Slack user id, so the owner approves on GitHub.
fn digest_post(app: &App, errors: &mut Vec<Value>) -> Result<Option<Pending>, BosunError> {
    let (items, _, skipped) = super::digest::board_read(app)?;
    if let Some(s) = skipped.as_array().filter(|s| !s.is_empty()) {
        // A tokenless listing reads less; say so instead of posting a partial board silently.
        errors.push(json!({"kind": "digest", "message": format!(
            "some PRs could not be read with this session's token, so the digest may be short: {}",
            Value::Array(s.clone()))}));
    }
    if items.is_empty() {
        return Ok(None);
    }
    let r = &app.ctx.repo;
    let mut text = format!("{} PR(s) wait for the owner:", items.len());
    for i in items.iter().take(MAX_DIGEST) {
        text.push_str(&format!(
            "\n#{} https://github.com/{}/{}/pull/{} \u{2014} approve: comment `/bosun approve {}` from your own GitHub account (check the head on the PR page) \u{2014} {} {}",
            i.pr,
            r.owner,
            r.name,
            i.pr,
            neutralise(&i.head),
            tick_free(&i.title),
            tick_free(&i.rules.join(", ")),
        ));
    }
    if items.len() > MAX_DIGEST {
        text.push_str(&format!("\nand {} more", items.len() - MAX_DIGEST));
    }
    text.push_str("\nSlack replies approve nothing.");
    Ok(Some(Pending {
        key: format!("digest:{}", &app.ctx.now.to_string()[..10]),
        kind: "digest",
        text,
        thread_ts: None,
    }))
}

/// Escaped text with no backtick, so a title cannot close or open the command's code span.
fn tick_free(text: &str) -> String {
    neutralise(text).replace('`', "'")
}

/// One `result` record that counts: by a writer, written after a writer's thread lease, which
/// gives its thread.
struct Reply {
    id: u64,
    outcome: String,
    pr: Option<u64>,
    at: Timestamp,
    thread_ts: String,
}

/// The counting results of one issue, oldest first. A result goes to the thread of the newest
/// lease with a valid `thread_ts` by a writer written before it, so a later re-spawn does not
/// move or drop it; one before any such lease belongs to no thread.
fn replies_of(gh: &dyn Github, trust: &mut Trust, issue: u64) -> Result<Vec<Reply>, BosunError> {
    let mut lease: Option<String> = None;
    let mut out = Vec::new();
    for c in gh.list_comments(issue)? {
        match parse_record(&c.body) {
            // A trusted lease sets the current thread; one without a (valid) thread_ts, such as
            // a later normal spawn, ends it, so that child's result is not a thread reply.
            Some(Record::Lease { thread_ts, .. }) if trust.ok(&c.user)? => {
                lease = thread_ts.filter(|t| valid_ts(t));
            }
            Some(Record::Result { outcome, pr, .. }) if trust.ok(&c.user)? => {
                if let Some(thread_ts) = &lease {
                    out.push(Reply {
                        id: c.id,
                        outcome,
                        pr,
                        at: parse_rfc3339(&c.created_at).unwrap_or(Timestamp(0)),
                        thread_ts: thread_ts.clone(),
                    });
                }
            }
            _ => {}
        }
    }
    Ok(out)
}

/// The unacked thread replies. `Results::Scan` reads one page of the issues updated in the last
/// 7 days (newest 100) and the comments of those with a state label; without `only_thread` a
/// result younger than the grace period is left to the live thread. `Results::Issue` checks one
/// issue, for `--ack`. The cap counts only results still pending.
fn thread_results(
    app: &App,
    trust: &mut Trust,
    done: &[String],
    only_thread: Option<&str>,
    mode: Results,
    pending: &mut Vec<Pending>,
    errors: &mut Vec<Value>,
) -> Result<(), BosunError> {
    let (gh, ctx) = (app.gh.as_ref(), &app.ctx);
    let issues: Vec<u64> = match mode {
        Results::Off | Results::Ask(_) => return Ok(()),
        Results::Issue(n) => vec![n],
        Results::Scan => {
            let (found, more) = gh.recent_issues(&ctx.now.minus_secs(WINDOW_SECS).to_string())?;
            if more {
                let max = crate::github::RECENT_PAGES * 100;
                errors.push(json!({"kind": "thread-result", "message":
                    format!("more than {max} issues updated in 7 days; some results may be missed")}));
            }
            found
                .into_iter()
                .filter(|i| i.labels.iter().any(|l| l.starts_with("bosun:state/")))
                .map(|i| i.number)
                .collect()
        }
    };
    let base = format!("https://github.com/{}/{}", ctx.repo.owner, ctx.repo.name);
    let mut count = 0;
    for n in issues {
        for r in replies_of(gh, trust, n)? {
            let key = format!("result:{n}:{}", r.id);
            let young = ctx.now.0 - r.at.0 < GRACE_SECS;
            if done.contains(&key)
                || only_thread.is_some_and(|t| t != r.thread_ts)
                || (mode == Results::Scan && only_thread.is_none() && young)
            {
                continue;
            }
            if mode == Results::Scan && count >= MAX_RESULTS {
                return Ok(());
            }
            let pr_text =
                r.pr.map_or(String::new(), |p| format!(" PR {base}/pull/{p}"));
            let text = format!(
                "result of #{n}: {}. {base}/issues/{n}{pr_text}",
                neutralise(&r.outcome)
            );
            pending.push(Pending {
                key,
                kind: "thread-result",
                text,
                thread_ts: Some(r.thread_ts),
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
    // Slack is optional: without a [slack] table there is nothing to post, and no error (F17).
    let Some(slack) = app.ctx.config.slack.as_ref() else {
        return Ok(Outcome::data(json!({
            "skipped": true,
            "reason": "no [slack] table in .bosun/bosun.toml: Slack is not configured, so nothing is posted",
            "pending": [],
            "errors": [],
        })));
    };
    let channel = slack.channel.trim().to_string();
    if channel.is_empty() {
        return Err(BosunError::error(
            "[slack] channel is not set",
            "set [slack] channel in .bosun/bosun.toml",
        ));
    }
    let mut errors_seat = Vec::new();
    let seat = own_seat(&app, &mut errors_seat);
    // A thread seat lists its own thread's results unless `--thread` names another.
    let own_thread = match &seat {
        Some(s) if s.role == "thread" && thread.is_none() => s.thread_ts.as_deref(),
        _ => None,
    };
    let only = thread.or(own_thread);
    let Found {
        mut pending,
        mut errors,
        ..
    } = collect(&app, only, own_thread.is_some(), Results::Scan)?;
    errors.extend(errors_seat);
    if own_thread.is_some() {
        // Owner questions of other threads are theirs; channel posts and ours stay.
        pending.retain(|p| p.thread_ts.as_deref().is_none_or(|t| Some(t) == own_thread));
    }

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
        "seat": seat.as_ref().map(|s| json!({"role": s.role, "channel": s.channel, "thread_ts": s.thread_ts})),
        "errors": errors,
        "pending": pending.iter().map(|p| json!({
            "key": p.key, "kind": p.kind, "text": p.text, "thread_ts": p.thread_ts,
            "ack": format!("bosun notify --ack {}", p.key),
        })).collect::<Vec<Value>>(),
    }));
    for p in &pending {
        let route = match (&p.thread_ts, seat.as_ref().map(|s| s.role.as_str())) {
            (Some(ts), Some("channel")) => Some((
                "send_message",
                json!({"to": "thread", "thread_ts": ts, "message": p.text}),
            )),
            (Some(ts), Some("thread")) if Some(ts.as_str()) == own_thread => {
                Some(("thread_reply", json!({"thread_ts": ts, "text": p.text})))
            }
            _ => None,
        };
        if let Some((tool, args)) = route {
            out = out.with_next(tool, args);
            continue;
        }
        let mut args = json!({"channel": channel, "text": p.text});
        if let Some(ts) = &p.thread_ts {
            args["thread_ts"] = json!(ts);
        }
        out = out.with_next("slack_post", args);
    }
    Ok(out)
}

/// This session's own Claude Tag seat from the registry. Read-only: a failing read goes to
/// `errors` and changes nothing.
fn own_seat(app: &App, errors: &mut Vec<Value>) -> Option<SlackSeat> {
    match registry::seat_of(app.gh.as_ref(), &app.ctx.session, app.ctx.now) {
        Ok(seat) => seat,
        Err(e) => {
            errors.push(json!({"kind": "seat", "message": e.message}));
            None
        }
    }
}
