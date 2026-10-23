//! `bosun answer <issue> --slack-user U... [--from -|FILE]`: an owner's answer from Slack
//! (spec section 10). Only an owner Slack ID counts; the text is fenced as data and never
//! becomes a flag, a label or a command. The answer is recorded only: it never changes the
//! task's state, the owner moves the task on GitHub.

use std::collections::HashMap;
use std::io::Read;
use std::path::Path;

use serde_json::json;

use super::AnswerArgs;
use crate::app::{App, default_branch, kill_switch};
use crate::cli::intake::fence_text;
use crate::cli::risk_check::{OWNERS_PATH, owner_slack_ids};
use crate::core::records::{Record, parse_record};
use crate::core::state::{State, state_of};
use crate::github::Github;
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

/// The newest ask by a collaborator with write access is open when no `answer`, `escalate` or
/// `failure` record follows it. A forged ask by anyone else never counts.
fn ask_is_open(gh: &dyn Github, n: u64) -> Result<bool, BosunError> {
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
    Ok(matches!((newest_ask, closed_at), (Some(a), c) if c.is_none_or(|c| c < a)))
}

pub fn run(dir: Option<&Path>, args: &AnswerArgs) -> CmdResult {
    let app = App::load(dir)?;
    let (gh, n) = (app.gh.as_ref(), args.issue);
    kill_switch(gh)?;
    if !valid_slack_id(&args.slack_user) {
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
    if state != Some(State::NeedsHuman) {
        let now = state.map_or("none".to_string(), |s| s.to_string());
        return Err(BosunError::conflict(
            format!("#{n} is {now}, not needs-human"),
            format!(
                "#{n} is {now}: only a task that `bosun ask` set to needs-human takes an answer"
            ),
        ));
    }
    if !ask_is_open(gh, n)? {
        return Err(BosunError::conflict(
            format!("#{n} is needs-human but has no open ask"),
            format!(
                "no open ask on #{n}: the owner answers on GitHub, or a session runs `bosun ask`"
            ),
        ));
    }
    let branch = default_branch(gh)?;
    let ids = owner_slack_ids(gh, &branch)?;
    if !ids.contains(&args.slack_user) {
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
        slack_user: args.slack_user.clone(),
        at: app.ctx.now.to_string(),
    };
    let truncated = text.trim_end().chars().count() > 4000;
    let cut = if truncated {
        "(cut at 4000 characters)\n\n"
    } else {
        ""
    };
    let body = format!(
        "Answer from the owner via Slack (sender id not verified by bosun)\n\n```text\n{}\n```\n\n{cut}{}",
        fence_text(text.trim_end()),
        record.to_line()
    );
    let comment = gh.add_comment(n, &body)?;
    // The answer is data: the owner moves the task on GitHub.
    let mut data = json!({"issue": n, "comment_id": comment.id, "state": "needs-human"});
    if truncated {
        data["truncated"] = json!(true);
    }
    Ok(Outcome::data(data))
}
