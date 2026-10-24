mod common;

use common::fake::FakeGithub;
use common::run;
use serde_json::{Value, json};

const T0: &str = "2026-10-22T10:00:00Z";

/// PR 42 with files a.rs and b.rs against the full-review fake.
fn setup() -> FakeGithub {
    let fake = FakeGithub::start();
    fake.pull_as_issue(42, "the pr");
    fake.pull(42, "the pr", "claude/x", "no link");
    fake.pull_file(42, "a.rs", "added", 1, 0, Some("@@ a"));
    fake.pull_file(42, "b.rs", "added", 1, 0, Some("@@ b"));
    fake
}

/// A verdict record for `sha`, with a `review-verdict` status in `state`.
fn verdict_at(fake: &FakeGithub, sha: &str, state: &str) {
    fake.comment(
        42,
        &format!(r#"{{"bosun":"verdict","session":"s","head_sha":"{sha}","approved":true}}"#),
    );
    fake.status(sha, "review-verdict", state, "x");
}

fn set_head(fake: &FakeGithub, sha: &str) {
    fake.pull_set(
        42,
        "head",
        json!({"sha": sha, "ref": "claude/x", "label": "acme:claude/x"}),
    );
}

fn pack(fake: &FakeGithub) -> Value {
    let (code, v) = run(fake, "s", T0, &["review-pack", "42"]);
    assert_eq!(code, 0, "{v}");
    v
}

fn names(v: &Value) -> Vec<&str> {
    v["data"]["files"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| f["filename"].as_str().unwrap())
        .collect()
}

/// UC-M8-2: a second review covers only the new commits.
#[test]
fn uc_m8_2() {
    let fake = setup();
    verdict_at(&fake, "A", "success");
    set_head(&fake, "B");
    fake.compare_file("A", "B", "b.rs", "@@ new");
    let v = pack(&fake);
    assert_eq!(v["data"]["since_sha"], "A");
    assert_eq!(names(&v), ["b.rs"]);
    assert_eq!(v["data"].get("delta_fallback"), Some(&Value::Null), "{v}");
}

#[test]
fn since_is_the_newest_older_head_that_was_approved() {
    let fake = setup();
    verdict_at(&fake, "A", "success");
    verdict_at(&fake, "B", "failure");
    set_head(&fake, "C");
    fake.compare_file("A", "C", "b.rs", "@@ since A");
    fake.compare_file("B", "C", "a.rs", "@@ since B, wrong");
    let v = pack(&fake);
    assert_eq!(v["data"]["since_sha"], "A", "{v}");
    assert_eq!(names(&v), ["b.rs"]);
    assert!(v["data"]["delta_fallback"].is_null());
}

#[test]
fn a_record_on_the_current_head_is_skipped() {
    let fake = setup();
    verdict_at(&fake, "A", "success");
    verdict_at(&fake, "C", "success");
    set_head(&fake, "C");
    fake.compare_file("A", "C", "b.rs", "@@ x");
    assert_eq!(pack(&fake)["data"]["since_sha"], "A");
}

#[test]
fn a_failed_compare_falls_back_to_a_full_review() {
    let fake = setup();
    verdict_at(&fake, "A", "success");
    set_head(&fake, "B");
    // no compare A...B in the fake: GitHub cannot compare (a force-push removed A)
    let (code, v) = run(&fake, "s", T0, &["review-pack", "42"]);
    assert_eq!(code, 0, "{v}");
    assert!(v["data"]["since_sha"].is_null(), "{v}");
    assert_eq!(names(&v), ["a.rs", "b.rs"]);
    let why = v["data"]["delta_fallback"].as_str().unwrap();
    assert!(
        why.contains("A...B") && why.contains("cannot compare"),
        "{why}"
    );
}

fn assert_full_with_reason(v: &Value, sha: &str, status: &str) {
    assert!(v["data"]["since_sha"].is_null(), "{v}");
    assert_eq!(names(v), ["a.rs", "b.rs"]);
    let why = v["data"]["delta_fallback"].as_str().unwrap();
    assert!(
        why.contains(sha) && why.contains("not an ancestor"),
        "{why}"
    );
    assert!(why.contains(status), "{why}");
}

#[test]
fn a_since_that_is_behind_the_head_gives_a_full_review() {
    // head D approved, then a force-push back to D~1: D...D~1 is empty and `behind`
    let fake = setup();
    verdict_at(&fake, "D", "success");
    set_head(&fake, "D1");
    fake.compare_file("D", "D1", "a.rs", "@@ misleading");
    fake.compare_status("D", "D1", "behind");
    assert_full_with_reason(&pack(&fake), "D", "behind");
}

#[test]
fn a_diverged_since_gives_a_full_review() {
    let fake = setup();
    verdict_at(&fake, "A", "success");
    set_head(&fake, "B");
    fake.compare_file("A", "B", "b.rs", "@@ misleading");
    fake.compare_status("A", "B", "diverged");
    assert_full_with_reason(&pack(&fake), "A", "diverged");
}

#[test]
fn each_older_sha_is_asked_once_and_the_scan_is_capped() {
    let fake = setup();
    for _ in 0..25 {
        verdict_at(&fake, "Z", "failure");
    }
    let v = pack(&fake);
    assert!(v["data"]["since_sha"].is_null());
    assert!(v["data"]["delta_fallback"].is_null(), "{v}");
    let asked = |sha: &str| {
        fake.requests()
            .iter()
            .filter(|r| r.path.contains(&format!("/commits/{sha}/")))
            .count()
    };
    assert_eq!(asked("Z"), 1);
    for i in 0..25 {
        verdict_at(&fake, &format!("S{i:02}"), "failure");
    }
    let v = pack(&fake);
    assert_eq!(
        v["data"]["delta_fallback"], "too many verdict records",
        "{v}"
    );
    assert_eq!(names(&v), ["a.rs", "b.rs"]);
    assert_eq!(asked("S04"), 0, "the scan stopped at the cap");
}

#[test]
fn an_approved_ahead_sha_outside_the_pr_gives_a_full_review() {
    // X was approved in a stacked PR and is an ancestor, but is not a commit of this PR
    let fake = setup();
    verdict_at(&fake, "X", "success");
    set_head(&fake, "B");
    fake.compare_file("X", "B", "b.rs", "@@ only after X");
    fake.pull_commit_shas(42, &["B"]);
    let v = pack(&fake);
    assert!(v["data"]["since_sha"].is_null(), "{v}");
    assert_eq!(names(&v), ["a.rs", "b.rs"]);
    assert_eq!(v["data"]["delta_fallback"], "X is not a commit of this PR");
    // the same sha is a delta once it is one of the PR's commits
    fake.pull_commit_shas(42, &["X", "B"]);
    assert_eq!(pack(&fake)["data"]["since_sha"], "X");
}
