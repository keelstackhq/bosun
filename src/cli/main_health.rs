//! `bosun main-health --result R` (spec section 9): keeps one main-red issue open while main is
//! red. It does not check the kill switch: it reports health and starts no work.

use std::path::Path;

use serde_json::{Value, json};

use super::MainHealthArgs;
use super::lesson::capture_lesson;
use crate::app::{App, MAIN_RED_LABEL, notes, trusted_notes};
use crate::core::meta::linked_issues;
use crate::core::records::{Record, parse_record};
use crate::core::state::State;
use crate::gates::main_open::judge_pr;
use crate::github::{Github, Issue, IssuePatch, IssueState, PatchState};
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
    // Only on success (main moved): a red run adds no API calls. Housekeeping: a failure here
    // is reported in `conflict_errors` and never changes the exit code.
    let mut cf = Conflicts::default();
    if result == "success"
        && let Err(e) = flag_conflicts(gh, &mut cf)
    {
        cf.errors.push(json!({"error": e.message}));
    }
    let data = json!({
        "result": result, "changed": changed, "main_red": red, "opened": opened,
        "closed": closed, "lesson": lesson, "rejudged": rejudged, "rejudge_errors": errors,
        "conflicts": cf.flagged, "skipped_unknown": cf.skipped, "conflict_errors": cf.errors,
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

#[derive(Default)]
struct Conflicts {
    flagged: Vec<Value>,
    skipped: Vec<u64>,
    errors: Vec<Value>,
}

/// Queued tasks (auto-merge on) whose PR became `dirty` get one `conflict` record on the task
/// and one comment on the PR; the task stays `queued`, so auto-merge lands the PR once the
/// branch is fixed and `reap` syncs it to `merged`. The PR is the one in the newest `task-pr`
/// record of a writer (as `reap` reads it), never one found through a PR body. GitHub computes
/// `mergeable_state` lazily: `null` or `unknown` is skipped (listed in `skipped_unknown`), so a
/// conflict can show one run late.
fn flag_conflicts(gh: &dyn Github, out: &mut Conflicts) -> Result<(), BosunError> {
    let mut queued = gh.list_issues(&[&State::Queued.label()], IssueState::Open)?;
    queued.sort_by_key(|i| i.number);
    for issue in queued {
        if let Err(e) = flag_one(gh, &issue, out) {
            out.errors
                .push(json!({"issue": issue.number, "error": e.message}));
        }
    }
    Ok(())
}

fn flag_one(gh: &dyn Github, issue: &Issue, out: &mut Conflicts) -> Result<(), BosunError> {
    let n = issue.number;
    let Some(pr) = trusted_notes(gh, n)?
        .iter()
        .rev()
        .find_map(|c| match parse_record(&c.body) {
            Some(Record::TaskPr { pr, .. }) => Some(pr),
            _ => None,
        })
    else {
        return Ok(());
    };
    let Some(pull) = gh
        .get_pull(pr)?
        .filter(|p| p.state == "open" && linked_issues(&p.body).contains(&n))
    else {
        return Ok(());
    };
    match pull.mergeable_state.as_deref() {
        Some("dirty") => {}
        None | Some("unknown") => {
            out.skipped.push(pr);
            return Ok(());
        }
        Some(_) => return Ok(()),
    }
    // Plain notes, not trusted ones: bosun's own record is written by github-actions[bot], which
    // the permission check may not count as a writer. A forged record could only suppress the
    // comments, which is low impact.
    // The key is (pr, head sha): GitHub updates the base sha whenever main moves, so keying on it
    // would post again on every update of main. The base sha in the record is information only.
    let seen = notes(gh, n)?.iter().any(|c| {
        matches!(parse_record(&c.body), Some(Record::Conflict { pr: p, head_sha, .. })
            if p == pr && head_sha == pull.head_sha)
    });
    let fix = format!(
        "merge {} into {}, resolve, push; auto-merge stays on",
        pull.base_ref, pull.head_ref
    );
    if !seen {
        // The PR comment goes first and is deliberately at-least-once: if the record then fails,
        // the next run repeats the comment (a repeated wake is better than a lost one).
        let short = |s: &str| s.chars().take(7).collect::<String>();
        gh.add_comment(
            pr,
            &format!(
                "This PR conflicts with {} (head {}, base {}). {}.",
                pull.base_ref,
                short(&pull.head_sha),
                short(&pull.base_sha),
                capitalize(&fix)
            ),
        )?;
        let record = Record::Conflict {
            pr,
            head_sha: pull.head_sha.clone(),
            base_sha: pull.base_sha.clone(),
        };
        gh.add_comment(
            n,
            &format!(
                "PR #{pr} conflicts with {}.\n\n{}",
                pull.base_ref,
                record.to_line()
            ),
        )?;
    }
    out.flagged
        .push(json!({"pr": pr, "issue": n, "posted": !seen, "fix": fix}));
    Ok(())
}

fn capitalize(s: &str) -> String {
    let mut c = s.chars();
    c.next()
        .map(|f| f.to_uppercase().chain(c).collect())
        .unwrap_or_default()
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
