//! A dismissed owner request for changes still withdraws approvals (spec §8). Only a newer owner
//! APPROVED review or a plain owner comment made after the dismissal clears it, never a signature.
mod common;

use common::fake::FakeGithub;
use common::{bosun, one_object, run};
use serde_json::{Value, json};

const HEAD: &str = "0123456789abcdef0123456789abcdef01234567";
// The fake stamps reviews 10:00; the timeline dismisses review 2 at 10:30.
const OLDER: &str = "2026-10-22T09:00:00Z";
const BETWEEN: &str = "2026-10-22T10:15:00Z";
const NEWER: &str = "2026-10-22T11:00:00Z";
const PEM: &str = "-----BEGIN PRIVATE KEY-----\nMC4CAQAwBQYDK2VwBCIEIJ1hsZ3v/VpguoRK9JLsLMREScVpezJpGXA7rAMcrn9g\n-----END PRIVATE KEY-----\n";
const PUB: &str = "11qYAYKxCrfVS/7TyWQHOg7hcvPapiMlrwIaaPcHURo=";
const SIG: &str =
    "dTnLJ8NM8jp0umTiTZRuGW0lwGdxPoLlu069r9NXkSyrLmKj3SBW1ULZrmXNsE/qzfn+5gY0INdSAqttzPxmBg==";

/// High PR 1 at HEAD, owner `ada` at base, key `k1` enrolled on main.
fn high_pr() -> FakeGithub {
    let fake = FakeGithub::start();
    fake.pull(1, "change", "feature", "");
    fake.pull_as_issue(1, "change");
    fake.pull_set(1, "head", json!({"sha": HEAD, "ref": "feature"}));
    fake.content(
        "governance/owners.toml",
        "base0000",
        "[[owner]]\ngithub = \"ada\"\n",
    );
    let keys = format!("[[key]]\nid = \"k1\"\ned25519 = \"{PUB}\"\n");
    fake.content(".bosun/approvers.toml", "refs/heads/main", &keys);
    let diff = Some("@@ -0,0 +1,1 @@\n+x");
    fake.pull_file(1, ".github/workflows/ci.yml", "modified", 1, 0, diff);
    fake
}

fn check(fake: &FakeGithub) -> (i32, Value) {
    run(
        fake,
        "session_test",
        "2026-10-22T12:00:00Z",
        &["risk-check", "1"],
    )
}

/// A request for changes (review 2) that was dismissed; the timeline says when unless `seen` is false.
fn dismissed(fake: &FakeGithub, seen: bool) {
    fake.review(1, "ada", "CHANGES_REQUESTED", HEAD);
    fake.review(1, "ada", "DISMISSED", HEAD);
    if seen {
        fake.dismiss_event(1, 2, "2026-10-22T10:30:00Z");
    }
}

fn signed(fake: &FakeGithub, at: &str) {
    let body = format!("/bosun approve {HEAD}\nbosun-sig ed25519 k1 {SIG}");
    fake.comment_at(1, "claude", &body, at);
    fake.last_comment(|c| c.via_app = Some("claude".into()));
}

fn owner_says(fake: &FakeGithub, at: &str) {
    fake.comment_at(1, "ada", &format!("/bosun approve {HEAD}"), at);
}

type Setup = fn(&FakeGithub);

#[test]
fn nothing_but_a_newer_review_or_plain_comment_ends_a_dismissal() {
    let cases: [(&str, Setup); 9] = [
        ("older comment", |f| {
            owner_says(f, OLDER);
            dismissed(f, true);
        }),
        ("older signature", |f| {
            signed(f, OLDER);
            dismissed(f, true);
        }),
        ("replayed signature", |f| {
            signed(f, OLDER);
            dismissed(f, true);
            signed(f, NEWER);
        }),
        ("comment before the dismissal", |f| {
            dismissed(f, true);
            owner_says(f, BETWEEN);
        }),
        ("edited comment", |f| {
            dismissed(f, true);
            owner_says(f, NEWER);
            f.last_comment(|c| c.updated_at = "2026-10-22T11:30:00Z".into());
        }),
        ("app comment", |f| {
            dismissed(f, true);
            owner_says(f, NEWER);
            f.last_comment(|c| c.via_app = Some("claude".into()));
        }),
        ("plain request for changes", |f| {
            f.review(1, "ada", "CHANGES_REQUESTED", HEAD);
            owner_says(f, NEWER);
        }),
        ("unreadable timeline", |f| {
            dismissed(f, false);
            owner_says(f, NEWER);
        }),
        ("event for another review", |f| {
            dismissed(f, false);
            f.dismiss_event(1, 99, "2026-10-22T10:30:00Z");
            owner_says(f, NEWER);
        }),
    ];
    for (name, setup) in cases {
        let fake = high_pr();
        setup(&fake);
        let (code, v) = check(&fake);
        assert_eq!(code, 5, "{name}: {v}");
        assert_eq!(v["data"]["approved_by"], Value::Null, "{name}");
    }
}

#[test]
fn a_newer_review_or_plain_comment_approves() {
    let cases: [(&str, &str, Setup); 3] = [
        ("ada", "newer APPROVED review", |f| {
            dismissed(f, true);
            f.review(1, "ada", "APPROVED", HEAD);
        }),
        ("ada", "comment after the dismissal", |f| {
            dismissed(f, true);
            owner_says(f, NEWER);
        }),
        ("key k1", "signature, no dismissal", |f| signed(f, NEWER)),
    ];
    for (who, name, setup) in cases {
        let fake = high_pr();
        setup(&fake);
        let (code, v) = check(&fake);
        assert_eq!(code, 0, "{name}: {v}");
        assert_eq!(v["data"]["approved_by"], who, "{name}");
    }
}

#[test]
fn sign_key_refuses_a_withdrawn_pr() {
    let fake = high_pr();
    dismissed(&fake, true);
    let key = tempfile::tempdir().unwrap().keep().join("key.pem");
    std::fs::write(&key, PEM).unwrap();
    let out = bosun()
        .env("BOSUN_GITHUB_API", fake.url())
        .args(["approve", &format!("1@{HEAD}"), "--print", "--sign-key"])
        .arg(&key)
        .output()
        .unwrap();
    let v = one_object(&out.stdout);
    assert_eq!(out.status.code(), Some(3), "{v}");
    assert!(v["error"]["fix"].as_str().unwrap().contains("plain"));
    assert!(v["data"]["body"].is_null(), "nothing was signed: {v}");
}
