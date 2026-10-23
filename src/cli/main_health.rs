//! `bosun main-health --result R` (spec section 9): keeps one main-red issue open while main is
//! red. It does not check the kill switch: it reports health and starts no work.

use std::path::Path;

use serde_json::{Value, json};

use super::MainHealthArgs;
use super::lesson::capture_lesson;
use crate::app::{App, MAIN_RED_LABEL, main_red};
use crate::gates::main_open::judge_pr;
use crate::github::{IssuePatch, IssueState, PatchState};
use crate::output::{BosunError, CmdResult, Outcome};

pub fn run(dir: Option<&Path>, a: &MainHealthArgs) -> CmdResult {
    let result = a.result.trim();
    if result.is_empty() {
        return Err(BosunError::usage(
            "--result is empty",
            "pass the CI conclusion: --result success|failure|cancelled|skipped",
        ));
    }
    let app = App::load(dir)?;
    let gh = app.gh.as_ref();
    let url = a.run_url.as_deref().unwrap_or("(no run URL given)");
    let open = main_red(gh)?;
    let (mut opened, mut closed, mut lesson) = (false, false, Value::Null);
    let mut red = open;
    match (result, open) {
        ("failure", None) => {
            gh.ensure_label(
                MAIN_RED_LABEL,
                "B60205",
                "main is red; only mainfix work continues",
            )?;
            let body = format!(
                "A run on main failed: {url}. Feature work pauses; fix it with a bosun:kind/mainfix PR (spec §9)."
            );
            let issue = gh.create_issue("main is red", &body, &[MAIN_RED_LABEL])?;
            red = Some(issue.number);
            opened = true;
            let c = capture_lesson(
                gh,
                &app.ctx.session,
                "main-red",
                "main went red",
                "A run on the default branch failed; main-red opened.",
                Some(issue.number),
            )?;
            lesson = json!({"issue": c.issue, "created": c.created, "hits": c.hits});
        }
        ("success", Some(n)) => {
            gh.add_comment(n, &format!("main is green again: {url}"))?;
            gh.update_issue(
                n,
                IssuePatch {
                    state: Some(PatchState::Closed),
                    state_reason: Some("completed".into()),
                    ..Default::default()
                },
            )?;
            red = None;
            closed = true;
        }
        _ => {}
    }
    let changed = opened || closed;
    let (mut rejudged, mut errors) = (0, Vec::new());
    if changed {
        for pull in gh.list_pulls(None, IssueState::Open)? {
            match judge_pr(gh, &pull, red) {
                Ok(_) => rejudged += 1,
                Err(e) => errors.push(json!({"pr": pull.number, "error": e.message})),
            }
        }
    }
    Ok(Outcome::data(json!({
        "result": result, "changed": changed, "main_red": red, "opened": opened,
        "closed": closed, "lesson": lesson, "rejudged": rejudged, "rejudge_errors": errors,
        "note": "main-health reports health; it checks no kill switch and starts no work",
    })))
}
