//! The review loop of one PR (spec §7): rounds counted from verdict comments, and the one next step.

use super::verdict::{Finding, Verdict, effective};

/// A verdict comment as read from the PR; `bot` verdicts (a `verdict_bots` login or a GitHub
/// Bot account) are never independent, so they neither count as a round nor make a PR ready.
#[derive(Debug, Clone, PartialEq)]
pub struct Seen {
    pub bot: bool,
    pub verdict: Verdict,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Step {
    SpawnReviewer,
    Fix,
    Ready,
    /// Escalated, or the round limit is reached: the text says which.
    AskOwner(String),
}

impl Step {
    pub fn name(&self) -> &'static str {
        match self {
            Step::SpawnReviewer => "spawn-reviewer",
            Step::Fix => "fix",
            Step::Ready => "ready",
            Step::AskOwner(_) => "ask-owner",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Loop {
    pub step: Step,
    /// Distinct heads an independent reviewer has judged.
    pub rounds: u32,
    /// The findings of the verdicts at the head that do not approve (for `fix`).
    pub findings: Vec<Finding>,
    /// Why those verdicts do not approve (a low confidence has no finding).
    pub reasons: Vec<String>,
}

/// `seen` is every verdict comment of the PR; a request-changes at the head beats an approve.
pub fn plan(head: &str, seen: &[Seen], max: u32) -> Loop {
    let own: Vec<&Verdict> = seen.iter().filter(|s| !s.bot).map(|s| &s.verdict).collect();
    let mut heads: Vec<&str> = own.iter().map(|v| v.head_sha.as_str()).collect();
    heads.sort_unstable();
    heads.dedup();
    let rounds = u32::try_from(heads.len()).unwrap_or(u32::MAX);
    let at: Vec<&Verdict> = own.iter().copied().filter(|v| v.head_sha == head).collect();
    let limit = || {
        Step::AskOwner(format!(
            "round limit {max} reached ({rounds} rounds done); the owner must answer before round {}",
            max + 1
        ))
    };
    let (step, findings, reasons) = if at.iter().any(|v| v.escalate_risk) {
        (
            Step::AskOwner("the review escalated to the owner".into()),
            vec![],
            vec![],
        )
    } else if at.iter().any(|v| !effective(v).approve) {
        let bad: Vec<&&Verdict> = at.iter().filter(|v| !effective(v).approve).collect();
        let f = bad.iter().flat_map(|v| v.findings.clone()).collect();
        let r = bad.iter().flat_map(|v| effective(v).reasons).collect();
        (if rounds >= max { limit() } else { Step::Fix }, f, r)
    } else if !at.is_empty() {
        (Step::Ready, vec![], vec![])
    } else if rounds >= max {
        (limit(), vec![], vec![])
    } else {
        (Step::SpawnReviewer, vec![], vec![])
    };
    Loop {
        step,
        rounds,
        findings,
        reasons,
    }
}
