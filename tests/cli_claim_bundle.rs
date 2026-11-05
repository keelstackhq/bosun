//! D-016: while the lead holds a bundle, the bundled issues of a single epic are not claimable.

mod common;

use common::fake::FakeGithub;
use common::run;
use serde_json::Value;

const PLAN: &str = "# Plan: X\n\nproject: x\n\n## M1: One\n\narea: core\ndepends: seed\n\nWords.\n\n\
### Tasks\n- T1: a\n  touch: src/a/**\n- T2: b\n  touch: src/b/**\n### Done when\n- UC-M1-1: works\n";
const NOW: &str = "2026-10-22T10:00:00Z";

fn task(fake: &FakeGithub, n: u64, state: &str, touch: &str, extra: &str) {
    let body = format!(
        r#"<!-- bosun:task {{"item":"M1","task":"T{}","epic":10,"touch":["{touch}"]{extra}}} -->"#,
        n - 10
    );
    fake.issue(
        n,
        &format!("Task {n}"),
        &[&format!("bosun:state/{state}")],
        &body,
    );
}

fn epic(shape: &str) -> FakeGithub {
    let fake = FakeGithub::start();
    let body =
        format!(r#"Epic<!-- bosun:epic {{"item":"M1","plan":"plans/x.md","shape":"{shape}"}} -->"#);
    fake.issue(1, "bosun registry", &["bosun:registry"], "");
    fake.issue(10, "Epic", &["bosun:state/claimed"], &body);
    fake.content("plans/x.md", "main", PLAN);
    fake
}

fn claim(fake: &FakeGithub, n: u64) -> (i32, Value) {
    run(fake, "session_b", NOW, &["claim", &n.to_string()])
}

#[test]
fn a_bundled_issue_is_not_claimable_while_the_lead_is_active() {
    let fake = epic("single");
    task(&fake, 11, "building", "src/a/**", "");
    task(&fake, 12, "ready", "src/b/**", "");
    let (code, v) = claim(&fake, 12);
    assert_eq!(code, 3, "{v}");
    assert_eq!(v["error"]["message"], "#12 is bundled into #11's PR");
    assert_eq!(v["error"]["fix"], "work in #11's bundle: its PR closes #12");
}

#[test]
fn gated_split_and_lone_tasks_stay_claimable() {
    // The claimed task is gated.
    let fake = epic("single");
    task(&fake, 11, "building", "src/a/**", "");
    task(&fake, 12, "ready", "src/b/**", r#","gated":true"#);
    assert_eq!(claim(&fake, 12).0, 0);
    // The active task is gated: it is not a bundle lead.
    let fake = epic("single");
    task(&fake, 11, "building", "src/a/**", r#","risk":"high""#);
    task(&fake, 12, "ready", "src/b/**", "");
    assert_eq!(claim(&fake, 12).0, 0);
    // A split epic.
    let fake = epic("split");
    task(&fake, 11, "building", "src/a/**", "");
    task(&fake, 12, "ready", "src/b/**", "");
    assert_eq!(claim(&fake, 12).0, 0);
    // Nothing active in the epic.
    let fake = epic("single");
    task(&fake, 11, "ready", "src/a/**", "");
    task(&fake, 12, "ready", "src/b/**", "");
    assert_eq!(claim(&fake, 11).0, 0);
}

#[test]
fn a_gated_touch_cannot_grow_into_a_bundled_task() {
    let gated = ".github/workflows/x.yml";
    let want = format!(
        "--touch {gated} is a gated path: gated work runs as its own task, not inside the bundle"
    );
    // A first claim with a gated --touch.
    let fake = epic("single");
    task(&fake, 11, "ready", "src/a.rs", "");
    let (code, v) = run(&fake, "session_b", NOW, &["claim", "11", "--touch", gated]);
    assert_eq!(code, 3, "{v}");
    assert_eq!(v["error"]["message"], want.as_str());
    assert_eq!(
        v["error"]["fix"],
        "drop the gated path and plan it as its own task (plan amend)"
    );
    // Growing a live lease with it.
    let fake = epic("single");
    task(&fake, 11, "ready", "src/a.rs", "");
    assert_eq!(claim(&fake, 11).0, 0);
    let (code, v) = run(&fake, "session_b", NOW, &["claim", "11", "--touch", gated]);
    assert_eq!(code, 3, "{v}");
    assert_eq!(v["error"]["message"], want.as_str());
    // A split epic, a gated task, and a plain glob are fine.
    let fake = epic("split");
    task(&fake, 11, "ready", "src/a.rs", "");
    assert_eq!(
        run(&fake, "session_b", NOW, &["claim", "11", "--touch", gated]).0,
        0
    );
    let fake = epic("single");
    task(&fake, 11, "ready", "src/a.rs", r#","gated":true"#);
    assert_eq!(
        run(&fake, "session_b", NOW, &["claim", "11", "--touch", gated]).0,
        0
    );
    let fake = epic("single");
    task(&fake, 11, "ready", "src/a.rs", "");
    assert_eq!(
        run(
            &fake,
            "session_b",
            NOW,
            &["claim", "11", "--touch", "src/b.rs"]
        )
        .0,
        0
    );
}
