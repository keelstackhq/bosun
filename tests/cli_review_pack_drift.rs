//! D-016: the review pack compares the PR's changed lines with the planned `size:` of the tasks
//! it links; more than twice the plan is a drift note, never an error.

mod common;

use common::fake::FakeGithub;
use common::run;
use serde_json::{Value, json};

const T0: &str = "2026-10-22T10:00:00Z";

fn task(fake: &FakeGithub, n: u64, size: &str) {
    let body = format!(
        r#"Do it.<!-- bosun:task {{"item":"M1","task":"T{n}","touch":["src/**"]{size}}} -->"#
    );
    fake.issue(n, "the task", &["bosun:state/review"], &body);
}

fn pack(fake: &FakeGithub) -> Value {
    let (code, v) = run(fake, "s", T0, &["review-pack", "42"]);
    assert_eq!(code, 0, "{v}");
    v
}

fn setup(body: &str) -> FakeGithub {
    let fake = FakeGithub::start();
    fake.pull_as_issue(42, "the pr");
    fake.pull(42, "the pr", "claude/x", body);
    fake
}

#[test]
fn more_than_twice_the_plan_is_a_drift_note() {
    let fake = setup("Closes #7, closes #8");
    task(&fake, 7, r#","size":60"#);
    task(&fake, 8, r#","size":40"#);
    fake.pull_file(42, "src/a.rs", "modified", 150, 50, Some("@@ a"));
    // Tests and lockfiles do not count.
    fake.pull_file(42, "tests/a.rs", "added", 900, 0, Some("@@ t"));
    fake.pull_file(42, "crates/x/Cargo.lock", "modified", 500, 100, None);
    fake.pull_file(42, "pnpm-lock.yaml", "modified", 500, 100, None);
    fake.pull_file(42, "src/b.rs", "modified", 1, 0, Some("@@ b"));
    let v = pack(&fake);
    assert_eq!(v["data"]["drift"], json!({"planned": 100, "actual": 201}));
    let note = v["data"]["note"].as_str().unwrap();
    assert_eq!(note, "re-plan? PR #42 is 201 changed lines, planned 100");
}

#[test]
fn within_twice_the_plan_or_unsized_is_no_drift() {
    let fake = setup("Closes #7");
    task(&fake, 7, r#","size":100"#);
    fake.pull_file(42, "src/a.rs", "modified", 150, 50, Some("@@ a"));
    let v = pack(&fake);
    assert_eq!(v["data"]["drift"], Value::Null);
    assert!(v["data"].get("note").is_none(), "{v}");
    // No linked task has a size.
    let fake = setup("Closes #7");
    task(&fake, 7, "");
    fake.pull_file(42, "src/a.rs", "modified", 5000, 0, Some("@@ a"));
    assert_eq!(pack(&fake)["data"]["drift"], Value::Null);
}
