//! Bootstrap-mode approvals (D-012, spec §8): `bosun approve --sign-key <pem> --bootstrap` signs
//! `bosun-bootstrap v1`, which risk-check counts as the owner only inside `[approvals]
//! bootstrap_until`, on an owner-class PR that touches no excluded path.
mod common;

use bosun::github::b64;
use common::fake::FakeGithub;
use common::{bosun, one_object, run};
use ring::signature::{Ed25519KeyPair, KeyPair};
use serde_json::{Value, json};

const BASE: &str = "base0000";
const HEAD: &str = "0123456789abcdef0123456789abcdef01234567";
const MAIN: &str = "refs/heads/main";
const IN: &str = "2026-11-10T10:00:00Z";
const LAST: &str = "2026-11-30T23:59:59Z";
const AFTER: &str = "2026-12-01T00:00:00Z";
const PLAIN: &str = "@@ -0,0 +1,1 @@\n+x";
const CONFIG: &str = "project = \"widget\"\n\n[approvals]\nbootstrap_until = \"2026-11-30\"\n";

/// A key generated for this test run (never a real one) and its PKCS#8 PEM.
struct Key(Ed25519KeyPair, String);

fn key() -> Key {
    let rng = ring::rand::SystemRandom::new();
    let doc = Ed25519KeyPair::generate_pkcs8(&rng).unwrap();
    let pem = format!(
        "-----BEGIN PRIVATE KEY-----\n{}\n-----END PRIVATE KEY-----\n",
        b64::encode(doc.as_ref())
    );
    Key(Ed25519KeyPair::from_pkcs8(doc.as_ref()).unwrap(), pem)
}

fn public(k: &Key) -> String {
    b64::encode(k.0.public_key().as_ref())
}

fn sig(k: &Key, msg: &str) -> String {
    b64::encode(k.0.sign(msg.as_bytes()).as_ref())
}

/// Open PR 1 at HEAD changing `file`, owner `ada`, key `k1` enrolled on main, `config` at base
/// and on main.
fn pr(k: &Key, file: &str, patch: &str, config: Option<&str>) -> FakeGithub {
    let fake = FakeGithub::start();
    fake.pull(1, "change", "feature", "");
    fake.pull_as_issue(1, "change");
    fake.pull_set(1, "head", json!({"sha": HEAD, "ref": "feature"}));
    fake.content(
        "governance/owners.toml",
        BASE,
        "[[owner]]\ngithub = \"ada\"\n",
    );
    let keys = format!("[[key]]\nid = \"k1\"\ned25519 = \"{}\"\n", public(k));
    fake.content(".bosun/approvers.toml", MAIN, &keys);
    if let Some(c) = config {
        fake.content(".bosun/bosun.toml", BASE, c);
        fake.content(".bosun/bosun.toml", MAIN, c);
    }
    fake.pull_file(1, file, "modified", 1, 0, Some(patch));
    fake
}

/// `.claude/**` is R-gov: the owner class, no excluded path.
fn owner_pr(k: &Key, config: Option<&str>) -> FakeGithub {
    pr(k, ".claude/settings.json", PLAIN, config)
}

/// `bosun approve 1@HEAD --sign-key <pem> --bootstrap --print` at `now`.
fn sign(fake: &FakeGithub, k: &Key, now: &str) -> (i32, Value) {
    let dir = tempfile::tempdir().unwrap().keep();
    let path = dir.join("key.pem");
    std::fs::write(&path, &k.1).unwrap();
    let out = bosun()
        .env("BOSUN_GITHUB_API", fake.url())
        .env("BOSUN_NOW", now)
        .env("CCR_AGENT_PROXY_ENABLED", "true")
        .args(["approve", &format!("1@{HEAD}"), "--bootstrap", "--print"])
        .arg("--sign-key")
        .arg(&path)
        .output()
        .unwrap();
    (out.status.code().unwrap(), one_object(&out.stdout))
}

fn post(fake: &FakeGithub, body: &str) {
    fake.comment_by(1, "claude", body);
    fake.last_comment(|c| c.via_app = Some("claude".into()));
}

fn check(fake: &FakeGithub, now: &str) -> (i32, Value) {
    run(fake, "session_test", now, &["risk-check", "1"])
}

fn bootstrap_body(k: &Key) -> String {
    let s = sig(k, &format!("bosun-bootstrap v1 acme/widget#1 {HEAD}"));
    format!("/bosun approve {HEAD} bootstrap\nbosun-sig ed25519 k1 {s}")
}

#[test]
fn a_bootstrap_signature_counts_inside_the_window_through_the_last_day() {
    for now in [IN, LAST] {
        let k = key();
        let fake = owner_pr(&k, Some(CONFIG));
        let (code, v) = sign(&fake, &k, now);
        assert_eq!(code, 0, "{v}");
        let body = v["data"]["body"].as_str().unwrap();
        assert!(body.starts_with(&format!("/bosun approve {HEAD} bootstrap\n")));
        assert!(body.contains("\nSigned in bootstrap mode (D-012, until 2026-11-30)\n"));
        post(&fake, body);
        let (code, v) = check(&fake, now);
        assert_eq!(code, 0, "{v}");
        assert_eq!(v["data"]["approved_by"], "key k1 (bootstrap mode, D-012)");
        let s = fake.statuses_of(HEAD).pop().unwrap();
        assert_eq!(
            (&s["context"], &s["state"]),
            (&json!("risk-check"), &json!("success"))
        );
        assert!(s["description"].to_string().contains("bootstrap mode"));
    }
}

#[test]
fn after_the_window_or_without_the_setting_it_does_not_count() {
    let k = key();
    let fake = owner_pr(&k, Some(CONFIG));
    post(&fake, &bootstrap_body(&k));
    let (code, v) = check(&fake, AFTER);
    assert_eq!(code, 5, "{v}");
    let text = v["error"].to_string();
    assert!(
        text.contains("D-012") && text.contains("2026-11-30"),
        "{text}"
    );
    // The signer refuses too.
    assert_eq!(sign(&fake, &k, AFTER).0, 1);

    for config in [None, Some("project = \"widget\"\n")] {
        let fake = owner_pr(&k, config);
        post(&fake, &bootstrap_body(&k));
        let (code, v) = check(&fake, IN);
        assert_eq!(code, 5, "{v}");
        let fix = v["error"]["fix"].to_string();
        assert!(
            fix.contains("D-012") && fix.contains("bootstrap_until"),
            "{fix}"
        );
        let (code, v) = sign(&fake, &k, IN);
        assert_eq!(code, 1, "{v}");
        assert!(v["error"]["fix"].to_string().contains("D-012"));
    }
}

#[test]
fn an_invalid_setting_is_a_parse_error_naming_the_key() {
    let k = key();
    let bad = "[approvals]\nbootstrap_until = \"2026-11-31\"\n";
    let fake = owner_pr(&k, Some(bad));
    let (code, v) = sign(&fake, &k, IN);
    assert_eq!(code, 1, "{v}");
    assert!(
        v["error"]["message"]
            .to_string()
            .contains("bootstrap_until")
    );
    post(&fake, &bootstrap_body(&k));
    assert_eq!(check(&fake, IN).0, 5);
}

#[test]
fn forged_markers_do_not_count() {
    let k = key();
    let other = key();
    let fake = owner_pr(&k, Some(CONFIG));
    let v1 = sig(&k, &format!("bosun-approve v1 acme/widget#1 {HEAD}"));
    let v2 = sig(
        &k,
        &format!("bosun-approve v2 acme/widget#1 {HEAD} after-review 7"),
    );
    let boot = sig(&k, &format!("bosun-bootstrap v1 acme/widget#1 {HEAD}"));
    let unknown = sig(&other, &format!("bosun-bootstrap v1 acme/widget#1 {HEAD}"));
    let line = "Signed in bootstrap mode (D-012, until 2026-11-30)";
    for b in [
        format!("/bosun approve {HEAD} bootstrap\n{line}\nbosun-sig ed25519 k1 {v1}"),
        format!("/bosun approve {HEAD} bootstrap\nbosun-sig ed25519 k1 {v2}"),
        format!("/bosun approve {HEAD} after-review 7 bootstrap\nbosun-sig ed25519 k1 {v2}"),
        format!("/bosun approve {HEAD}\n{line}\nbosun-sig ed25519 k1 {boot}"),
        format!("/bosun approve {HEAD} after-review 7\nbosun-sig ed25519 k1 {boot}"),
        format!("/bosun approve {HEAD} bootstrap\n{line}"),
        format!("/bosun approve {HEAD} bootstrap\nbosun-sig ed25519 k2 {unknown}"),
        format!("/bosun approve {HEAD} bootstrap\nbosun-sig ed25519 k1 {unknown}"),
    ] {
        post(&fake, &b);
    }
    let (code, v) = check(&fake, IN);
    assert_eq!(code, 5, "{v}");
    assert_eq!(v["data"]["approved_by"], Value::Null);
}

#[test]
fn a_bootstrap_signature_never_counts_outside_the_owner_class() {
    let k = key();
    // `channel/**` is R-ci-review, two verdicts.
    let fake = pr(&k, "channel/x.md", PLAIN, Some(CONFIG));
    post(&fake, &bootstrap_body(&k));
    let (code, v) = check(&fake, IN);
    assert_eq!(code, 5, "{v}");
    assert_eq!(v["data"]["requirement"], "two-verdicts");
    assert!(v["error"]["fix"].to_string().contains("D-012"));
}

#[test]
fn excluded_paths_always_need_the_owner() {
    for (file, patch) in [
        (".bosun/bosun.toml", PLAIN),
        (".bosun/risk.toml", PLAIN),
        ("governance/owners.md", PLAIN),
        ("hoist.toml", PLAIN),
        ("src/cli/risk_check.rs", PLAIN),
        ("src/core/approve.rs", PLAIN),
        (".github/workflows/release.yml", PLAIN),
        (
            ".github/workflows/ci.yml",
            "@@ -1,1 +1,1 @@\n-    environment: release\n+x",
        ),
        ("config/secrets.toml", PLAIN),
    ] {
        let k = key();
        let fake = pr(&k, file, patch, Some(CONFIG));
        post(&fake, &bootstrap_body(&k));
        let (code, v) = check(&fake, IN);
        assert_eq!(code, 5, "{file}: {v}");
        assert!(
            v["error"]["fix"].to_string().contains("D-012"),
            "{file}: {v}"
        );
        // A plain owner signature still counts where signatures do.
        let s = sig(&k, &format!("bosun-approve v1 acme/widget#1 {HEAD}"));
        post(
            &fake,
            &format!("/bosun approve {HEAD}\nbosun-sig ed25519 k1 {s}"),
        );
        assert_eq!(check(&fake, IN).0, 0, "{file}");
    }
}
