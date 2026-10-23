mod common;

use common::fake::FakeGithub;
use common::run;
use serde_json::{Value, json};

const NOW: &str = "2026-10-23T10:00:00Z";
const URL: &str = "https://example.test/runs/9";

fn health(fake: &FakeGithub, result: &str) -> (i32, Value) {
    run(
        fake,
        "session_a",
        NOW,
        &["main-health", "--result", result, "--run-url", URL],
    )
}

fn issues_with(fake: &FakeGithub, label: &str) -> Vec<u64> {
    let s = fake.state.lock().unwrap();
    s.issues
        .iter()
        .filter(|i| !i.is_pr && i.labels.iter().any(|l| l == label))
        .map(|i| i.number)
        .collect()
}

fn main_open_state(fake: &FakeGithub, sha: &str) -> String {
    let all = fake.statuses_of(sha);
    let last = all.last().expect("a status was posted");
    assert_eq!(last["context"], "main-open");
    last["state"].as_str().unwrap().to_string()
}

fn two_prs(fake: &FakeGithub) {
    fake.pull(10, "feature", "feat", "No link.");
    fake.pull(11, "fix main", "fix", "No link.");
    fake.pull_set(11, "labels", json!([{"name": "bosun:kind/mainfix"}]));
}

#[test]
fn uc_m3_1_red_push_opens_one_issue_and_green_closes_it() {
    let fake = FakeGithub::start();
    two_prs(&fake);

    let (code, v) = health(&fake, "failure");
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["command"], "main-health");
    assert_eq!(v["data"]["opened"], true);
    assert_eq!(v["data"]["rejudged"], 2);
    assert_eq!(v["data"]["lesson"]["created"], true);
    let red = issues_with(&fake, "bosun:main-red");
    assert_eq!(red.len(), 1);
    assert_eq!(v["data"]["main_red"], red[0]);
    let issue = fake.issue_state(red[0]);
    assert_eq!(issue.title, "main is red");
    assert!(issue.body.contains(URL));
    let lessons = issues_with(&fake, "bosun:lesson");
    assert_eq!(lessons.len(), 1);
    assert_eq!(v["data"]["lesson"]["issue"], lessons[0]);
    assert_eq!(main_open_state(&fake, "sha10"), "failure");
    assert_eq!(main_open_state(&fake, "sha11"), "success");

    // A second red run opens nothing new.
    let (code, v) = health(&fake, "failure");
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["opened"], false);
    assert_eq!(v["data"]["lesson"], Value::Null);
    assert_eq!(issues_with(&fake, "bosun:main-red"), red);
    assert_eq!(issues_with(&fake, "bosun:lesson"), lessons);

    let (code, v) = health(&fake, "success");
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["closed"], true);
    assert_eq!(v["data"]["main_red"], Value::Null);
    assert_eq!(fake.issue_state(red[0]).state, "closed");
    let comments = fake.comments_of(red[0]);
    assert!(
        comments
            .iter()
            .any(|c| c.body == format!("main is green again: {URL}"))
    );
    assert_eq!(main_open_state(&fake, "sha10"), "success");
}

#[test]
fn cancelled_run_changes_nothing() {
    let fake = FakeGithub::start();
    two_prs(&fake);
    let (code, v) = health(&fake, "cancelled");
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["changed"], false);
    assert_eq!(v["data"]["rejudged"], 0);
    assert!(issues_with(&fake, "bosun:main-red").is_empty());
    assert!(fake.statuses_of("sha10").is_empty());
}

#[test]
fn success_with_no_red_issue_changes_nothing() {
    let fake = FakeGithub::start();
    two_prs(&fake);
    let (code, v) = health(&fake, "success");
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["closed"], false);
    assert_eq!(v["data"]["changed"], false);
    assert!(fake.statuses_of("sha10").is_empty());
}

#[test]
fn empty_result_is_usage() {
    let fake = FakeGithub::start();
    let (code, v) = health(&fake, "");
    assert_eq!(code, 2, "{v}");
}
