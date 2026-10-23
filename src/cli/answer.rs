//! `bosun answer <issue> --slack-user U... [--from -|FILE]`: an owner's answer from Slack
//! (spec section 10). Only an owner Slack ID counts; the text is fenced as data and never
//! becomes a flag, a label or a command.

use std::io::Read;
use std::path::Path;

use serde_json::json;

use super::AnswerArgs;
use crate::app::{App, default_branch, kill_switch, notes, set_state};
use crate::cli::risk_check::{OWNERS_PATH, owner_slack_ids};
use crate::core::records::{Record, parse_record};
use crate::core::state::{State, state_of};
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

/// The text made safe to fence: it cannot close the fence or forge a record line.
fn neutralise(text: &str) -> String {
    text.replace('`', "\u{02cb}")
        .replace(r#"{"bosun""#, r#"{ "bosun""#)
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
    let asked = notes(gh, n)?
        .iter()
        .any(|c| matches!(parse_record(&c.body), Some(Record::Ask { .. })));
    if !asked {
        return Err(BosunError::conflict(
            format!("#{n} is needs-human but has no ask record"),
            format!("#{n} was not set by `bosun ask`; the owner answers it on GitHub"),
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
    let body = format!(
        "Answer from the owner via Slack\n\n```text\n{}\n```\n\n{}",
        neutralise(text.trim_end()),
        record.to_line()
    );
    let comment = gh.add_comment(n, &body)?;
    set_state(gh, n, &issue, State::Ready)?;
    Ok(Outcome::data(
        json!({"issue": n, "comment_id": comment.id, "state": "ready"}),
    ))
}
