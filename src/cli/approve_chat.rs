//! `bosun approve <pr>@<head> --chat`: an orchestrator records the owner's chat yes as one PR
//! comment (spec section 8, chat approvals). It posts with the session's own token, also in
//! the cloud: risk-check counts the record through the app. Refusals, in order: chat off, not
//! an orchestrator, head moved, no clean verdict from another session, no stored owner answer
//! (or, from Claude Tag, a Slack user who is not an owner; `--slack-user` only from origin slack).

use std::path::Path;

use serde_json::json;

use super::ApproveArgs;
use crate::app::{App, Result, default_branch, kill_switch};
use crate::cli::answer::valid_slack_id;
use crate::cli::risk_check::{
    autonomous_clean, chat_enabled, owner_slack_ids, review_models, verdict_inputs,
};
use crate::core::approve::{chat_comment, is_orchestrator, parse_target};
use crate::core::meta::linked_issues_in;
use crate::ctx::Origin;
use crate::github::Pull;
use crate::hooks::answers;
use crate::output::{BosunError, CmdResult, Outcome};

const ASK_AGAIN: &str = "ask the owner again: run `bosun approve --list` and its AskUserQuestion next, then `bosun approve <pr>@<head> --chat --question <data.ready[].ask.question_id>`";

pub fn run(dir: Option<&Path>, args: &ApproveArgs) -> CmdResult {
    let target = match args.prs.as_slice() {
        [t] => parse_target(t),
        _ => None,
    };
    let (pr, head) = target.ok_or_else(|| {
        BosunError::usage(
            "--chat takes one <pr>@<40-hex head sha>: the head the owner approved",
            "run `bosun approve --list`, ask the owner, then `bosun approve <pr>@<head> --chat --question <id>`",
        )
    })?;
    if args.question.is_none() && args.slack_user.is_none() {
        return Err(BosunError::usage(
            "--chat needs --question <id> (the owner's AskUserQuestion answer) or --slack-user U...",
            ASK_AGAIN,
        ));
    }
    let app = App::load(dir)?;
    Ok(Outcome::data(approve_chat(&app, pr, &head, args)?))
}

fn approve_chat(app: &App, pr: u64, head: &str, args: &ApproveArgs) -> Result<serde_json::Value> {
    let gh = app.gh.as_ref();
    kill_switch(gh)?;
    let pull = gh
        .get_pull(pr)?
        .filter(|p| p.state == "open")
        .ok_or_else(|| {
            BosunError::conflict(format!("PR #{pr} is not an open PR"), "name an open PR")
        })?;
    if !chat_enabled(gh, &pull.base_sha)? {
        return Err(BosunError::needs_human(
            "chat approvals are off (`[approvals] chat = false` at the PR's base)",
            "the owner approves on GitHub (`/bosun approve <head>`) or with a signed approval",
        ));
    }
    let session = &app.ctx.session;
    let members = crate::registry::load(gh)?
        .map(|(_, notes)| crate::core::lease::members(&notes))
        .unwrap_or_default();
    if !is_orchestrator(&members, session) {
        return Err(BosunError::needs_human(
            format!(
                "session {session} is not an orchestrator in the registry; its chat record would not count"
            ),
            "hand the PR to the orchestrator with `bosun handoff-pr`; it asks the owner and records the answer",
        ));
    }
    if pull.head_sha != head {
        return Err(BosunError::conflict(
            format!(
                "PR #{pr} head changed (now {}); nothing was recorded",
                pull.head_sha
            ),
            "show the owner the new head and record it only on their new answer",
        ));
    }
    if !clean_verdict_by_other(app, &pull)? {
        return Err(BosunError::needs_human(
            format!(
                "no clean verdict on head {head} from a session other than the author and this one"
            ),
            "get one independent clean verdict with `bosun verdict comment --from -` in a reviewer session, then record again",
        ));
    }
    let question = match &args.slack_user {
        Some(_) if app.ctx.origin != Origin::Slack => {
            return Err(BosunError::needs_human(
                "--slack-user records an answer given in a Claude Tag thread; this session's origin is not slack",
                ASK_AGAIN,
            ));
        }
        Some(user) => {
            let ids = owner_slack_ids(gh, &default_branch(gh)?)?;
            if !valid_slack_id(user) || !ids.contains(user) {
                return Err(BosunError::needs_human(
                    format!("{user:?} is not an owner Slack ID; nothing was recorded"),
                    "only an owner's answer counts: an owner adds `slack = \"U...\"` to their [[owner]] table in governance/owners.toml, or answers on GitHub",
                ));
            }
            None
        }
        None => {
            let q = args.question.as_deref().unwrap_or_default();
            stored_approve(&app.ctx.dir, q, pr, head)?;
            Some(q)
        }
    };
    let body = chat_comment(head, session, &app.ctx.now.to_string(), question);
    let c = gh.add_comment(pr, &body)?;
    Ok(json!({"pr": pr, "head_sha": head, "comment_id": c.id, "question_id": question}))
}

/// The owner's stored answer to `q` must be `approve` for exactly `pr` at `head`, and the id must
/// name them (`<pr>-<short head>-<unix>`). The file is only a guard against confused workers.
fn stored_approve(dir: &Path, q: &str, pr: u64, head: &str) -> Result<()> {
    let prefix = format!("{pr}-{}-", &head[..7]);
    let ok = q.starts_with(&prefix)
        && answers::get(dir, q)
            .is_some_and(|a| a.answer == "approve" && a.pr == pr && a.head == head);
    if ok {
        return Ok(());
    }
    Err(BosunError::needs_human(
        format!("no stored owner answer approving #{pr} at {head} for question {q:?}"),
        ASK_AGAIN,
    ))
}

/// A clean verdict on the head from a session that is neither a lease holder of a linked task
/// (the author) nor this session (shared with D-017's rule, without its task requirement).
fn clean_verdict_by_other(app: &App, pull: &Pull) -> Result<bool> {
    let gh = app.gh.as_ref();
    let linked = linked_issues_in(&pull.body, &app.ctx.repo.to_string());
    let (comments, mut excluded) = verdict_inputs(gh, pull.number, &linked)?;
    excluded.push(app.ctx.session.clone());
    let models = review_models(gh, &pull.base_sha)?;
    Ok(!autonomous_clean(&comments, &pull.head_sha, &excluded, models.as_deref()).is_empty())
}
