//! `bosun gate sources --pr N` (spec §15): who posted each gate on the PR head. A status names no
//! App (anyone with write can post one); a check run of the gate App (`[checks] app_id`) can only
//! come from the App's token. Read-only: it posts nothing.

use std::path::Path;

use serde_json::{Value, json};

use crate::app::App;
use crate::config::CheckMode;
use crate::github::gate::as_status;
use crate::output::{BosunError, CmdResult, Outcome};

pub const CONTEXTS: [&str; 3] = ["risk-check", "review-verdict", "main-open"];

/// What one context has on the head.
#[derive(Debug, Default)]
pub struct Seen {
    /// The newest status: `(state, creator)`.
    pub status: Option<(String, String)>,
    /// The newest run of the gate App: `(state, id)`.
    pub app_run: Option<(String, u64)>,
}

#[derive(Debug, PartialEq, Eq)]
pub struct Judged {
    pub state: Option<String>,
    pub trusted: bool,
    pub disagree: bool,
}

fn rank(state: &str) -> u8 {
    match state {
        "success" => 0,
        "pending" => 1,
        "failure" => 2,
        _ => 3,
    }
}

/// `checks`: the App run only; `dual`: the worse of status and App run; `statuses`: the status,
/// never trusted. Trusted = the App has posted (not in `statuses`). Disagree = both differ.
pub fn judge(mode: CheckMode, seen: &Seen) -> Judged {
    let status = seen.status.as_ref().map(|(s, _)| s.clone());
    let run = seen.app_run.as_ref().map(|(s, _)| s.clone());
    let state = match mode {
        CheckMode::Statuses => status.clone(),
        CheckMode::Checks => run.clone(),
        CheckMode::Dual => match (&status, &run) {
            (Some(s), Some(r)) => Some(if rank(s) >= rank(r) { s } else { r }.clone()),
            (s, r) => s.clone().or(r.clone()),
        },
    };
    Judged {
        state,
        trusted: mode != CheckMode::Statuses && run.is_some(),
        disagree: matches!((&status, &run), (Some(s), Some(r)) if s != r),
    }
}

pub fn run(dir: Option<&Path>, pr: Option<u64>) -> CmdResult {
    let n = pr
        .or_else(|| (std::env::var("BOSUN_PR").ok()).and_then(|v| v.trim().parse().ok()))
        .ok_or_else(|| {
            BosunError::usage("sources needs a PR number", "pass --pr N (or set BOSUN_PR)")
        })?;
    let app = App::load(dir)?;
    let gh = app.gh.as_ref();
    let checks = &app.ctx.config.checks;
    let pull = gh.get_pull(n)?.ok_or_else(|| {
        BosunError::error(
            format!("PR #{n} does not exist"),
            "check the number; `gh pr list` lists the open PRs",
        )
    })?;
    let head = &pull.head_sha;
    let statuses = gh.statuses(head)?;
    let runs = gh.check_runs_all(head)?;
    let mut gates = Vec::new();
    let mut bad = Vec::new();
    for context in CONTEXTS {
        let status = statuses.iter().find(|s| s.context == context);
        let named = || runs.iter().filter(|r| r.name == context);
        let mine = named()
            .filter(|r| Some(r.app_id) == checks.app_id)
            .max_by_key(|r| r.id);
        let other_runs = named().filter(|r| Some(r.app_id) != checks.app_id).count();
        let seen = Seen {
            status: status.map(|s| (s.state.clone(), s.creator.clone())),
            app_run: mine.map(|r| (as_status(r.clone()).state, r.id)),
        };
        let j = judge(checks.mode, &seen);
        // In `checks` mode a status counts for nothing, so one that disagrees is only reported.
        if !j.trusted || (j.disagree && checks.mode == CheckMode::Dual) {
            bad.push(context);
        }
        let status = (seen.status.as_ref()).map(|(s, c)| json!({"state": s, "creator": c}));
        let app_run = (seen.app_run.as_ref()).map(|(s, id)| json!({"state": s, "id": id}));
        gates.push(
            json!({"context": context, "status": status, "app_run": app_run,
            "other_runs": other_runs, "state": j.state, "trusted": j.trusted,
            "disagree": j.disagree}),
        );
    }
    let data = json!({"pr": n, "head_sha": head, "mode": checks.mode.as_str(),
        "app_id": checks.app_id, "gates": Value::Array(gates), "verified": bad.is_empty()});
    if bad.is_empty() {
        return Ok(Outcome::data(data));
    }
    let fix = if checks.mode == CheckMode::Statuses {
        "set [checks] mode = \"dual\" with the gate App's app_id and app_slug (spec §15)"
    } else {
        "a status disagrees with the gate App or the App has not posted: re-run the gate workflow (risk: comment `/bosun recheck`) and treat the status as unverified"
    };
    Err(BosunError::conflict(
        format!("gates not verified on {head}: {}", bad.join(", ")),
        fix,
    )
    .with_data(data))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seen(status: Option<&str>, run: Option<&str>) -> Seen {
        Seen {
            status: status.map(|s| (s.into(), "u".into())),
            app_run: run.map(|r| (r.into(), 1)),
        }
    }

    #[test]
    fn dual_takes_the_worse_and_flags_disagreement() {
        let j = judge(CheckMode::Dual, &seen(Some("success"), Some("failure")));
        assert_eq!(j.state.as_deref(), Some("failure"));
        assert!(j.trusted && j.disagree);
        let j = judge(CheckMode::Dual, &seen(Some("error"), Some("pending")));
        assert_eq!(j.state.as_deref(), Some("error"));
        let j = judge(CheckMode::Dual, &seen(Some("success"), None));
        assert_eq!(j.state.as_deref(), Some("success"));
        assert!(!j.trusted && !j.disagree);
    }

    #[test]
    fn checks_reads_the_run_and_statuses_trust_nothing() {
        let j = judge(CheckMode::Checks, &seen(Some("failure"), Some("success")));
        assert_eq!(j.state.as_deref(), Some("success"));
        assert!(j.trusted && j.disagree);
        let j = judge(CheckMode::Checks, &seen(Some("success"), None));
        assert_eq!(j.state, None);
        assert!(!j.trusted);
        let j = judge(CheckMode::Statuses, &seen(Some("success"), Some("success")));
        assert_eq!(j.state.as_deref(), Some("success"));
        assert!(!j.trusted && !j.disagree);
    }
}
