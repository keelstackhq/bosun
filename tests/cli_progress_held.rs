//! Operating-model #9: `bosun progress` leaves tasks whose PR was reverted out of `held`.
mod common;

use common::fake::FakeGithub;
use common::run;
use serde_json::json;

const NOW: &str = "2026-10-23T12:00:00Z";

fn spend(usd: f64) -> String {
    format!(
        "{{\"bosun\":\"spend\",\"session\":\"s\",\"usd\":{usd},\"at\":\"2026-10-21T09:00:00Z\"}}"
    )
}

/// A merged task (spend $4) whose PR is `pr`.
fn task(fake: &FakeGithub, n: u64, pr: u64) {
    let body = format!(r#"<!-- bosun:task {{"item":"M4","task":"T{n}"}} -->"#);
    fake.issue(n, &format!("Task {n}"), &["bosun:state/merged"], &body);
    fake.comment(n, &spend(4.0));
    fake.comment(1, &spend(4.0));
    fake.comment(
        n,
        &format!(r#"{{"bosun":"task-pr","session":"s","pr":{pr}}}"#),
    );
    fake.close_issue(n, "2026-10-22T09:00:00Z");
}

fn merged_pull(fake: &FakeGithub, n: u64, head: &str) {
    fake.pull(n, &format!("PR {n}"), head, "");
    fake.pull_set(n, "state", json!("closed"));
    fake.pull_set(n, "merged", json!(true));
}

#[test]
fn reverted_task_is_not_held() {
    let fake = FakeGithub::start();
    fake.issue(1, "bosun registry", &["bosun:registry"], "");
    task(&fake, 10, 30);
    task(&fake, 11, 31);
    merged_pull(&fake, 30, "feat/a");
    merged_pull(&fake, 31, "feat/b");
    merged_pull(&fake, 40, "bosun/revert-30");
    // An open revert PR does not count yet.
    fake.pull(41, "Revert #31", "bosun/revert-31", "");
    let (code, v) = run(&fake, "session_a", NOW, &["progress"]);
    assert_eq!(code, 0, "{v}");
    let d = &v["data"];
    assert_eq!(d["merged"], 2);
    assert_eq!(d["reverted"], 1);
    assert_eq!(d["held"], 1);
    assert_eq!(d["spend_usd"], 8.0);
    assert_eq!(d["cost_per_merged_task_usd"], 4.0);
    assert_eq!(d["cost_per_held_task_usd"], 8.0);
}

#[test]
fn nothing_reverted_or_all_reverted() {
    let fake = FakeGithub::start();
    fake.issue(1, "bosun registry", &["bosun:registry"], "");
    task(&fake, 10, 30);
    merged_pull(&fake, 30, "feat/a");
    let (_, v) = run(&fake, "session_a", NOW, &["progress"]);
    assert_eq!(v["data"]["reverted"], 0);
    assert_eq!(v["data"]["held"], 1);
    assert_eq!(v["data"]["cost_per_held_task_usd"], 4.0);
    merged_pull(&fake, 40, "bosun/revert-30");
    let (_, v) = run(&fake, "session_a", NOW, &["progress"]);
    assert_eq!(v["data"]["held"], 0);
    assert!(v["data"]["cost_per_held_task_usd"].is_null());
}
