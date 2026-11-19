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
use crate::cli::pr_no_ci::clean_by_other;
use crate::cli::risk_check::{chat_enabled, owner_slack_ids};
use crate::core::approve::{chat_comment, is_orchestrator, parse_target};
use crate::ctx::Origin;
use crate::hooks::answers;
use crate::output::{BosunError, CmdResult, Outcome};

const ASK_AGAIN: &str = "ask the owner again: run `bosun approve --list` and its AskUserQuestion next, then `bosun approve <pr>@<head> --chat --question <data.ready[].ask.question_id>`";

pub fn run(dir: Option<&Path>, args: &ApproveArgs) -> CmdResult {
    let target = match args.prs.as_slice() {
        [t] => parse_target(t),
        _ => None,
    };
    // From Claude Tag the owner types `approve <short7>`: resolved against the open PRs below.
    let short = match (args.prs.as_slice(), &args.slack_user) {
        ([t], Some(_)) if target.is_none() && is_short(t) => Some(t.as_str()),
        _ => None,
    };
    if short.is_none() && target.is_none() {
        return Err(BosunError::usage(
            "--chat takes one <pr>@<40-hex head sha>: the head the owner approved",
            "run `bosun approve --list`, ask the owner, then `bosun approve <pr>@<head> --chat --question <id>`",
        ));
    }
    if args.question.is_none() {
        return Err(BosunError::usage(
            "--chat needs --question <id> (data.ready[].ask.question_id), also with --slack-user U...",
            ASK_AGAIN,
        ));
    }
    let app = App::load(dir)?;
    let (pr, head) = match (target, short) {
        (Some(t), _) => t,
        (None, Some(short)) => {
            resolve_short(&app, short, args.question.as_deref().unwrap_or_default())?
        }
        (None, None) => unreachable!("checked above"),
    };
    Ok(Outcome::data(approve_chat(&app, pr, &head, args)?))
}

fn is_short(t: &str) -> bool {
    t.len() == 7
        && t.bytes()
            .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
}

/// The one PR in the current ready list (`approve --list`) whose head starts with `short`:
/// zero or several matches refuse (exit 3), so a short head typed in Slack never picks a PR by
/// guess. The PR named by `--question` must be that PR.
fn resolve_short(app: &App, short: &str, question: &str) -> Result<(u64, String)> {
    let board = crate::cli::approve::board(app.gh.as_ref(), &app.ctx, true)?;
    let mut hit = board.ready.iter().filter(|r| r.head_sha.starts_with(short));
    let (pr, head) = match (hit.next(), hit.next()) {
        (Some(r), None) => (r.pr, r.head_sha.clone()),
        (None, _) => {
            return Err(BosunError::conflict(
                format!(
                    "no PR in the ready list has a head starting with {short}; nothing was recorded"
                ),
                ASK_AGAIN,
            ));
        }
        _ => {
            return Err(BosunError::conflict(
                format!(
                    "several PRs in the ready list have a head starting with {short}; nothing was recorded"
                ),
                "ask again naming one PR: `bosun approve <pr>@<head> --chat --slack-user U... --question <id>`",
            ));
        }
    };
    if !question.starts_with(&format!("{pr}-")) {
        return Err(BosunError::conflict(
            format!(
                "question {question:?} names another PR than #{pr} (head {short}); nothing was recorded"
            ),
            ASK_AGAIN,
        ));
    }
    Ok((pr, head))
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
    // The child flag is read directly too: BOSUN_ORIGIN can relabel `ctx.origin`, the flag
    // Claude Code sets for a child session cannot be dropped by it.
    let child_flag = std::env::var("CLAUDE_CODE_CHILD_SESSION").is_ok_and(|v| !v.trim().is_empty());
    if child_flag || app.ctx.origin == Origin::Spawned {
        return Err(BosunError::needs_human(
            "this session is a spawned child (CLAUDE_CODE_CHILD_SESSION set or BOSUN_ORIGIN=spawned); only an orchestrator records a chat approval",
            "hand the PR to the orchestrator with `bosun handoff-pr`; it asks the owner and records the answer",
        ));
    }
    // Every comment, not only writers': in a cloud session the permission lookup is hidden, so
    // the session's own join would not count as trusted. Only risk-check's trusted read counts a
    // record; this check guards a confused worker.
    let members = crate::registry::load_own(gh)?
        .map(|(_, notes)| crate::core::lease::members(&notes))
        .unwrap_or_default();
    if !is_orchestrator(&members, session, app.ctx.now) {
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
    if clean_by_other(app, &pull)?.is_empty() {
        return Err(BosunError::needs_human(
            format!(
                "no clean verdict on head {head} from a session other than the author and this one"
            ),
            "get one independent clean verdict with `bosun verdict comment --from -` in a reviewer session, then record again",
        ));
    }
    match &args.slack_user {
        // Both the env origin and the registry join: BOSUN_ORIGIN alone is set by the session.
        Some(_) if app.ctx.origin != Origin::Slack || !joined_from_slack(&members, session) => {
            return Err(BosunError::needs_human(
                "--slack-user records an answer given in a Claude Tag thread; this session's origin (and its registry join) is not slack",
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
            // A Slack answer is not stored; the id still names this PR and head, and a hold wins.
            let q = args.question.as_deref().unwrap_or_default();
            let repo = app.ctx.repo.to_string();
            let named = q.starts_with(&format!("{pr}-{}-", &head[..7]));
            // The id must be an ask `approve --list` issued as a Slack post for this PR and head.
            let issued = answers::asked(&app.ctx.dir, &repo, q)
                .is_some_and(|a| a.pr == pr && a.head == head);
            if !named || !issued || answers::held(&app.ctx.dir, &repo, pr, head) {
                return Err(BosunError::needs_human(
                    format!(
                        "question {q:?} does not name #{pr} at {head}, was not issued by `approve --list` as a Claude Tag ask, or the owner held it; nothing was recorded"
                    ),
                    ASK_AGAIN,
                ));
            }
        }
        None => {
            let q = args.question.as_deref().unwrap_or_default();
            stored_approve(&app.ctx, q, pr, head)?;
        }
    };
    let question = args.question.as_deref();
    let body = chat_comment(head, session, &app.ctx.now.to_string(), question);
    let c = gh.add_comment(pr, &body)?;
    Ok(json!({"pr": pr, "head_sha": head, "comment_id": c.id, "question_id": question}))
}

/// Whether `session`'s newest registry join has origin `slack`.
fn joined_from_slack(members: &[crate::core::lease::Member], session: &str) -> bool {
    (members.iter())
        .filter(|m| m.session == session)
        .max_by_key(|m| m.comment_id)
        .is_some_and(|m| m.origin == "slack")
}

/// The owner's stored answer must be `approve` for exactly `pr` at `head` (the id names them:
/// `<pr>-<short head>-<unix>`), the newest answer for that PR and head (an older approve on
/// another question under a newer hold refuses), and given in this orchestrator session: a
/// missing, empty or other session id never counts. The file is only a hint against confused
/// workers, never proof.
fn stored_approve(ctx: &crate::ctx::Ctx, q: &str, pr: u64, head: &str) -> Result<()> {
    let (dir, repo, session) = (&ctx.dir, ctx.repo.to_string(), &ctx.session);
    let prefix = format!("{pr}-{}-", &head[..7]);
    let mine = |a: &answers::Answer| {
        a.answer == "approve"
            && a.pr == pr
            && a.head == head
            && !a.session.is_empty()
            && a.session == *session
    };
    let ok = q.starts_with(&prefix)
        && answers::current(dir, &repo, pr, head).is_some_and(|a| mine(&a))
        && answers::get(dir, &repo, q).is_some_and(|a| mine(&a));
    if ok {
        return Ok(());
    }
    Err(BosunError::needs_human(
        format!("no stored owner answer approving #{pr} at {head} for question {q:?}"),
        ASK_AGAIN,
    ))
}
