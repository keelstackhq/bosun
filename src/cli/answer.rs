//! `bosun answer <issue> --slack-user U... [--from -|FILE]`: an owner's answer from Slack
//! (spec section 10). Only an owner Slack ID counts; the text is fenced as data and never
//! becomes a flag, a label or a command. A Slack answer is recorded only: it never changes the
//! task's state, the owner moves the task on GitHub. `--chat` takes the owner's answer relayed
//! in chat by the orchestrator: it counts only with a verified signature of an enrolled owner
//! key, and then moves a needs-human task to ready.

use std::collections::HashMap;
use std::io::Read;
use std::path::Path;

use serde_json::{Value, json};

use super::AnswerArgs;
use crate::app::{App, default_branch, kill_switch, main_red, set_state, task_meta};
use crate::cli::approve::read_key;
use crate::cli::intake::fence_text;
use crate::cli::risk_check::{OWNERS_PATH, enrolled_keys, owner_slack_ids};
use crate::core::answer::{answer_message, chat_form, normalize, parse_chat, text_sha256};
use crate::core::approve::{APPROVERS_PATH, Role, verify_ed25519};
use crate::core::records::{Record, parse_record};
use crate::core::state::{State, state_of};
use crate::github::{Github, Issue, b64};
use crate::output::{BosunError, CmdResult, Outcome};

fn valid_slack_id(id: &str) -> bool {
    let b = id.as_bytes();
    b.len() >= 3
        && matches!(b[0], b'U' | b'W')
        && b[1..]
            .iter()
            .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit())
}

fn read_text(from: &str) -> Result<String, BosunError> {
    let mut text = String::new();
    let res = if from == "-" {
        std::io::stdin().read_to_string(&mut text).map(|_| ())
    } else {
        std::fs::read_to_string(from).map(|t| text = t)
    };
    res.map_err(|e| {
        BosunError::usage(
            format!("cannot read the answer from {from}: {e}"),
            "pass the answer text on stdin or with --from FILE",
        )
    })?;
    Ok(text)
}

/// The comment id of the open ask: the newest ask by a collaborator with write access, when no
/// `answer`, `escalate` or `failure` record follows it. A forged ask by anyone else never counts.
fn open_ask(gh: &dyn Github, n: u64) -> Result<Option<u64>, BosunError> {
    let comments = gh.list_comments(n)?;
    let mut cache: HashMap<String, bool> = HashMap::new();
    let mut newest_ask = None;
    let mut closed_at = None;
    for (i, c) in comments.iter().enumerate() {
        let Some(record) = parse_record(&c.body) else {
            continue;
        };
        let is_ask = matches!(record, Record::Ask { .. });
        let closes = matches!(
            record,
            Record::Answer { .. } | Record::Escalate { .. } | Record::Failure { .. }
        );
        if !is_ask && !closes {
            continue;
        }
        // Asks and closers count only from a collaborator with write access.
        let trusted = match cache.get(&c.user) {
            Some(t) => *t,
            None => {
                let perm = gh.permission(&c.user)?.unwrap_or_default();
                let t = matches!(perm.as_str(), "admin" | "write");
                cache.insert(c.user.clone(), t);
                t
            }
        };
        if !trusted {
            continue;
        }
        if is_ask {
            newest_ask = Some(i);
        } else {
            closed_at = Some(i);
        }
    }
    Ok(match (newest_ask, closed_at) {
        (Some(a), c) if c.is_none_or(|c| c < a) => Some(comments[a].id),
        _ => None,
    })
}

pub fn run(dir: Option<&Path>, args: &AnswerArgs) -> CmdResult {
    let app = App::load(dir)?;
    let (gh, n) = (app.gh.as_ref(), args.issue);
    kill_switch(gh)?;
    if let Some(user) = &args.slack_user
        && !valid_slack_id(user)
    {
        return Err(BosunError::usage(
            "--slack-user is not a Slack user ID",
            "pass the ID from the Claude Tag event, such as U01ABC23",
        ));
    }
    let text = read_text(&args.from)?;
    if text.trim().is_empty() {
        return Err(BosunError::usage(
            "the answer text is empty",
            "pass the owner's answer on stdin or with --from FILE",
        ));
    }
    let issue = gh.get_issue(n)?.ok_or_else(|| {
        BosunError::error(
            format!("issue #{n} does not exist"),
            "check the number; `bosun status` lists the tasks",
        )
    })?;
    let state = state_of(&issue.labels);
    // A signed chat answer is recorded whatever the state; only a needs-human task moves.
    if state != Some(State::NeedsHuman) && args.slack_user.is_some() {
        let now = state.map_or("none".to_string(), |s| s.to_string());
        return Err(BosunError::conflict(
            format!("#{n} is {now}, not needs-human"),
            format!(
                "#{n} is {now}: only a task that `bosun ask` set to needs-human takes an answer"
            ),
        ));
    }
    let Some(ask) = open_ask(gh, n)? else {
        let now = state.map_or("none".to_string(), |s| s.to_string());
        return Err(BosunError::conflict(
            format!("#{n} is {now} but has no open ask"),
            format!(
                "no open ask on #{n}: the owner answers on GitHub, or a session runs `bosun ask`"
            ),
        ));
    };
    let slack_user = match &args.slack_user {
        Some(u) => u.clone(),
        None => return chat(&app, args, &issue, ask, &text),
    };
    let branch = default_branch(gh)?;
    let ids = owner_slack_ids(gh, &branch)?;
    if !ids.contains(&slack_user) {
        let why = if ids.is_empty() {
            "no owner has a Slack ID"
        } else {
            "not an owner Slack ID"
        };
        return Err(BosunError::needs_human(
            format!("the answer does not count: {why}"),
            format!(
                "an owner answers on GitHub, or adds `slack = \"U...\"` to their [[owner]] table in {OWNERS_PATH}"
            ),
        )
        .with_data(json!({"issue": n, "reason": why})));
    }
    let record = Record::Answer {
        session: app.ctx.session.clone(),
        slack_user,
        at: app.ctx.now.to_string(),
        key: None,
        issue: None,
        ask: None,
        sha256: None,
        sig: None,
    };
    let head = "Answer from the owner via Slack (sender id not verified by bosun)";
    let mut data = post(gh, n, head, &text, &record)?;
    // The answer is data: the owner moves the task on GitHub.
    data["state"] = json!("needs-human");
    Ok(Outcome::data(data))
}

/// Post the fenced answer comment with its record; returns data with the issue and comment.
fn post(
    gh: &dyn Github,
    n: u64,
    head: &str,
    text: &str,
    record: &Record,
) -> Result<Value, BosunError> {
    let truncated = text.trim_end().chars().count() > 4000;
    let cut = if truncated {
        "(cut at 4000 characters)\n\n"
    } else {
        ""
    };
    let body = format!(
        "{head}\n\n```text\n{}\n```\n\n{cut}{}",
        fence_text(text.trim_end()),
        record.to_line()
    );
    let comment = gh.add_comment(n, &body)?;
    let mut data = json!({"issue": n, "comment_id": comment.id});
    if truncated {
        data["truncated"] = json!(true);
    }
    Ok(data)
}

/// What the orchestrator does when a chat answer does not count.
const CHAT_FIX: &str = "relay the owner's answer again in chat form, signed for the open ask with an enrolled owner key (`bosun answer <issue> --chat --sign-key <pem>`), or the owner answers on GitHub";

/// `--chat`: verify the relayed chat form and record it, or with `--sign-key` sign the owner's
/// plain answer and print the chat form (nothing is posted).
fn chat(app: &App, args: &AnswerArgs, issue: &Issue, ask: u64, text: &str) -> CmdResult {
    let (gh, n) = (app.gh.as_ref(), args.issue);
    let (owner, repo) = (&app.ctx.repo.owner, &app.ctx.repo.name);
    let keys = enrolled_keys(gh).map_err(|why| {
        BosunError::error(
            format!("signed answers are off: {why}"),
            format!("enroll the orchestrator's key in {APPROVERS_PATH} on the default branch with a PR the owner approves on GitHub"),
        )
    })?;
    if let Some(path) = &args.sign_key {
        let pair = read_key(path)?;
        let public = b64::encode(ring::signature::KeyPair::public_key(&pair).as_ref());
        let key = keys
            .iter()
            .find(|k| b64::decode(&k.ed25519).map(|b| b64::encode(&b)) == Some(public.clone()))
            .filter(|k| k.role == Role::Owner)
            .ok_or_else(|| {
                BosunError::error(
                    format!("the key is not an owner key in {APPROVERS_PATH} on the default branch; nothing was signed"),
                    format!("enroll the public key {public} as an owner key in {APPROVERS_PATH} with a PR the owner approves on GitHub"),
                )
            })?;
        let sig = pair.sign(answer_message(owner, repo, n, ask, text).as_bytes());
        let form = chat_form(n, ask, text, &key.id, &b64::encode(sig.as_ref()));
        return Ok(Outcome::data(
            json!({"issue": n, "ask": ask, "key_id": key.id, "chat": form}),
        ));
    }
    let refuse = |why: &str| {
        BosunError::needs_human(format!("the chat answer does not count: {why}"), CHAT_FIX)
            .with_data(json!({"issue": n, "reason": why}))
    };
    let form = parse_chat(text).ok_or_else(|| refuse("not in chat form"))?;
    if form.issue != n {
        return Err(refuse("signed for another issue"));
    }
    if form.ask != ask {
        return Err(BosunError::conflict(
            format!(
                "the chat answer is for ask comment {}, the open ask on #{n} is {ask}",
                form.ask
            ),
            CHAT_FIX,
        ));
    }
    let (key_id, sig) = form.sig.ok_or_else(|| refuse("not signed"))?;
    let key = keys
        .iter()
        .find(|k| k.id == key_id)
        .ok_or_else(|| refuse("key not enrolled"))?;
    if key.role != Role::Owner {
        return Err(refuse("not an owner key"));
    }
    let msg = answer_message(owner, repo, n, ask, &form.text);
    let verified = match (b64::decode(&key.ed25519), b64::decode(sig)) {
        (Some(public), Some(raw)) => verify_ed25519(&public, msg.as_bytes(), &raw),
        _ => false,
    };
    if !verified {
        return Err(refuse("bad signature"));
    }
    // Only a needs-human task moves, and never while main is red, except a mainfix task (as
    // `bosun claim`): exit 4 before anything is written, so the same signed answer can be
    // relayed again once main is green. The kill switch was checked before any read.
    let state = state_of(&issue.labels);
    let moves = state == Some(State::NeedsHuman);
    let mainfix = task_meta(issue).is_some_and(|m| m.kind == "mainfix")
        || issue.labels.iter().any(|l| l == "bosun:kind/mainfix");
    if moves
        && !mainfix
        && let Some(red) = main_red(gh)?
    {
        return Err(BosunError::halted(
            format!(
                "main is red (#{red}); the answer would move #{n} to ready, so nothing was recorded"
            ),
            format!("wait for #{red} to close, then relay the same signed answer again"),
        ));
    }
    let record = Record::Answer {
        session: app.ctx.session.clone(),
        slack_user: String::new(),
        at: app.ctx.now.to_string(),
        key: Some(key.id.clone()),
        issue: Some(n),
        ask: Some(ask),
        sha256: Some(text_sha256(&form.text)),
        sig: Some(sig.to_string()),
    };
    let head = format!(
        "Answer from the owner, relayed in chat (signature of key {} verified by bosun)",
        key.id
    );
    let mut data = post(gh, n, &head, &normalize(&form.text), &record)?;
    data["key_id"] = json!(key.id);
    data["ask"] = json!(ask);
    if moves {
        set_state(gh, n, issue, State::Ready).map_err(|e| {
            let fix = format!(
                "the answer is recorded (comment {}); move #{n} from needs-human to ready on GitHub",
                data["comment_id"]
            );
            BosunError::error(format!("the answer is recorded but #{n} did not move: {}", e.message), fix)
                .with_data(data.clone())
        })?;
        data["state"] = json!("ready");
        data["moved"] = json!(true);
    } else {
        let now = state.map_or("none".to_string(), |s| s.to_string());
        data["state"] = json!(now);
        data["moved"] = json!(false);
        data["not_moved"] = json!(format!(
            "#{n} is {now}, not needs-human: the answer is recorded and the state is left alone"
        ));
    }
    Ok(Outcome::data(data))
}
