//! `bosun budget record --pr`: write the spend of a session on each PR it made (spec §9).

use std::path::Path;

use serde_json::{Value, json};

use super::BudgetRecordArgs;
use super::budget::{read_input, usd_of};
use crate::app::{App, kill_switch, spend_notes};
use crate::core::records::{Record, parse_record};
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
    let mut prs = a.pr.clone();
    prs.sort_unstable();
    prs.dedup();
    let input = a.from.as_deref().map(read_input).transpose()?;
    // stdin is read once: the cost comes from the input already read.
    let usd = match (a.usd, &input) {
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
    let session = input
        .as_ref()
        .and_then(|v| text(v, &[&["session_id"], &["session", "id"], &["id"]]))
        .unwrap_or(&ctx.session)
        .to_string();
    let model = a.model.clone().or_else(|| {
        let v = input.as_ref()?;
        text(
            v,
            &[
                &["session_context", "model"],
                &["external_metadata", "last_served_model"],
                &["model"],
            ],
        )
        .map(String::from)
    });
    // Check every PR before writing any: a refusal leaves nothing half recorded.
    for &pr in &prs {
        if gh.get_pull(pr)?.is_none() {
            return Err(BosunError::error(
                format!("PR #{pr} does not exist"),
                "check the number; pass --pr once per PR the session made",
            ));
        }
        let seen = spend_notes(gh, pr)?.iter().any(|n| {
            matches!(parse_record(&n.body), Some(Record::Spend { session: s, .. }) if s == session)
        });
        if seen {
            return Err(BosunError::conflict(
                format!("session {session} already has a spend record on PR #{pr}"),
                "do not record it again; the spend is already on the PR",
            ));
        }
    }
    let share = usd / prs.len() as f64;
    for &pr in &prs {
        let spend = Record::Spend {
            session: session.clone(),
            usd: share,
            at: ctx.now.to_string(),
            issue: None,
            model: model.clone(),
        };
        gh.add_comment(pr, &spend.to_line())?;
    }
    Ok(Outcome::data(json!({
        "prs": prs, "usd": usd, "usd_per_pr": share, "session": session, "model": model,
    })))
}
