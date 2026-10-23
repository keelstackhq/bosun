//! Agreement between review mode a (a session) and mode b (`bosun review-ci`), UC-M8-3.

use crate::core::verdict::{Verdict, effective};

/// The pairs needed, and the share that must agree: 18 of 20.
pub const AGREE_MIN: (usize, usize) = (18, 20);

/// Approved after the rules: [`effective`], and no `escalate_risk` (it makes the status fail).
fn approved(v: &Verdict) -> bool {
    effective(v).approve && !v.escalate_risk
}

/// Two verdicts agree when their effective outcome is the same: both approved, or both not
/// approved (see [`approved`]). Different confidences, findings or `request-changes` versus a
/// low-confidence `approve` do not matter; only approved versus not approved does.
pub fn agrees(a: &Verdict, b: &Verdict) -> bool {
    approved(a) == approved(b)
}

#[derive(Debug, Clone, PartialEq)]
pub struct Agreement {
    pub total: usize,
    pub agree: usize,
    /// The PR numbers (of mode a's verdict) where the two disagree.
    pub disagree: Vec<u64>,
}

/// Compare `(a, b)` pairs.
pub fn agreement(pairs: &[(Verdict, Verdict)]) -> Agreement {
    let disagree: Vec<u64> = pairs
        .iter()
        .filter(|(a, b)| !agrees(a, b))
        .map(|(a, _)| a.pr)
        .collect();
    Agreement {
        total: pairs.len(),
        agree: pairs.len() - disagree.len(),
        disagree,
    }
}

/// At least [`AGREE_MIN`].1 pairs, and `agree * 20 >= 18 * total` (a rate of 90% or more).
/// Fewer pairs never pass.
pub fn passes(a: &Agreement) -> bool {
    a.total >= AGREE_MIN.1 && a.agree * AGREE_MIN.1 >= AGREE_MIN.0 * a.total
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(pr: u64, verdict: &str, confidence: f64, blocker: bool, escalate: bool) -> Verdict {
        let findings = if blocker {
            r#"[{"severity":"blocker","file":"a.rs","claim":"c","evidence":"e"}]"#
        } else {
            "[]"
        };
        Verdict::parse(&format!(
            r#"{{"pr":{pr},"head_sha":"s","verdict":"{verdict}","confidence":{confidence},
            "findings":{findings},"escalate_risk":{escalate}}}"#
        ))
        .unwrap()
    }

    #[test]
    fn same_outcome_agrees_whatever_the_reason() {
        let ok = v(1, "approve", 0.9, false, false);
        assert!(agrees(&ok, &v(1, "approve", 0.99, false, false)));
        let rc = v(1, "request-changes", 0.9, false, false);
        assert!(agrees(&rc, &v(1, "approve", 0.84, false, false)));
        assert!(agrees(&rc, &v(1, "approve", 0.95, true, false)));
        assert!(agrees(&rc, &v(1, "approve", 0.95, false, true)));
        assert!(!agrees(&ok, &rc));
        assert!(!agrees(&ok, &v(1, "approve", 0.84, false, false)));
    }

    fn pairs(n: usize, bad: usize) -> Vec<(Verdict, Verdict)> {
        (0..n)
            .map(|i| {
                let a = v(i as u64, "approve", 0.9, false, false);
                let b = if i < bad {
                    v(i as u64, "request-changes", 0.9, false, false)
                } else {
                    a.clone()
                };
                (a, b)
            })
            .collect()
    }

    #[test]
    fn agreement_counts_and_names_the_disagreeing_prs() {
        let a = agreement(&pairs(5, 2));
        assert_eq!((a.total, a.agree), (5, 3));
        assert_eq!(a.disagree, vec![0, 1]);
        assert_eq!(agreement(&[]).total, 0);
    }

    #[test]
    fn passes_needs_twenty_pairs_and_ninety_percent() {
        assert!(passes(&agreement(&pairs(20, 2))));
        assert!(!passes(&agreement(&pairs(20, 3))));
        assert!(!passes(&agreement(&pairs(10, 1))));
        assert!(!passes(&agreement(&pairs(19, 0))));
        assert!(!passes(&agreement(&[])));
        assert!(passes(&agreement(&pairs(40, 4))));
        assert!(!passes(&agreement(&pairs(40, 5))));
    }
}
