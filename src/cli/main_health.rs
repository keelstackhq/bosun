//! `bosun main-health --result R` (spec section 9): keeps one main-red issue open while main is
//! red. It does not check the kill switch: it reports health and starts no work.

use std::path::Path;

use serde_json::{Value, json};

use super::MainHealthArgs;
use super::lesson::capture_lesson;
use crate::app::{App, MAIN_RED_LABEL};
use crate::gates::main_open::judge_pr;
use crate::github::{Github, IssuePatch, IssueState, PatchState};
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
    let is_red = matches!(result, "failure" | "timed_out" | "startup_failure");
    let (mut opened, mut closed, mut lesson) = (false, false, Value::Null);
    let mut open = open_reds(gh)?;
    let mut red = open.first().copied();
    if is_red && open.is_empty() {
        gh.ensure_label(
            MAIN_RED_LABEL,
            "B60205",
            "main is red; only mainfix work continues",
        )?;
        let body = format!(
            "A run on main failed: {url}. Feature work pauses; fix it with a bosun:kind/mainfix PR (spec §9)."
        );
        let mine = gh
            .create_issue("main is red", &body, &[MAIN_RED_LABEL])?
            .number;
        // A parallel run may have opened one too: the oldest issue stays, the newer closes.
        open = open_reds(gh)?;
        let keep = open.first().copied().unwrap_or(mine);
        if keep != mine {
            gh.add_comment(mine, &format!("duplicate of #{keep}"))?;
            close(gh, mine)?;
            red = Some(keep);
        } else {
            red = Some(mine);
            opened = true;
            let c = capture_lesson(
                gh,
                &app.ctx.session,
                "main-red",
                "main went red",
                "A run on the default branch failed; main-red opened.",
                Some(mine),
            )?;
            lesson = json!({"issue": c.issue, "created": c.created, "hits": c.hits});
        }
    } else if result == "success" {
        for n in open {
            gh.add_comment(n, &format!("main is green again: {url}"))?;
            close(gh, n)?;
            closed = true;
        }
        red = None;
    }
    let changed = opened || closed;
    let (mut rejudged, mut errors) = (0, Vec::new());
    if is_red || result == "success" {
        for pull in gh.list_pulls(None, IssueState::Open)? {
            match judge_pr(gh, &pull, red) {
                Ok(_) => rejudged += 1,
                Err(e) => errors.push(json!({"pr": pull.number, "error": e.message})),
            }
        }
    }
    let data = json!({
        "result": result, "changed": changed, "main_red": red, "opened": opened,
        "closed": closed, "lesson": lesson, "rejudged": rejudged, "rejudge_errors": errors,
        "note": "main-health reports health; it checks no kill switch and starts no work",
    });
    if !errors.is_empty() {
        return Err(BosunError::error(
            format!("{} PR(s) could not be re-judged", errors.len()),
            "rerun `bosun main-health` with the same --result; see data.rejudge_errors",
        )
        .with_data(data));
    }
    Ok(Outcome::data(data))
}

/// Numbers of the open main-red issues, oldest first.
fn open_reds(gh: &dyn Github) -> Result<Vec<u64>, BosunError> {
    let mut v: Vec<u64> = gh
        .list_issues(&[MAIN_RED_LABEL], IssueState::Open)?
        .iter()
        .map(|i| i.number)
        .collect();
    v.sort_unstable();
    Ok(v)
}

fn close(gh: &dyn Github, n: u64) -> Result<(), BosunError> {
    gh.update_issue(
        n,
        IssuePatch {
            state: Some(PatchState::Closed),
            state_reason: Some("completed".into()),
            ..Default::default()
        },
    )?;
    Ok(())
}
