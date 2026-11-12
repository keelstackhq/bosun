//! `bosun metrics` logic (plan PR 5): CI facts of a head, the revert marker, p50, risk rules.

use super::approve::TRUSTED_APP;
use crate::time::Timestamp;

/// One check run, as far as the brake reads it.
pub struct CiRun {
    pub name: String,
    pub app: String,
    pub completed: bool,
    pub conclusion: Option<String>,
    pub at: Option<Timestamp>,
}

/// The `ci-ok` verdict of one commit: any failing run counts, a green rerun cannot erase it.
pub struct HeadCi {
    pub failing: bool,
    pub at: Option<Timestamp>,
}

/// Completed runs of the Actions app only; every run counts, so a green rerun cannot erase a
/// red one. A failed `ci-ok` is red unless `excuse` holds and it failed alone (no other job of
/// the commit failed): the caller excuses a head a newer push superseded (its run was
/// cancelled) and the head of a draft. `None` when the commit has no green and no red `ci-ok`.
pub fn ci_head(runs: &[CiRun], excuse: bool) -> Option<HeadCi> {
    let done = |r: &&CiRun| r.app == TRUSTED_APP && r.completed;
    let red = |r: &&CiRun| {
        matches!(
            r.conclusion.as_deref(),
            Some("failure" | "timed_out" | "startup_failure")
        )
    };
    let (ok, jobs): (Vec<&CiRun>, Vec<&CiRun>) =
        runs.iter().filter(done).partition(|r| r.name == "ci-ok");
    let failing = ok.iter().any(red) && !(excuse && !jobs.iter().any(red));
    let green = ok
        .iter()
        .any(|r| r.conclusion.as_deref() == Some("success"));
    (failing || green).then(|| HeadCi {
        failing,
        at: ok.iter().filter_map(|r| r.at).max(),
    })
}

/// N of the `<!-- bosun:revert N -->` marker `bosun revert` writes.
pub fn revert_of(body: &str) -> Option<u64> {
    let rest = body.split("<!-- bosun:revert ").nth(1)?;
    rest.split_once(" -->")?.0.parse().ok()
}

/// Median; sorts `v`.
pub fn p50(v: &mut [f64]) -> Option<f64> {
    v.sort_by(f64::total_cmp);
    let n = v.len();
    (n > 0).then(|| (v[(n - 1) / 2] + v[n / 2]) / 2.0)
}

/// Rule ids of the Risk note when it asks for an owner approval.
pub fn owner_rules(body: &str) -> Vec<String> {
    let (Some(a), Some(b)) = (
        body.find("<!-- bosun:risk -->"),
        body.find("<!-- /bosun:risk -->"),
    ) else {
        return Vec::new();
    };
    let note = body.get(a..b).unwrap_or_default();
    if !note.lines().any(|l| l.starts_with("Owner: review")) {
        return Vec::new();
    }
    let rule = |l: &str| Some(l.strip_prefix("- **")?.split_once("**:")?.0.to_string());
    note.lines().filter_map(rule).collect()
}
