//! `bosun handoff-pr <pr> [--to SESSION]`: a child tells its parent that its PR is ready. Returns
//! a `send_message` call with the PR, head SHA, risk paths and review minors; sends nothing and
//! changes nothing (spec D29). Not `bosun handoff`, which is a root's lineage handoff.

use std::path::Path;

use serde_json::json;

use super::HandoffPrArgs;
use crate::app::{App, trusted_notes};
use crate::core::handoff::{minors, parent_of, pr_message};
use crate::core::lease::orchestrator_of;
use crate::core::risk::note_paths;
use crate::output::{BosunError, CmdResult, Outcome};
use crate::registry;

pub fn run(dir: Option<&Path>, args: &HandoffPrArgs) -> CmdResult {
    let app = App::load(dir)?;
    let (gh, session) = (app.gh.as_ref(), &app.ctx.session);
    let n = args.pr;
    let pull = gh.get_pull(n)?.ok_or_else(|| {
        BosunError::error(
            format!("PR #{n} does not exist"),
            "check the number; `bosun pr <issue>` prints your PR",
        )
    })?;
    let parent = match &args.to {
        Some(to) => to.clone(),
        None => {
            let joined = match registry::load(gh)? {
                Some((number, _)) => parent_of(&trusted_notes(gh, number)?, session),
                None => None,
            };
            joined
                .or_else(|| {
                    session
                        .contains('/')
                        .then(|| orchestrator_of(session).to_string())
                })
                .ok_or_else(|| {
                    BosunError::usage(
                        format!("{session} has no registered parent"),
                        "pass the parent session with --to <session_id> (it is in your prompt)",
                    )
                })?
        }
    };
    let mut comments = gh.list_comments(n)?;
    comments.sort_by_key(|c| c.id);
    let minors = minors(comments.iter().map(|c| c.body.as_str()), &pull.head_sha);
    let risk = note_paths(&pull.body);
    let message = pr_message(session, n, &pull.head_sha, &risk, &minors);
    Ok(Outcome::data(json!({
        "pr": n, "head_sha": pull.head_sha, "parent": parent,
        "risk_paths": risk, "review_minors": minors,
    }))
    .with_next(
        "send_message",
        json!({"session_id": parent, "message": message}),
    ))
}
