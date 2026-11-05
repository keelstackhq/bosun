//! `bosun result` and `bosun reap` move a task whose own PR merged to merged, even when GitHub
//! left the task issue open. The PR must close the task and the record must come from a writer.

mod common;

use common::fake::FakeGithub;
use common::run;
use serde_json::json;

const NOW: &str = "2026-10-23T10:00:00Z";
const CHILD: &str = "session_child";
const RECORD: &str = r#"{"bosun":"task-pr","session":"session_child","pr":20}"#;

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

fn is_review(fake: &FakeGithub, n: u64) -> bool {
    fake.labels_of(n).contains(&"bosun:state/review".into())
}

#[test]
fn result_moves_a_task_whose_closing_pr_merged_without_a_record() {
    let fake = setup();
    task(&fake, 10, "review");
    task(&fake, 11, "review");
    fake.pull(20, "t", "claude/t10", "Closes #10");
    fake.pull_merged(20, "abc");
    fake.pull(21, "t", "claude/t11", "Closes #11");
    let (code, v) = run(&fake, CHILD, NOW, &["result", "9", "--outcome", "merged"]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["tasks_merged"], json!([10]));
    assert!(fake.labels_of(10).contains(&"bosun:state/merged".into()));
    assert!(is_review(&fake, 11));
}

#[test]
fn a_marker_alone_is_not_a_link() {
    let fake = setup();
    task(&fake, 10, "review");
    fake.pull(20, "t", "claude/t10", "<!-- bosun:task-pr 10 -->");
    fake.pull_merged(20, "abc");
    let (_, v) = run(&fake, CHILD, NOW, &["result", "9", "--outcome", "merged"]);
    assert_eq!(v["data"]["tasks_merged"], json!([]), "{v}");
    assert!(is_review(&fake, 10));
}

#[test]
fn result_uses_the_task_pr_record_before_listing_pulls() {
    let fake = setup();
    task(&fake, 10, "building");
    fake.comment(10, RECORD);
    fake.pull(20, "t", "claude/t10", "Closes #10");
    fake.pull_merged(20, "abc");
    let (code, v) = run(&fake, CHILD, NOW, &["result", "9", "--outcome", "partial"]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["tasks_merged"], json!([10]));
}

#[test]
fn a_record_naming_a_pr_that_does_not_close_the_task_moves_nothing() {
    let fake = setup();
    task(&fake, 10, "review");
    fake.comment_by(10, "outsider", RECORD);
    fake.permission("outsider", "read");
    fake.pull(20, "t", "claude/other", "Closes #99");
    fake.pull_merged(20, "abc");
    let (_, v) = run(&fake, "session_other", NOW, &["reap"]);
    assert_eq!(v["data"]["merged"], json!([]), "{v}");
    let (_, v) = run(&fake, CHILD, NOW, &["result", "9", "--outcome", "partial"]);
    assert_eq!(v["data"]["tasks_merged"], json!([]), "{v}");
    assert!(is_review(&fake, 10));
}

#[test]
fn a_record_from_a_non_writer_is_ignored_even_when_the_pr_closes_the_task() {
    let fake = setup();
    task(&fake, 10, "review");
    fake.comment_by(10, "outsider", RECORD);
    fake.permission("outsider", "read");
    fake.pull(20, "t", "claude/t10", "Closes #10");
    fake.pull_merged(20, "abc");
    let (_, v) = run(&fake, "session_other", NOW, &["reap"]);
    assert_eq!(v["data"]["merged"], json!([]), "{v}");
}

#[test]
fn a_closing_pr_by_a_non_writer_does_not_count() {
    let fake = setup();
    task(&fake, 10, "review");
    fake.pull(20, "t", "claude/t10", "Closes #10");
    fake.pull_set(20, "user", json!({"login": "mallory"}));
    fake.permission("mallory", "read");
    fake.pull_merged(20, "abc");
    let (_, v) = run(&fake, CHILD, NOW, &["result", "9", "--outcome", "merged"]);
    assert_eq!(v["data"]["tasks_merged"], json!([]), "{v}");
}

#[test]
fn reap_moves_an_open_review_task_with_a_merged_recorded_pr() {
    let fake = setup();
    task(&fake, 10, "review");
    task(&fake, 11, "review");
    fake.comment(10, RECORD);
    fake.pull(20, "t", "claude/t10", "Closes #10");
    fake.pull_merged(20, "abc");
    let (code, v) = run(&fake, "session_other", NOW, &["reap"]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["merged"], json!([10]));
    assert!(is_review(&fake, 11));
}
