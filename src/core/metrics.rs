//! `bosun metrics` logic (plan PR 5, 8): CI facts of a head, the revert marker, p50, risk rules,
//! curator proposals.

use super::approve::{Stat, TRUSTED_APP, TRUSTED_CREATOR, WAITING_PREFIX, risk_ids};
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

/// Rules the curator never proposes to loosen (modes spec, metrics and curator): the floor,
/// workflow paths (R-ci), the CI bosun ships to other repos (R-ci-review) and the supply chain
/// (R-deps).
pub const NEVER_PROPOSED: [&str; 10] = [
    "R-weaken",
    "R-secrets",
    "R-prod-write",
    "R-gov",
    "R-gate",
    "R-revert",
    "R-ci",
    "R-ci-review",
    "R-deps",
    "R-unseen",
];

/// Owner blocks of one rule in the window: the PRs merged on their first head (passed), and
/// how many PRs were caught (closed unmerged or re-pushed).
pub struct RuleBlocks {
    pub rule: String,
    pub passed: Vec<u64>,
    pub caught: u32,
}

/// The rule ids of every trusted risk-check post on a head that asked for an owner, deduped.
pub fn waiting_rules(history: &[Stat]) -> Vec<String> {
    let mut ids: Vec<String> = history
        .iter()
        .filter(|s| s.context == "risk-check" && s.creator == TRUSTED_CREATOR)
        .filter(|s| s.state == "failure" && s.description.starts_with(WAITING_PREFIX))
        .flat_map(|s| risk_ids(&s.description))
        .collect();
    ids.sort();
    ids.dedup();
    ids
}

/// A rule the curator may propose to loosen, with a draft PR title and body.
pub struct Proposal {
    pub rule: String,
    pub blocks: u32,
    pub prs: Vec<u64>,
    pub title: String,
    pub body: String,
}

/// Rules of `defined` (the ids of the base `.bosun/risk.toml`) with at least 10 passed blocks
/// and no catch in `days`, never [`NEVER_PROPOSED`] (ids compared ignoring case).
pub fn proposals(all: &[RuleBlocks], defined: &[String], days: u32) -> Vec<Proposal> {
    let is = |a: &str, b: &str| a.eq_ignore_ascii_case(b);
    let ok = |b: &&RuleBlocks| {
        b.passed.len() >= 10
            && b.caught == 0
            && defined.iter().any(|d| d == &b.rule)
            && !NEVER_PROPOSED.iter().any(|f| is(f, &b.rule))
    };
    all.iter()
        .filter(ok)
        .map(|b| {
            let n = b.passed.len() as u32;
            let prs: String = b.passed.iter().map(|p| format!("- #{p}\n")).collect();
            Proposal {
                rule: b.rule.clone(),
                blocks: n,
                prs: b.passed.clone(),
                title: format!("risk: loosen {} (no catch in {days} days)", b.rule),
                body: format!(
                    "{rule} asked for an owner approval {n} times: {n} owner blocks in {days} \
                     days, each merged on its first head, none caught (no PR re-pushed or \
                     closed). Rule ids come from the trusted risk-check posts.\n\n\
                     Blocked PRs:\n{prs}\nThis PR loosens {rule} in `.bosun/risk.toml`. It \
                     matches R-weaken and R-gov: the owner decides.\n",
                    rule = b.rule
                ),
            }
        })
        .collect()
}
