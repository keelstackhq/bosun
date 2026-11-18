use bosun::core::review_loop::{Seen, Step, plan};
use bosun::core::verdict::Verdict;

fn seen(head: &str, kind: &str, bot: bool, escalate: bool) -> Seen {
    Seen {
        bot,
        verdict: Verdict {
            pr: 1,
            head_sha: head.into(),
            verdict: kind.into(),
            confidence: 0.95,
            findings: vec![],
            lessons_hit: vec![],
            escalate_risk: escalate,
            chunks_reviewed: vec![],
        },
    }
}

#[test]
fn steps_and_rounds() {
    assert_eq!(plan("h", &[], 3).step, Step::SpawnReviewer);
    let old = [
        seen("a", "request-changes", false, false),
        seen("b", "approve", false, false),
    ];
    let l = plan("h", &old, 3);
    assert_eq!((l.step, l.rounds), (Step::SpawnReviewer, 2));
    assert!(matches!(plan("h", &old, 2).step, Step::AskOwner(_)));
    assert_eq!(plan("b", &old, 3).step, Step::Ready);
    assert_eq!(plan("a", &old, 3).step, Step::Fix);
    assert!(matches!(plan("a", &old, 2).step, Step::AskOwner(_)));
}

#[test]
fn bots_are_neither_rounds_nor_ready_and_escalation_asks() {
    let b = [seen("h", "approve", true, true)];
    let l = plan("h", &b, 3);
    assert_eq!((l.step, l.rounds), (Step::SpawnReviewer, 0));
    assert!(matches!(
        plan("h", &[seen("h", "approve", false, true)], 3).step,
        Step::AskOwner(_)
    ));
}
