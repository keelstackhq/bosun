mod common;

use common::fake::FakeGithub;
use common::{bosun, one_object, run};
use serde_json::{Value, json};

const T0: &str = "2026-10-22T10:00:00Z";

/// PR 42 (head `sha42`) closing task issue 7 in `review`.
fn setup() -> FakeGithub {
    let fake = FakeGithub::start();
    fake.issue(7, "task", &["bosun:state/review"], "Do it.");
    fake.pull_as_issue(42, "the pr");
    fake.pull(42, "the pr", "claude/x", "Closes #7");
    fake
}

fn verdict(sha: &str, v: &str, conf: f64, sev: &[&str], escalate: bool) -> String {
    let findings: Vec<Value> = sev
        .iter()
        .map(|s| json!({"severity": s, "file": "a.rs", "line": 3, "claim": "c", "evidence": "e"}))
        .collect();
    json!({"pr": 42, "head_sha": sha, "verdict": v, "confidence": conf,
        "findings": findings, "lessons_hit": [], "escalate_risk": escalate})
    .to_string()
}

fn post(fake: &FakeGithub, input: &str) -> (i32, Value) {
    let out = bosun()
        .env("BOSUN_GITHUB_API", fake.url())
        .args(["verdict", "post"])
        .write_stdin(input.to_string())
        .output()
        .unwrap();
    (out.status.code().unwrap(), one_object(&out.stdout))
}

#[test]
fn approve_at_threshold_sets_success() {
    let fake = setup();
    let (code, v) = post(&fake, &verdict("sha42", "approve", 0.85, &["major"], false));
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["approved"], true);
    let st = fake.statuses_of("sha42");
    assert_eq!(st[0]["context"], "review-verdict");
    assert_eq!(st[0]["state"], "success");
    assert_eq!(st[0]["description"], "approve 0.85, 0 blocker, 1 major");
    // Task 15 queues the task, not the verdict
    assert!(fake.labels_of(7).contains(&"bosun:state/review".into()));
    let rec = fake.comments_of(42);
    assert!(rec[0].body.contains(r#""bosun":"verdict""#));
    assert!(rec[0].body.contains(r#""head_sha":"sha42""#));
}

#[test]
fn below_threshold_does_not_approve() {
    let fake = setup();
    let (code, v) = post(&fake, &verdict("sha42", "approve", 0.84, &[], false));
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["approved"], false);
    assert_eq!(fake.statuses_of("sha42")[0]["state"], "failure");
    assert!(fake.labels_of(7).contains(&"bosun:state/fixing".into()));
}

#[test]
fn a_blocker_blocks_and_request_changes_moves_to_fixing() {
    let fake = setup();
    let (_, v) = post(
        &fake,
        &verdict("sha42", "approve", 0.99, &["blocker"], false),
    );
    assert_eq!(v["data"]["approved"], false);
    assert!(
        v["data"]["reasons"][0]
            .as_str()
            .unwrap()
            .contains("blocker")
    );
    assert_eq!(fake.statuses_of("sha42")[0]["state"], "failure");
    assert!(fake.labels_of(7).contains(&"bosun:state/fixing".into()));
}

#[test]
fn stale_sha_is_exit_3() {
    let fake = setup();
    let (code, v) = post(&fake, &verdict("old", "approve", 0.9, &[], false));
    assert_eq!(code, 3);
    assert!(
        v["error"]["message"]
            .as_str()
            .unwrap()
            .contains("stale verdict for old")
    );
    assert!(v["error"]["fix"].as_str().unwrap().contains("review-pack"));
    assert!(fake.statuses_of("sha42").is_empty());
}

#[test]
fn escalate_risk_needs_human() {
    let fake = setup();
    let (code, _) = post(&fake, &verdict("sha42", "approve", 0.95, &[], true));
    assert_eq!(code, 0);
    assert!(
        fake.labels_of(7)
            .contains(&"bosun:state/needs-human".into())
    );
}

#[test]
fn invalid_json_is_usage_and_names_the_field() {
    let fake = setup();
    let (code, v) = post(&fake, r#"{"pr":42,"head_sha":"sha42","verdict":"approve"}"#);
    assert_eq!(code, 2);
    assert!(
        v["error"]["message"]
            .as_str()
            .unwrap()
            .contains("confidence")
    );
}

#[test]
fn halted_by_the_kill_switch() {
    let fake = setup();
    fake.variable("BOSUN_HALT", "1");
    let (code, _) = run(
        &fake,
        "s",
        T0,
        &["verdict", "post", "--from", "/nonexistent"],
    );
    assert_eq!(code, 4);
}
