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
const OLD_HEAD: &str = "1111111111111111111111111111111111111111";
/// `bosun-approve v2 acme/widget#1 <HEAD> after-review 2`, and `... after-review 4`.
const SIG_V2_R2: &str =
    "Kd2dTz60zgX2u/bIBxxzoHBJ/ZQP3HSLjzclJPUWBKT2iwCrb49m0NVbruZUEgiFpfLgrbPSq/yNi0wHbsDEAQ==";
const SIG_V2_R4: &str =
    "cmJSC0NyhP+2N/5/6XxohzvvPZJyg5t5WkyYnnQgquwPDJqINEXwTw7VVv2zby/p6S6ezWkh3OEwzfNeWEWZBw==";
/// `bosun-approve v2 acme/widget#1 <OLD_HEAD> after-review 2`.
const SIG_V2_OLD_HEAD: &str =
    "kZrauGSNziVxBKtBxWEu92JllOOr6+uo9qJ6uCBtUmieerOohkuBd98LaRa66zAQJMOVBFkuTn826cj+rgEvDg==";
/// The same key over the v2 message for `acme/widget#2` and for `acme/other#1` (review 2).
const SIG_V2_OTHER_PR: &str =
    "4+zgKwTC+GGWZXUZDANk+TmTV4BZWfcA8FIuZjsG17TGgLnWgjRc2iEpMG56ZK8fpQ0Hl210WzMNeZw4sSQSAA==";
const SIG_V2_OTHER_REPO: &str =
    "64dss7h/7TbXioRrhyERtHMWNKPCmzFzJnYh5i6r43QA82+7SRcH9PMzPIgfaeyYCkkXM3KaZsfuQro8HSGoBQ==";
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

/// A v2 signature comment of key `k1` through the app.
fn v2(fake: &FakeGithub, head: &str, after: &str, sig: &str, at: &str) {
    let body = format!("/bosun approve {head} after-review {after}\nbosun-sig ed25519 k1 {sig}");
    fake.comment_at(1, "claude", &body, at);
    fake.last_comment(|c| c.via_app = Some("claude".into()));
}

/// A second request for changes (review 3) that was dismissed too (review 4).
fn dismissed_again(fake: &FakeGithub) {
    fake.review(1, "ada", "CHANGES_REQUESTED", HEAD);
    fake.review(1, "ada", "DISMISSED", HEAD);
    fake.dismiss_event(1, 4, "2026-10-22T10:45:00Z");
}

/// A second owner `bob` whose request for changes (review 3) was dismissed too (review 4).
fn bob_dismissed(fake: &FakeGithub) {
    let owners = "[[owner]]\ngithub = \"ada\"\n[[owner]]\ngithub = \"bob\"\n";
    fake.content("governance/owners.toml", "base0000", owners);
    dismissed(fake, true);
    fake.review(1, "bob", "CHANGES_REQUESTED", HEAD);
    fake.review(1, "bob", "DISMISSED", HEAD);
    fake.dismiss_event(1, 4, "2026-10-22T10:45:00Z");
}

type Setup = fn(&FakeGithub);

#[test]
fn nothing_but_a_newer_review_or_plain_comment_ends_a_dismissal() {
    let cases: [(&str, Setup); 19] = [
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
        ("v2 stale review id", |f| {
            dismissed(f, true);
            v2(f, HEAD, "2", SIG_V2_R2, BETWEEN);
            dismissed_again(f);
        }),
        ("v2 replayed after a second dismissal", |f| {
            dismissed(f, true);
            v2(f, HEAD, "2", SIG_V2_R2, OLDER);
            dismissed_again(f);
            v2(f, HEAD, "2", SIG_V2_R2, NEWER);
        }),
        ("v2 for another head", |f| {
            dismissed(f, true);
            v2(f, OLD_HEAD, "2", SIG_V2_OLD_HEAD, NEWER);
        }),
        ("v2 for another PR number", |f| {
            dismissed(f, true);
            v2(f, HEAD, "2", SIG_V2_OTHER_PR, NEWER);
        }),
        ("v2 for another repo", |f| {
            dismissed(f, true);
            v2(f, HEAD, "2", SIG_V2_OTHER_REPO, NEWER);
        }),
        ("v2 with an unparseable id", |f| {
            dismissed(f, true);
            v2(f, HEAD, "02", SIG_V2_R2, NEWER);
            v2(f, HEAD, "x", SIG_V2_R2, NEWER);
        }),
        ("v2 with an unreadable timeline", |f| {
            dismissed(f, false);
            v2(f, HEAD, "2", SIG_V2_R2, NEWER);
        }),
        ("v2 under a plain request for changes", |f| {
            f.review(1, "ada", "CHANGES_REQUESTED", HEAD);
            v2(f, HEAD, "1", SIG_V2_R2, NEWER);
        }),
        ("v2 for one of two dismissed owners", |f| {
            bob_dismissed(f);
            v2(f, HEAD, "2", SIG_V2_R2, NEWER);
        }),
        ("v2 by a delegate key", |f| {
            let keys = format!("[[key]]\nid = \"k1\"\ned25519 = \"{PUB}\"\nrole = \"delegate\"\n");
            f.content(".bosun/approvers.toml", "refs/heads/main", &keys);
            dismissed(f, true);
            v2(f, HEAD, "2", SIG_V2_R2, NEWER);
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
    let cases: [(&str, &str, Setup); 7] = [
        ("ada", "newer APPROVED review", |f| {
            dismissed(f, true);
            f.review(1, "ada", "APPROVED", HEAD);
        }),
        ("ada", "comment after the dismissal", |f| {
            dismissed(f, true);
            owner_says(f, NEWER);
        }),
        ("key k1", "signature, no dismissal", |f| signed(f, NEWER)),
        ("key k1", "v2 naming the dismissed review", |f| {
            dismissed(f, true);
            v2(f, HEAD, "2", SIG_V2_R2, NEWER);
        }),
        ("key k1", "v2 naming the second dismissal", |f| {
            dismissed(f, true);
            dismissed_again(f);
            v2(f, HEAD, "4", SIG_V2_R4, NEWER);
        }),
        ("key k1", "v2 for each of two dismissed owners", |f| {
            bob_dismissed(f);
            v2(f, HEAD, "2", SIG_V2_R2, NEWER);
            v2(f, HEAD, "4", SIG_V2_R4, NEWER);
        }),
        ("key k1", "v2, no dismissal", |f| {
            v2(f, HEAD, "2", SIG_V2_R2, NEWER)
        }),
    ];
    for (who, name, setup) in cases {
        let fake = high_pr();
        setup(&fake);
        let (code, v) = check(&fake);
        assert_eq!(code, 0, "{name}: {v}");
        assert_eq!(v["data"]["approved_by"], who, "{name}");
    }
}

fn sign_cmd(fake: &FakeGithub, print: bool) -> (i32, Value) {
    let key = tempfile::tempdir().unwrap().keep().join("key.pem");
    std::fs::write(&key, PEM).unwrap();
    let out = bosun()
        .env("BOSUN_GITHUB_API", fake.url())
        .args(["approve", &format!("1@{HEAD}")])
        .args(print.then_some("--print"))
        .arg("--sign-key")
        .arg(&key)
        .output()
        .unwrap();
    (out.status.code().unwrap(), one_object(&out.stdout))
}

#[test]
fn sign_key_signs_v2_after_a_dismissal_v1_normally_and_refuses_changes() {
    let fake = high_pr();
    fake.review(1, "ada", "CHANGES_REQUESTED", HEAD);
    let (code, v) = sign_cmd(&fake, true);
    assert_eq!(code, 3, "{v}");
    assert!(v["data"]["body"].is_null(), "nothing was signed: {v}");
    let fake = high_pr();
    dismissed(&fake, true);
    let (code, v) = sign_cmd(&fake, true);
    let want = format!("/bosun approve {HEAD} after-review 2\nbosun-sig ed25519 k1 {SIG_V2_R2}");
    assert_eq!((code, &v["data"]["body"]), (0, &json!(want)), "{v}");
    // Posting it clears the dismissal.
    let (code, v) = sign_cmd(&fake, false);
    assert_eq!(
        (code, &v["data"]["approved"][0]["after_review"]),
        (0, &json!(2))
    );
    let (code, v) = check(&fake);
    assert_eq!(
        (code, &v["data"]["approved_by"]),
        (0, &json!("key k1")),
        "{v}"
    );
    let (code, v) = sign_cmd(&high_pr(), true);
    let want = format!("/bosun approve {HEAD}\nbosun-sig ed25519 k1 {SIG}");
    assert_eq!((code, &v["data"]["body"]), (0, &json!(want)), "{v}");
}
