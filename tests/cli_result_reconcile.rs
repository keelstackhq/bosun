//! `bosun result` and `bosun reap` move a task whose own PR merged to merged, even when GitHub
//! left the task issue open (the PR did not close it).

mod common;

use common::fake::FakeGithub;
use common::run;
use serde_json::json;

const NOW: &str = "2026-10-23T10:00:00Z";
const CHILD: &str = "session_child";

fn task(fake: &FakeGithub, n: u64, state: &str) {
    fake.issue(
        n,
        "task",
        &["bosun:source/plan", &format!("bosun:state/{state}")],
        &format!(
            r#"<!-- bosun:task {{"item":"M1","task":"T{n}","epic":9,"touch":["src/a.rs"]}} -->"#
        ),
    );
}

fn setup() -> FakeGithub {
    let fake = FakeGithub::start();
    fake.issue(9, "Epic", &["bosun:state/claimed"], "");
    fake
}

#[test]
fn result_moves_a_task_whose_marker_pr_merged() {
    let fake = setup();
    task(&fake, 10, "review");
    task(&fake, 11, "review");
    fake.pull(20, "t", "claude/t10", "<!-- bosun:task-pr 10 -->");
    fake.pull_merged(20, "abc");
    fake.pull(21, "t", "claude/t11", "<!-- bosun:task-pr 11 -->");
    let (code, v) = run(&fake, CHILD, NOW, &["result", "9", "--outcome", "merged"]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["tasks_merged"], json!([10]));
    assert!(fake.labels_of(10).contains(&"bosun:state/merged".into()));
    assert!(fake.labels_of(11).contains(&"bosun:state/review".into()));
}

#[test]
fn result_uses_the_task_pr_record_before_listing_pulls() {
    let fake = setup();
    task(&fake, 10, "building");
    fake.comment(
        10,
        r#"{"bosun":"task-pr","session":"session_child","pr":20}"#,
    );
    fake.pull(20, "t", "claude/t10", "no marker here");
    fake.pull_merged(20, "abc");
    let (code, v) = run(&fake, CHILD, NOW, &["result", "9", "--outcome", "partial"]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["tasks_merged"], json!([10]));
}

#[test]
fn reap_moves_an_open_review_task_with_a_merged_recorded_pr() {
    let fake = setup();
    task(&fake, 10, "review");
    task(&fake, 11, "review");
    fake.comment(
        10,
        r#"{"bosun":"task-pr","session":"session_child","pr":20}"#,
    );
    fake.pull(20, "t", "claude/t10", "x");
    fake.pull_merged(20, "abc");
    let (code, v) = run(&fake, "session_other", NOW, &["reap"]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["merged"], json!([10]));
    assert!(fake.labels_of(11).contains(&"bosun:state/review".into()));
}
