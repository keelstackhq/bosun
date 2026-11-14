//! risk-check takes the linked task's `risk: high` from the plan (D-016), not only the paths.

mod common;

use common::fake::FakeGithub;
use common::run;

const NOW: &str = "2026-10-22T10:00:00Z";

fn pr(task_meta: &str) -> FakeGithub {
    let fake = FakeGithub::start();
    fake.pull(1, "change", "feature", "Closes #7");
    fake.issue(
        7,
        "task",
        &[],
        &format!("Do it.\n<!-- bosun:task {task_meta} -->"),
    );
    fake.pull_file(1, "docs/note.md", "modified", 1, 0, Some("@@ -1 +1 @@"));
    fake
}

#[test]
fn a_high_risk_task_makes_a_docs_only_pr_high() {
    let fake = pr(r#"{"risk":"high"}"#);
    let (code, v) = run(&fake, "session_test", NOW, &["risk-check", "1"]);
    assert_eq!(code, 5, "{v}");
    assert_eq!(v["data"]["level"], "high");
    assert!(v["data"]["matches"].to_string().contains("R-plan"), "{v}");
}

#[test]
fn a_normal_task_stays_normal() {
    let fake = pr("{}");
    let (code, v) = run(&fake, "session_test", NOW, &["risk-check", "1"]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["level"], "normal");
}
