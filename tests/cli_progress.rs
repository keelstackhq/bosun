//! UC-M4-4: `bosun progress` shows the cost per merged task for the last 7 days.
mod common;

use common::fake::FakeGithub;
use common::run;
use serde_json::{Value, json};

const NOW: &str = "2026-10-23T12:00:00Z";

fn spend(usd: f64, at: &str) -> String {
    format!("{{\"bosun\":\"spend\",\"session\":\"s\",\"usd\":{usd},\"at\":\"{at}\"}}")
}

/// A task issue with its spend on the task and on the registry (issue 1), as `budget record`.
fn task(fake: &FakeGithub, n: u64, labels: &[&str], usd: f64, at: &str) {
    let body = format!(r#"<!-- bosun:task {{"item":"M4","task":"T{n}"}} -->"#);
    fake.issue(n, &format!("Task {n}"), labels, &body);
    fake.comment(n, &spend(usd, at));
    fake.comment(1, &spend(usd, at));
}

/// Two tasks done in the window ($4, $2), one done 10 days ago ($6), one open ($3).
fn seeded() -> FakeGithub {
    let fake = FakeGithub::start();
    fake.issue(1, "bosun registry", &["bosun:registry"], "");
    task(
        &fake,
        10,
        &["bosun:state/merged"],
        4.0,
        "2026-10-21T09:00:00Z",
    );
    fake.close_issue(10, "2026-10-22T09:00:00Z");
    task(
        &fake,
        11,
        &["bosun:state/merged"],
        2.0,
        "2026-10-20T09:00:00Z",
    );
    fake.close_issue(11, "2026-10-23T08:00:00Z");
    task(
        &fake,
        12,
        &["bosun:state/merged"],
        6.0,
        "2026-10-12T09:00:00Z",
    );
    fake.close_issue(12, "2026-10-13T09:00:00Z");
    task(
        &fake,
        13,
        &["bosun:state/building"],
        3.0,
        "2026-10-22T11:00:00Z",
    );
    fake
}

fn no_writes(fake: &FakeGithub) {
    for r in fake.requests() {
        assert!(
            r.method == "GET",
            "progress is read-only, saw {} {}",
            r.method,
            r.path
        );
    }
}

#[test]
fn uc_m4_4_cost_per_merged_task_over_7_days() {
    let fake = seeded();
    let (code, v) = run(&fake, "session_a", NOW, &["progress"]);
    assert_eq!(code, 0, "{v}");
    let d = &v["data"];
    assert_eq!(v["command"], "progress");
    assert_eq!(d["days"], 7);
    assert_eq!(d["since"], "2026-10-16T12:00:00Z");
    assert_eq!(d["merged"], 2);
    // Registry spend in the window: 4 + 2 + 3 (the $6 of 10 days ago is outside).
    assert_eq!(d["spend_usd"], 9.0);
    assert_eq!(d["cost_per_merged_task_usd"], 4.5);
    assert_eq!(d["task_cost_avg_usd"], 3.0);
    assert_eq!(
        d["tasks"],
        json!([
            {"issue": 10, "title": "Task 10", "closed_at": "2026-10-22T09:00:00Z", "usd": 4.0},
            {"issue": 11, "title": "Task 11", "closed_at": "2026-10-23T08:00:00Z", "usd": 2.0},
        ])
    );
    no_writes(&fake);
}

#[test]
fn uc_m4_4_days_30_includes_the_older_task() {
    let fake = seeded();
    let (code, v) = run(&fake, "session_a", NOW, &["progress", "--days", "30"]);
    assert_eq!(code, 0, "{v}");
    let d = &v["data"];
    assert_eq!(d["days"], 30);
    assert_eq!(d["merged"], 3);
    assert_eq!(d["spend_usd"], 15.0);
    assert_eq!(d["cost_per_merged_task_usd"], 5.0);
    assert_eq!(d["task_cost_avg_usd"], 4.0);
    let issues: Vec<u64> = d["tasks"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["issue"].as_u64().unwrap())
        .collect();
    assert_eq!(issues, [10, 11, 12]);
    no_writes(&fake);
}

#[test]
fn uc_m4_4_zero_merged_has_null_headline_and_exit_0() {
    let fake = FakeGithub::start();
    fake.issue(1, "bosun registry", &["bosun:registry"], "");
    task(
        &fake,
        13,
        &["bosun:state/building"],
        3.0,
        "2026-10-22T11:00:00Z",
    );
    let (code, v) = run(&fake, "session_a", NOW, &["progress"]);
    assert_eq!(code, 0, "{v}");
    let d = &v["data"];
    assert_eq!(d["merged"], 0);
    assert_eq!(d["spend_usd"], 3.0);
    assert!(d["cost_per_merged_task_usd"].is_null() && d["task_cost_avg_usd"].is_null());
    assert_eq!(d["tasks"], Value::Array(vec![]));
    no_writes(&fake);
}

#[test]
fn uc_m4_4_empty_repo_is_ok() {
    let fake = FakeGithub::start();
    let (code, v) = run(&fake, "session_a", NOW, &["progress"]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["spend_usd"], 0.0);
    assert!(v["data"]["cost_per_merged_task_usd"].is_null());
}

#[test]
fn uc_m4_4_not_planned_tasks_non_tasks_and_future_spend_do_not_count() {
    let fake = seeded();
    // A task closed as not planned, in the window: its spend counts, it is not merged.
    task(&fake, 14, &[], 5.0, "2026-10-22T10:00:00Z");
    fake.close_issue_as(14, "2026-10-22T11:00:00Z", "not_planned");
    // A closed issue without task metadata is not a task.
    fake.issue(15, "Not a task", &[], "plain issue");
    fake.close_issue(15, "2026-10-22T12:00:00Z");
    // A registry record stamped after now (clock skew) is outside the window.
    fake.comment(1, &spend(100.0, "2026-10-24T00:00:00Z"));
    let (code, v) = run(&fake, "session_a", NOW, &["progress"]);
    assert_eq!(code, 0, "{v}");
    let d = &v["data"];
    assert_eq!(d["merged"], 2);
    assert_eq!(d["spend_usd"], 14.0);
    assert_eq!(d["cost_per_merged_task_usd"], 7.0);
    no_writes(&fake);
}
