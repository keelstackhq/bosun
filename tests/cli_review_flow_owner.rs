//! M7: a valid owner approval of the exact head clears an escalated review-verdict.
mod common;

use common::fake::FakeGithub;
use common::run;
use serde_json::Value;

const NOW: &str = "2026-10-22T10:00:00Z";
const BASE: &str = "base0000";
const HEAD: &str = "sha1";
const OWNERS: &str = "[[owner]]\ngithub = \"ada\"\n";

fn high_pr() -> FakeGithub {
    let fake = FakeGithub::start();
    fake.pull(1, "change", "feature", "");
    fake.content("governance/owners.toml", BASE, OWNERS);
    fake.pull_file(
        1,
        ".github/workflows/ci.yml",
        "modified",
        1,
        0,
        Some("@@ -0,0 +1,1 @@\n+x"),
    );
    fake
}

fn last(fake: &FakeGithub, sha: &str, ctx: &str) -> Option<Value> {
    fake.statuses_of(sha)
        .into_iter()
        .rev()
        .find(|s| s["context"] == ctx)
}

const ESCALATED: &str =
    "escalated to the owner (approvable): approve 0.95 (approves at 0.85), 0 blocker, 0 major";

#[test]
fn an_owner_approval_of_the_head_clears_an_escalated_verdict() {
    let fake = high_pr();
    fake.status(HEAD, "review-verdict", "failure", ESCALATED);
    fake.review(1, "ada", "APPROVED", HEAD);
    let (code, v) = run(&fake, "session_test", NOW, &["risk-check", "1"]);
    assert_eq!(code, 0, "{v}");
    let s = last(&fake, HEAD, "review-verdict").unwrap();
    assert_eq!(s["state"], "success", "{s}");
    assert!(
        s["description"]
            .as_str()
            .unwrap()
            .contains("owner cleared escalation"),
        "{s}"
    );
    assert_eq!(v["data"]["verdict_cleared_by"], "ada");
}

#[test]
fn an_owner_approval_of_another_head_does_not_clear_it() {
    let fake = high_pr();
    fake.status(HEAD, "review-verdict", "failure", ESCALATED);
    fake.review(1, "ada", "APPROVED", "oldsha");
    let (code, _) = run(&fake, "session_test", NOW, &["risk-check", "1"]);
    assert_eq!(code, 5);
    assert_eq!(
        last(&fake, HEAD, "review-verdict").unwrap()["state"],
        "failure"
    );
}

#[test]
fn an_owner_approval_does_not_clear_a_plain_request_changes() {
    let fake = high_pr();
    let rc = "request-changes 0.90 (approves at 0.85), 1 blocker, 0 major";
    fake.status(HEAD, "review-verdict", "failure", rc);
    fake.review(1, "ada", "APPROVED", HEAD);
    let (code, v) = run(&fake, "session_test", NOW, &["risk-check", "1"]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(
        last(&fake, HEAD, "review-verdict").unwrap()["state"],
        "failure"
    );
}

#[test]
fn an_owner_approval_does_not_clear_an_escalated_request_for_changes() {
    let fake = high_pr();
    let rc = "escalated to the owner (not approving): request-changes 0.90 (approves at 0.85), 1 blocker, 0 major";
    fake.status(HEAD, "review-verdict", "failure", rc);
    fake.review(1, "ada", "APPROVED", HEAD);
    let (code, v) = run(&fake, "session_test", NOW, &["risk-check", "1"]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(
        last(&fake, HEAD, "review-verdict").unwrap()["state"],
        "failure"
    );
}

#[test]
fn an_owner_approval_clears_an_escalation_on_a_normal_risk_head() {
    let fake = FakeGithub::start();
    fake.pull(1, "change", "feature", "");
    fake.content("governance/owners.toml", BASE, OWNERS);
    fake.pull_file(1, "src/a.rs", "modified", 1, 0, Some("@@ -0,0 +1,1 @@\n+x"));
    fake.status(HEAD, "review-verdict", "failure", ESCALATED);
    let (code, v) = run(&fake, "session_test", NOW, &["risk-check", "1"]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(
        last(&fake, HEAD, "review-verdict").unwrap()["state"],
        "failure",
        "no approval yet"
    );
    fake.review(1, "ada", "APPROVED", HEAD);
    let (code, v) = run(&fake, "session_test", NOW, &["risk-check", "1"]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(
        last(&fake, HEAD, "review-verdict").unwrap()["state"],
        "success"
    );
    assert_eq!(v["data"]["verdict_cleared_by"], "ada");
}

#[test]
fn an_old_release_escalated_request_for_changes_is_not_cleared() {
    let fake = high_pr();
    let old = "escalated to the owner; request-changes 0.40 (approves at 0.85), 2 blocker, 0 major";
    fake.status(HEAD, "review-verdict", "failure", old);
    fake.review(1, "ada", "APPROVED", HEAD);
    let (code, v) = run(&fake, "session_test", NOW, &["risk-check", "1"]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(
        last(&fake, HEAD, "review-verdict").unwrap()["state"],
        "failure"
    );
}

#[test]
fn an_old_release_escalated_approval_is_not_cleared_either() {
    // The old form rounds the confidence: 0.849 reads 0.85. Only the new form is judged unrounded.
    let fake = high_pr();
    let old = "escalated to the owner; approve 0.85 (approves at 0.85), 0 blocker, 0 major";
    fake.status(HEAD, "review-verdict", "failure", old);
    fake.review(1, "ada", "APPROVED", HEAD);
    let (code, v) = run(&fake, "session_test", NOW, &["risk-check", "1"]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(
        last(&fake, HEAD, "review-verdict").unwrap()["state"],
        "failure"
    );
}
