use bosun::core::plan;
use bosun::core::prereqs::{self as pq, ApprovalNeed, CheckItem};

const PLAN: &str = "# Plan: x\n\nproject: x\n\n\
## M1: One\n\narea: core\ndepends: seed\n\nWords.\n\n### Tasks\n\
- T1: a\n  touch: docs/a.md\n- T2: b\n  touch: src/x.rs\n  depends: T1\n- T3: c\n  touch: docs/a.md\n\
### Done when\n- UC-M1-1: works\n\n\
## M2: Two\n\narea: core\ndepends: M1\n\n### Tasks\n- T1: d\n  touch: README.md\n  risk: high\n\
### Done when\n- UC-M2-1: works\n";

const SECTION: &str = "\n## Prerequisites\n\nProse is ignored.\n\n\
- H1: npm token\n  on: phone\n  steps: do it\n  verify: secret release/NPM_TOKEN\n  blocks: M1/T2, M2, M2\n\
- H2: pc thing\n  who: bob\n  on: computer\n  steps: run it\n  blocks: M1\n";

fn plan_of(extra: &str) -> (String, plan::Plan) {
    let t = format!("{PLAN}{extra}");
    let p = plan::parse(&t).expect("plan parses");
    (t, p)
}

/// What `approvals` gives for PLAN (Q1c): two delegate tasks in batch 1, two owner tasks.
fn approvals() -> Vec<ApprovalNeed> {
    let a = [
        ("M1/T1", "delegate", 1),
        ("M1/T2", "owner", 2),
        ("M1/T3", "delegate", 1),
    ];
    let a = a.into_iter().chain([("M2/T1", "owner", 3)]);
    let need = |(t, n, batch): (&str, &str, usize)| (t.to_string(), n.to_string(), batch);
    a.map(need)
        .map(|(task, needs, batch)| ApprovalNeed {
            task,
            needs,
            why: vec![],
            batch,
        })
        .collect()
}

fn items() -> Vec<CheckItem> {
    let (t, p) = plan_of(SECTION);
    pq::parse(&t, &p)
        .unwrap()
        .iter()
        .map(CheckItem::from)
        .collect()
}

#[test]
fn message_is_short_and_capped() {
    let it = items();
    let m = pq::message("Plan x", &it, &[], Some("https://x/1"));
    let lines: Vec<_> = m.lines().collect();
    assert_eq!(
        lines[0],
        "Plan x: 2 steps before agents can finish (1 on a computer)."
    );
    assert!(lines[1].starts_with("- H2 (computer) (by bob): pc thing: run it"));
    assert!(lines[2].starts_with("- H1 (phone)"));
    assert_eq!(lines[3], "Tick them off here: https://x/1");
    let done: Vec<_> = it
        .iter()
        .cloned()
        .map(|mut i| {
            i.done = true;
            i
        })
        .collect();
    assert_eq!(
        pq::message("Plan x", &done, &[], None),
        "Plan x: nothing for you to prepare."
    );
    let many: Vec<_> = (0..20)
        .map(|n| CheckItem {
            id: format!("H{n}"),
            ..it[0].clone()
        })
        .collect();
    let m = pq::message("P", &many, &approvals(), None);
    assert_eq!(m.lines().count(), 1 + 15 + 1 + 1);
    assert!(m.contains("- \u{2026} and 5 more (see the issue)"));
    assert!(
        m.contains("Later: 3 approval batches (2 PRs need you: M1/T2, M2/T1; 2 by a delegate).")
    );
    let one = &approvals()[..1];
    assert!(
        pq::message("P", &[], one, None).ends_with("Later: 1 approval batch (1 by a delegate).")
    );
}

#[test]
fn long_steps_are_cut_in_the_message() {
    let mut it = items();
    it[0].steps = "x".repeat(200);
    let m = pq::message("P", &it, &[], None);
    assert!(m.lines().all(|l| l.chars().count() < 160), "{m}");
}
