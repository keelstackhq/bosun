//! `bosun progress` held outcomes: only writers' task-pr records count, and a revert is found
//! by its own branch in this repo, not by scanning every closed PR.
mod common;

use common::fake::FakeGithub;
use common::run;
use serde_json::json;

const NOW: &str = "2026-10-23T12:00:00Z";
const MERGED: &str = "bosun:state/merged";

fn merged_pull(fake: &FakeGithub, n: u64, head: &str) {
    fake.pull(n, &format!("PR {n}"), head, "");
    fake.pull_set(n, "state", json!("closed"));
    fake.pull_set(n, "merged", json!(true));
}

fn task_10(fake: &FakeGithub) {
    fake.issue(1, "bosun registry", &["bosun:registry"], "");
    let body = r#"<!-- bosun:task {"item":"M4","task":"T10"} -->"#;
    fake.issue(10, "Task 10", &[MERGED], body);
}

#[test]
fn a_task_pr_record_from_a_reader_does_not_count() {
    let fake = FakeGithub::start();
    task_10(&fake);
    fake.permission("mallory", "read");
    // A reader points the task at PR 30, which a merged revert undid: no effect.
    fake.comment_by(
        10,
        "mallory",
        r#"{"bosun":"task-pr","session":"s","pr":30}"#,
    );
    fake.close_issue(10, "2026-10-22T09:00:00Z");
    merged_pull(&fake, 30, "feat/a");
    merged_pull(&fake, 40, "bosun/revert-30");
    let (code, v) = run(&fake, "session_a", NOW, &["progress"]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["reverted"], 0, "{v}");
    assert_eq!(v["data"]["held"], 1, "{v}");
}

#[test]
fn a_fork_branch_named_like_a_revert_does_not_count() {
    let fake = FakeGithub::start();
    task_10(&fake);
    fake.comment(10, r#"{"bosun":"task-pr","session":"s","pr":30}"#);
    fake.close_issue(10, "2026-10-22T09:00:00Z");
    merged_pull(&fake, 30, "feat/a");
    merged_pull(&fake, 40, "bosun/revert-30");
    fake.pull_set(
        40,
        "head",
        json!({"sha": "f", "ref": "bosun/revert-30", "label": "fork:bosun/revert-30"}),
    );
    let (code, v) = run(&fake, "session_a", NOW, &["progress"]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["reverted"], 0, "{v}");
}
