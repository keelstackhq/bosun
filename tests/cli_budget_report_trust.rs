//! `bosun budget report`: models come only from writers' records, one row per PR, and a task
//! whose PR is unknown or unreadable is listed as skipped.
mod common;

use common::fake::FakeGithub;
use common::run;
use serde_json::json;

const NOW: &str = "2026-11-09T12:00:00Z";
const REVIEW: &str = "bosun:state/review";

fn task(fake: &FakeGithub, n: u64, usd: f64) {
    let body = format!(r#"<!-- bosun:task {{"item":"M4","task":"T{n}"}} -->"#);
    fake.issue(n, &format!("Task {n}"), &[REVIEW], &body);
    fake.comment(
        n,
        &format!(r#"{{"bosun":"spend","session":"s{n}","usd":{usd},"at":"2026-11-02T09:00:00Z"}}"#),
    );
}

fn task_pr(fake: &FakeGithub, n: u64, pr: u64) {
    fake.comment(
        n,
        &format!(r#"{{"bosun":"task-pr","session":"s","pr":{pr}}}"#),
    );
}

fn verdict(model: &str) -> String {
    format!(
        r#"{{"bosun":"verdict","session":"r","head_sha":"sha20","approved":true,"model":"{model}"}}"#
    )
}

fn open_pr(fake: &FakeGithub, pr: u64) {
    fake.pull(pr, &format!("PR {pr}"), &format!("feat/{pr}"), "");
    fake.pull_set(pr, "updated_at", json!("2026-11-05T10:00:00Z"));
}

fn rows(v: &serde_json::Value) -> Vec<serde_json::Value> {
    v["data"]["rows"].as_array().unwrap().clone()
}

#[test]
fn models_come_only_from_writers() {
    let fake = FakeGithub::start();
    fake.issue(1, "bosun registry", &["bosun:registry"], "");
    fake.permission("mallory", "read");
    task(&fake, 10, 2.0);
    task_pr(&fake, 10, 20);
    open_pr(&fake, 20);
    fake.comment(20, &verdict("opus"));
    fake.comment_by(10, "mallory", &verdict("evil-task"));
    fake.comment_by(20, "mallory", &verdict("evil-pr"));
    fake.comment_by(
        10,
        "mallory",
        r#"{"bosun":"escalate","session":"s","from":"a","to":"evil-up","reason":"r"}"#,
    );
    let (code, v) = run(&fake, "session_a", NOW, &["budget", "report"]);
    assert_eq!(code, 0, "{v}");
    let rows = rows(&v);
    assert_eq!(rows.len(), 1, "{v}");
    assert_eq!(rows[0]["models"], json!(["opus"]), "{v}");
}

#[test]
fn two_tasks_on_one_pr_give_one_row_for_the_lowest_task() {
    let fake = FakeGithub::start();
    fake.issue(1, "bosun registry", &["bosun:registry"], "");
    task(&fake, 11, 7.0);
    task(&fake, 10, 2.0);
    task_pr(&fake, 11, 20);
    task_pr(&fake, 10, 20);
    open_pr(&fake, 20);
    let (code, v) = run(&fake, "session_a", NOW, &["budget", "report"]);
    assert_eq!(code, 0, "{v}");
    let rows = rows(&v);
    assert_eq!(rows.len(), 1, "{v}");
    assert_eq!(
        (rows[0]["issue"].as_u64(), rows[0]["usd"].as_f64()),
        (Some(10), Some(2.0))
    );
    assert_eq!(v["data"]["totals"]["usd"], 2.0);
    assert_eq!(v["data"]["totals"]["prs"], 1);
}

#[test]
fn unreadable_or_unwritten_task_pr_is_skipped() {
    let fake = FakeGithub::start();
    fake.issue(1, "bosun registry", &["bosun:registry"], "");
    fake.permission("mallory", "read");
    task(&fake, 10, 1.0); // task-pr names PR 999, which does not exist
    task_pr(&fake, 10, 999);
    task(&fake, 11, 1.0); // task-pr from a reader only
    fake.comment_by(
        11,
        "mallory",
        r#"{"bosun":"task-pr","session":"s","pr":20}"#,
    );
    open_pr(&fake, 20);
    let (code, v) = run(&fake, "session_a", NOW, &["budget", "report"]);
    assert_eq!(code, 0, "{v}");
    assert!(rows(&v).is_empty(), "{v}");
    assert_eq!(v["data"]["skipped_no_pr"], json!([10, 11]));
}

#[test]
fn a_readers_forged_spend_is_ignored() {
    let fake = FakeGithub::start();
    fake.issue(1, "bosun registry", &["bosun:registry"], "");
    fake.permission("mallory", "read");
    let forged = r#"{"bosun":"spend","session":"mallory","usd":50.0,"at":"2026-11-02T09:00:00Z"}"#;
    task(&fake, 10, 2.0);
    task_pr(&fake, 10, 20);
    fake.comment_by(10, "mallory", forged);
    open_pr(&fake, 20);
    // A task with only forged spend never enters the report.
    fake.issue(
        11,
        "Task 11",
        &[REVIEW],
        r#"<!-- bosun:task {"item":"M4","task":"T11"} -->"#,
    );
    task_pr(&fake, 11, 21);
    fake.comment_by(11, "mallory", forged);
    open_pr(&fake, 21);
    let (code, v) = run(&fake, "session_a", NOW, &["budget", "report"]);
    assert_eq!(code, 0, "{v}");
    let rows = rows(&v);
    assert_eq!(rows.len(), 1, "{v}");
    assert_eq!(rows[0]["usd"], 2.0, "{v}");
    assert_eq!(rows[0]["sessions"], json!(["s10"]), "{v}");
    assert_eq!(v["data"]["totals"]["usd"], 2.0);
}
