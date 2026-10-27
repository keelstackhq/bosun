//! A dismissed owner request for changes still withdraws approvals (spec §8): anyone with write
//! access can dismiss a review, and GitHub then reports it as DISMISSED. Only a newer owner
//! APPROVED review or a newer plain-token owner comment clears it, never a signature.
mod common;

use common::fake::FakeGithub;
use common::run;
use serde_json::{Value, json};

const NOW: &str = "2026-10-22T12:00:00Z";
const BASE: &str = "base0000";
const HEAD: &str = "0123456789abcdef0123456789abcdef01234567";
const MAIN: &str = "refs/heads/main";
const OWNERS: &str = "[[owner]]\ngithub = \"ada\"\n";
/// The fake stamps every review `2026-10-22T10:00:00Z`.
const OLDER: &str = "2026-10-22T09:00:00Z";
const NEWER: &str = "2026-10-22T11:00:00Z";
// RFC 8032 test 1 key, enrolled as `k1`; SIG signs `bosun-approve v1 acme/widget#1 <HEAD>`.
const PUB: &str = "11qYAYKxCrfVS/7TyWQHOg7hcvPapiMlrwIaaPcHURo=";
const SIG: &str =
    "dTnLJ8NM8jp0umTiTZRuGW0lwGdxPoLlu069r9NXkSyrLmKj3SBW1ULZrmXNsE/qzfn+5gY0INdSAqttzPxmBg==";
// RFC 8032 test 2 key, enrolled as the delegate `d1`; SIG_D is its signature over the OWNER message.
const SIG_D: &str =
    "QaSlWA89SY/lbC3KRhb7YMRUCs8twG1ipEnTH1RI7j3C6YmLPO5fj5lhr5Sm4hBYozbu0XKkPfWAOunh4IYfBQ==";
const PUB_D: &str = "PUAXw+hDiVqStwqnTRt+vJyYLM8uxJaMwM1V8Sr0Zgw=";

/// High PR 1 at HEAD, owner `ada` at base, owner key `k1` and delegate key `d1` enrolled on main.
fn high_pr() -> FakeGithub {
    let fake = FakeGithub::start();
    fake.pull(1, "change", "feature", "");
    fake.pull_as_issue(1, "change");
    fake.pull_set(1, "head", json!({"sha": HEAD, "ref": "feature"}));
    fake.content("governance/owners.toml", BASE, OWNERS);
    fake.content(
        ".bosun/approvers.toml",
        MAIN,
        &format!(
            "[[key]]\nid = \"k1\"\ned25519 = \"{PUB}\"\n\n[[key]]\nid = \"d1\"\nrole = \"delegate\"\ned25519 = \"{PUB_D}\"\n"
        ),
    );
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

fn check(fake: &FakeGithub) -> (i32, Value) {
    run(fake, "session_test", NOW, &["risk-check", "1"])
}

fn dismissed(fake: &FakeGithub) {
    fake.review(1, "ada", "CHANGES_REQUESTED", HEAD);
    fake.review(1, "ada", "DISMISSED", HEAD);
}

fn signed(fake: &FakeGithub, at: &str) {
    let body = format!("/bosun approve {HEAD}\nbosun-sig ed25519 k1 {SIG}");
    fake.comment_at(1, "claude", &body, at);
    fake.last_comment(|c| c.via_app = Some("claude".into()));
}

fn denied(fake: &FakeGithub) {
    let (code, v) = check(fake);
    assert_eq!(code, 5, "{v}");
    assert_eq!(v["data"]["approved_by"], Value::Null);
}

#[test]
fn a_dismissed_request_withdraws_an_older_comment_approval() {
    let fake = high_pr();
    fake.comment_at(1, "ada", &format!("/bosun approve {HEAD}"), OLDER);
    dismissed(&fake);
    denied(&fake);
}

#[test]
fn a_dismissed_request_withdraws_an_older_signed_approval() {
    let fake = high_pr();
    signed(&fake, OLDER);
    dismissed(&fake);
    denied(&fake);
}

#[test]
fn a_newer_owner_review_or_plain_comment_clears_a_dismissal() {
    // a newer APPROVED review
    let fake = high_pr();
    dismissed(&fake);
    fake.review(1, "ada", "APPROVED", HEAD);
    let (code, v) = check(&fake);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["approved_by"], "ada");

    // a plain-token owner comment made after the dismissal
    let fake = high_pr();
    dismissed(&fake);
    fake.comment_at(1, "ada", &format!("/bosun approve {HEAD}"), NEWER);
    let (code, v) = check(&fake);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["approved_by"], "ada");
}

#[test]
fn an_edited_or_app_written_comment_after_the_dismissal_does_not_clear_it() {
    // Someone with write access edited the owner's newer comment.
    let fake = high_pr();
    dismissed(&fake);
    fake.comment_at(1, "ada", &format!("/bosun approve {HEAD}"), NEWER);
    fake.last_comment(|c| c.updated_at = "2026-10-22T11:30:00Z".into());
    denied(&fake);

    // An agent session acting as the owner through an app.
    let fake = high_pr();
    dismissed(&fake);
    fake.comment_at(1, "ada", &format!("/bosun approve {HEAD}"), NEWER);
    fake.last_comment(|c| c.via_app = Some("claude".into()));
    denied(&fake);
}

#[test]
fn a_signed_comment_posted_after_the_dismissal_does_not_clear_it() {
    // A signature has no time: anyone can re-post an old valid one (replay).
    let fake = high_pr();
    signed(&fake, OLDER);
    dismissed(&fake);
    signed(&fake, NEWER);
    denied(&fake);
}

#[test]
fn a_delegate_signature_does_not_clear_a_dismissal() {
    // The signature verifies, but a delegate-role key is not an owner key.
    let fake = high_pr();
    dismissed(&fake);
    let body = format!("/bosun approve {HEAD}\nbosun-sig ed25519 d1 {SIG_D}");
    fake.comment_at(1, "claude", &body, NEWER);
    fake.last_comment(|c| c.via_app = Some("claude".into()));
    denied(&fake);
}

#[test]
fn a_plain_request_for_changes_is_not_cleared_by_a_newer_comment() {
    let fake = high_pr();
    fake.review(1, "ada", "CHANGES_REQUESTED", HEAD);
    fake.comment_at(1, "ada", &format!("/bosun approve {HEAD}"), NEWER);
    denied(&fake);
}

#[test]
fn a_dismissed_review_alone_is_no_approval() {
    let fake = high_pr();
    fake.review(1, "ada", "DISMISSED", HEAD);
    denied(&fake);
}

#[test]
fn a_comment_approval_without_reviews_still_approves() {
    let fake = high_pr();
    fake.comment_by(1, "ada", &format!("/bosun approve {HEAD}"));
    let (code, v) = check(&fake);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["approved_by"], "ada");
}
