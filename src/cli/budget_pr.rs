//! `bosun budget record --pr`: write the spend of a session on each PR it made (spec §9).

use std::path::Path;

use serde_json::{Value, json};

use super::BudgetRecordArgs;
use super::budget::{read_input, usd_of};
use crate::app::{App, kill_switch, spend_notes};
use crate::core::meta::{linked_issues_in, parse_task_meta};
use crate::core::records::{Record, parse_record};
use crate::core::state::state_of;
use crate::output::{BosunError, CmdResult, Outcome};
use crate::sessions::usage;

fn text<'a>(v: &'a Value, paths: &[&[&str]]) -> Option<&'a str> {
    paths
        .iter()
        .find_map(|p| p.iter().try_fold(v, |v, k| v.get(k))?.as_str())
}

pub fn run(dir: Option<&Path>, a: &BudgetRecordArgs) -> CmdResult {
    let app = App::load(dir)?;
    let (gh, ctx) = (app.gh.as_ref(), &app.ctx);
    kill_switch(gh)?;
    let at = match a.at.as_deref() {
        Some(s) => super::budget::parse_at(s)?,
        None => ctx.now,
    };
    let mut prs = a.pr.clone();
    prs.sort_unstable();
    prs.dedup();
    let input = a.from.as_deref().map(read_input).transpose()?;
    // stdin is read once: the cost comes from the input already read.
    let mut usd = match (a.usd, &input) {
        (Some(u), _) => u,
        (None, Some(v)) if a.provider == "claude-sessions" => usage::from_claude_session(v)?,
        _ => usd_of(a, &ctx.config.prices)?,
    };
    if !usd.is_finite() || usd < 0.0 {
        return Err(BosunError::error(
            format!("the cost {usd} is negative or not a number"),
            "pass the unchanged get_session result, or --usd with a cost of 0 or more",
        ));
    }
    let session = a.session.clone().unwrap_or_else(|| {
        input
            .as_ref()
            .and_then(|v| {
                text(
                    v,
                    &[
                        &["session_id"],
                        &["session", "id"],
                        &["id"],
                        &["ccr", "session_id"],
                        &["ccr", "id"],
                    ],
                )
            })
            .unwrap_or(&ctx.session)
            .to_string()
    });
    let model = a.model.clone().or_else(|| {
        let v = input.as_ref()?;
        text(
            v,
            &[
                &["session_context", "model"],
                &["external_metadata", "last_served_model"],
                &["model"],
                &["ccr", "session_context", "model"],
                &["ccr", "external_metadata", "last_served_model"],
            ],
        )
        .map(String::from)
    });
    // A session total from get_session already counted on a task (the registry ledger) is not
    // counted again on the PRs.
    if a.usd.is_none() && input.is_some() && a.provider == "claude-sessions" {
        let held =
            super::budget::session_spent(&crate::registry::trusted_spend_notes(gh)?, &session);
        usd = (usd - held).max(0.0);
    }
    // Check every PR before writing any. A PR that already has this session's record is left
    // alone, so a retry after a failed write adds only what is missing.
    let (mut todo, mut done) = (Vec::new(), Vec::new());
    for &pr in &prs {
        let Some(pull) = gh.get_pull(pr)? else {
            return Err(BosunError::error(
                format!("PR #{pr} does not exist"),
                "check the number; pass --pr once per PR the session made",
            ));
        };
        // A task's spend goes to its issue: the per-task cap and the daily guard see only those.
        for n in linked_issues_in(&pull.body, &ctx.repo.to_string()) {
            let is_task = gh.get_issue(n)?.is_some_and(|i| {
                state_of(&i.labels).is_some() || parse_task_meta(i.body_text()).is_some()
            });
            if is_task {
                return Err(BosunError::conflict(
                    format!("PR #{pr} is the PR of task #{n}: its spend belongs on the task"),
                    format!(
                        "run `bosun budget record {n} --from -` instead (--pr is for PRs without a task issue)"
                    ),
                ));
            }
        }
        let seen = spend_notes(gh, pr)?.iter().any(|n| {
            matches!(parse_record(&n.body), Some(Record::Spend { session: s, .. }) if s == session)
        });
        if seen { done.push(pr) } else { todo.push(pr) }
    }
    if todo.is_empty() {
        return Err(BosunError::conflict(
            format!("session {session} already has a spend record on PR(s) {done:?}"),
            "do not record it again; the spend is already on the PR",
        ));
    }
    let share = usd / prs.len() as f64;
    for &pr in &todo {
        let spend = Record::Spend {
            session: session.clone(),
            usd: share,
            at: at.to_string(),
            issue: None,
            model: model.clone(),
        };
        gh.add_comment(pr, &spend.to_line())?;
    }
    Ok(Outcome::data(json!({
        "prs": prs, "recorded": todo, "already_recorded": done, "usd": usd, "usd_per_pr": share, "session": session, "model": model,
    })))
}
