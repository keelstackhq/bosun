//! `bosun reap` reads the `task-pr` record from repo writers only.
mod common;

use common::fake::FakeGithub;

const NOW: &str = "2026-10-27T10:00:00Z";

#[test]
fn a_readers_task_pr_record_does_not_make_the_task_merged() {
    let fake = FakeGithub::start();
    fake.issue(
        1,
        "C1",
        &["bosun:source/plan", "bosun:state/ready"],
        r#"C1 <!-- bosun:epic {"item":"C"} -->"#,
    );
    fake.issue(
        10,
        "task",
        &["bosun:source/plan", "bosun:state/queued"],
        r#"<!-- bosun:task {"item":"C","task":"T","epic":1} -->"#,
    );
    // The writer's record names the open PR 30; the reader's newer one names merged PR 31.
    fake.comment(10, r#"{"bosun":"task-pr","session":"s","pr":30}"#);
    fake.permission("mallory", "read");
    fake.comment_by(
        10,
        "mallory",
        r#"{"bosun":"task-pr","session":"s","pr":31}"#,
    );
    fake.pull(30, "t", "claude/t10", "Closes #10");
    fake.pull(31, "t", "claude/other", "Closes #10");
    fake.pull_merged(31, "abc");
    fake.issue_updated(10, NOW);
    let (code, v) = common::run(&fake, "janitor", NOW, &["reap"]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["merged"], serde_json::json!([]), "{v}");
}
