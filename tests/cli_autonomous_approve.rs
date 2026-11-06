//! `[approvals] policy = "autonomous"` (D-017): an orchestrator signature counts in risk-check
//! only with a clean independent verdict on the head, and `bosun approve --bootstrap` signs only
//! under the same conditions.
mod common;

use bosun::github::b64;
use common::fake::FakeGithub;
use common::{bosun, lease_note, one_object, run};
use ring::signature::{Ed25519KeyPair, KeyPair};
use serde_json::{Value, json};

const BASE: &str = "base0000";
const HEAD: &str = "0123456789abcdef0123456789abcdef01234567";
const OTHER: &str = "89abcdef0123456789abcdef0123456789abcdef";
const MAIN: &str = "refs/heads/main";
const NOW: &str = "2026-12-05T10:00:00Z";
const PLAIN: &str = "@@ -0,0 +1,1 @@\n+x";
const ENV_DIFF: &str = "@@ -1,1 +1,1 @@\n-    environment: release\n+x";
const WAITING: &str = "high risk, owner approval needed";
const AUTO: &str = "high risk, autonomous mode (D-017)";

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

fn config(policy: Option<&str>) -> String {
    let mut c = "project = \"widget\"\n\n[approvals]\n".to_string();
    if let Some(p) = policy {
        c.push_str(&format!("policy = \"{p}\"\n"));
    }
    c
}

fn verdict(session: &str, head: &str, major: u32) -> String {
    let r = json!({"bosun": "verdict", "session": session, "head_sha": head,
        "approved": true, "confidence": 0.95, "major": major});
    format!("Review verdict: approve 0.95, 0 blocker.\n\n{r}")
}

/// PR 1 (Closes #7, leased by `session_w`) at HEAD changing `file`; owner `ada`, key `k1`.
fn pr(k: &Key, file: &str, patch: &str, body: &str, policy: Option<&str>) -> FakeGithub {
    let fake = FakeGithub::start();
    fake.pull(1, "change", "feature", body);
    fake.pull_as_issue(1, "change");
    fake.pull_set(1, "head", json!({"sha": HEAD, "ref": "feature"}));
    fake.issue(7, "task", &[], "task\n<!-- bosun:task {} -->");
    fake.comment(
        7,
        &lease_note("session_w", "2026-12-06T12:00:00Z", "src/**"),
    );
    fake.content(
        "governance/owners.toml",
        BASE,
        "[[owner]]\ngithub = \"ada\"\n",
    );
    let public = b64::encode(k.0.public_key().as_ref());
    let keys = format!("[[key]]\nid = \"k1\"\ned25519 = \"{public}\"\n");
    fake.content(".bosun/approvers.toml", MAIN, &keys);
    fake.content(".bosun/bosun.toml", BASE, &config(policy));
    fake.content(".bosun/bosun.toml", MAIN, &config(policy));
    fake.pull_file(1, file, "modified", 1, 0, Some(patch));
    fake
}

fn auto(k: &Key, file: &str, patch: &str) -> FakeGithub {
    pr(k, file, patch, "Closes #7", Some("autonomous"))
}

fn post_sig(fake: &FakeGithub, k: &Key) {
    let msg = format!("bosun-bootstrap v1 acme/widget#1 {HEAD}");
    let s = b64::encode(k.0.sign(msg.as_bytes()).as_ref());
    fake.comment_by(
        1,
        "claude",
        &format!("/bosun approve {HEAD} bootstrap\nbosun-sig ed25519 k1 {s}"),
    );
    fake.last_comment(|c| c.via_app = Some("claude".into()));
}

fn post_verdict(fake: &FakeGithub, session: &str, head: &str, major: u32) {
    fake.comment_by(1, "github-actions[bot]", &verdict(session, head, major));
}

fn check(fake: &FakeGithub) -> (i32, Value) {
    run(fake, "session_test", NOW, &["risk-check", "1"])
}

fn description(fake: &FakeGithub) -> String {
    let s = fake.statuses_of(HEAD);
    let s = s
        .iter()
        .rev()
        .find(|s| s["context"] == "risk-check")
        .unwrap();
    s["description"].as_str().unwrap().to_string()
}

/// Signature plus one verdict (`session`, `head`, `major`) on `file`.
fn with(file: &str, patch: &str, session: &str, head: &str, major: u32) -> (i32, Value, String) {
    let k = key();
    let fake = auto(&k, file, patch);
    post_verdict(&fake, session, head, major);
    post_sig(&fake, &k);
    let (code, v) = check(&fake);
    (code, v, description(&fake))
}

#[test]
fn a_signature_with_a_clean_independent_verdict_counts() {
    let (code, v, _) = with("deny.toml", PLAIN, "session_r", HEAD, 0);
    assert_eq!(code, 0, "{v}");
    let by = v["data"]["approved_by"].as_str().unwrap();
    assert!(
        by.starts_with("key k1 (") && by.contains("autonomous mode, D-017"),
        "{by}"
    );
    assert_eq!(v["data"]["policy"], "autonomous");
    assert_eq!(v["data"]["autonomous_verdicts"], json!(["session_r"]));
}

#[test]
fn a_workflow_change_without_an_environment_counts() {
    let (code, v, _) = with(".github/workflows/ci.yml", PLAIN, "session_r", HEAD, 0);
    assert_eq!(code, 0, "{v}");
}

#[test]
fn a_signature_alone_or_a_verdict_alone_does_not_count() {
    let k = key();
    let fake = auto(&k, "deny.toml", PLAIN);
    post_sig(&fake, &k);
    let (code, v) = check(&fake);
    assert_eq!(code, 5, "{v}");
    let d = description(&fake);
    assert!(d.starts_with(AUTO) && !d.starts_with(WAITING), "{d}");
    assert_eq!(v["data"]["policy"], "autonomous");

    let fake = auto(&k, "deny.toml", PLAIN);
    post_verdict(&fake, "session_r", HEAD, 0);
    let (code, v) = check(&fake);
    assert_eq!(code, 5, "{v}");
    assert!(description(&fake).starts_with(AUTO));
}

#[test]
fn the_verdict_must_be_independent_clean_and_on_the_head() {
    for (session, head, major) in [
        ("session_w", HEAD, 0),
        ("session_r", OTHER, 0),
        ("session_r", HEAD, 1),
    ] {
        let (code, v, d) = with("deny.toml", PLAIN, session, head, major);
        assert_eq!(code, 5, "{session} {head} {major}: {v}");
        assert!(d.starts_with(AUTO), "{d}");
    }
}

#[test]
fn an_unlinked_pr_gets_no_autonomous_approval() {
    let k = key();
    let fake = pr(&k, "deny.toml", PLAIN, "", Some("autonomous"));
    post_verdict(&fake, "session_r", HEAD, 0);
    post_sig(&fake, &k);
    let (code, v) = check(&fake);
    assert_eq!(code, 5, "{v}");
    assert!(description(&fake).starts_with(WAITING));
}

#[test]
fn releases_secrets_environments_and_trust_files_stay_with_the_owner() {
    for (file, patch) in [
        (".github/workflows/release.yml", PLAIN),
        (".github/workflows/ci.yml", ENV_DIFF),
        ("hoist.toml", PLAIN),
        (".bosun/approvers.toml", PLAIN),
    ] {
        let (code, v, d) = with(file, patch, "session_r", HEAD, 0);
        assert_eq!(code, 5, "{file}: {v}");
        assert!(d.starts_with(WAITING), "{file}: {d}");
    }
}

#[test]
fn other_policies_are_unchanged() {
    for policy in [None, Some("owner-gated")] {
        let k = key();
        let fake = pr(&k, ".github/workflows/ci.yml", PLAIN, "Closes #7", policy);
        post_verdict(&fake, "session_r", HEAD, 0);
        post_sig(&fake, &k);
        let (code, v) = check(&fake);
        assert_eq!(code, 5, "{policy:?}: {v}");
        assert!(description(&fake).starts_with(WAITING));
        assert!(v["data"].get("autonomous_verdicts").is_none());
    }
}

fn sign(fake: &FakeGithub, k: &Key, session: &str) -> (i32, Value) {
    let dir = tempfile::tempdir().unwrap().keep();
    let path = dir.join("key.pem");
    std::fs::write(&path, &k.1).unwrap();
    let out = bosun()
        .env("BOSUN_GITHUB_API", fake.url())
        .env("BOSUN_NOW", NOW)
        .env("BOSUN_SESSION", session)
        .env("CCR_AGENT_PROXY_ENABLED", "true")
        .args(["approve", &format!("1@{HEAD}"), "--bootstrap", "--print"])
        .arg("--sign-key")
        .arg(&path)
        .output()
        .unwrap();
    let v = one_object(&out.stdout);
    if v["ok"] == false {
        assert!(!v["error"]["fix"].as_str().unwrap().trim().is_empty());
    }
    (out.status.code().unwrap(), v)
}

#[test]
fn approve_signs_with_the_autonomous_line_after_a_clean_verdict() {
    let k = key();
    let fake = auto(&k, ".github/workflows/ci.yml", PLAIN);
    post_verdict(&fake, "session_r", HEAD, 0);
    let (code, v) = sign(&fake, &k, "session_o");
    assert_eq!(code, 0, "{v}");
    let body = v["data"]["body"].as_str().unwrap();
    assert!(
        body.contains("Signed in autonomous mode (D-017), not an owner answer"),
        "{body}"
    );
    assert!(body.starts_with(&format!("/bosun approve {HEAD} bootstrap\n")));
    assert_eq!(v["data"]["policy"], "autonomous");
}

#[test]
fn approve_refuses_without_an_independent_clean_verdict() {
    let k = key();
    for (session, head, major) in [
        ("session_o", HEAD, 0), // the signer's own
        ("session_w", HEAD, 0), // the author's lease
        ("session_r", OTHER, 0),
        ("session_r", HEAD, 2),
    ] {
        let fake = auto(&k, "deny.toml", PLAIN);
        post_verdict(&fake, session, head, major);
        let (code, v) = sign(&fake, &k, "session_o");
        assert_eq!(code, 1, "{session} {head} {major}: {v}");
    }
    let fake = auto(&k, "deny.toml", PLAIN);
    assert_eq!(sign(&fake, &k, "session_o").0, 1);
    // No linked task: no lease to check the reviewer against.
    let fake = pr(&k, "deny.toml", PLAIN, "", Some("autonomous"));
    post_verdict(&fake, "session_r", HEAD, 0);
    assert_eq!(sign(&fake, &k, "session_o").0, 1);
}

#[test]
fn approve_refuses_excluded_paths_even_with_a_verdict() {
    let k = key();
    for (file, patch) in [
        (".github/workflows/release.yml", PLAIN),
        (".github/workflows/ci.yml", ENV_DIFF),
        ("hoist.toml", PLAIN),
    ] {
        let fake = auto(&k, file, patch);
        post_verdict(&fake, "session_r", HEAD, 0);
        let (code, v) = sign(&fake, &k, "session_o");
        assert_eq!(code, 1, "{file}: {v}");
    }
}

const SECRET_DIFF: &str = "@@ -0,0 +1,1 @@\n+API_KEY = abc";

/// A signature and a clean verdict by `session_r` on a PR that links `body`'s issues.
fn decoy(k: &Key, body: &str) -> FakeGithub {
    let fake = pr(k, "deny.toml", PLAIN, body, Some("autonomous"));
    fake.issue(8, "decoy", &[], "no task marker");
    fake.issue(9, "unleased", &[], "<!-- bosun:task {} -->");
    post_verdict(&fake, "session_r", HEAD, 0);
    fake
}

#[test]
fn a_decoy_link_leaves_the_owner_path_and_approve_refuses() {
    // The task and its lease must be the same issue: a lease on a non-task decoy (#8) does not
    // stand in for the missing lease on the task (#9).
    for body in ["Closes #8", "Closes #9", "Closes #8\nCloses #9"] {
        let k = key();
        let fake = decoy(&k, body);
        let note = lease_note("session_x", "2026-12-06T12:00:00Z", "src/**");
        fake.comment(8, &note);
        post_sig(&fake, &k);
        let (code, v) = check(&fake);
        assert_eq!(code, 5, "{body}: {v}");
        assert!(description(&fake).starts_with(WAITING), "{body}");
        assert!(v["data"].get("autonomous_verdicts").is_none());
        let fake = decoy(&k, body);
        fake.comment(8, &note);
        let (code, v) = sign(&fake, &k, "session_o");
        assert_eq!(code, 1, "{body}: {v}");
    }
}

#[test]
fn an_unchanged_environment_line_in_a_workflow_stays_with_the_owner() {
    // Quoted keys and a space before the colon are valid YAML too.
    let ymls = [
        "jobs:\n  x:\n    environment: prod\n    steps: []\n",
        "jobs:\n  x:\n    \"environment\": prod\n",
        "jobs:\n  x:\n    'Environment' : prod\n",
        "jobs: {x: {environment : prod}}\n",
    ];
    for (r, yml) in [HEAD, BASE].into_iter().flat_map(|r| ymls.map(|y| (r, y))) {
        let k = key();
        let fake = auto(&k, ".github/workflows/ci.yml", PLAIN);
        fake.content(".github/workflows/ci.yml", r, yml);
        post_verdict(&fake, "session_r", HEAD, 0);
        post_sig(&fake, &k);
        let (code, v) = check(&fake);
        assert_eq!(code, 5, "{r}: {v}");
        assert!(description(&fake).starts_with(WAITING), "{r}");
        let (code, v) = sign(&fake, &k, "session_o");
        assert_eq!(code, 1, "{r}: {v}");
    }
}

#[test]
fn a_secret_match_keeps_the_owner_path() {
    let (code, v, d) = with("a.env", SECRET_DIFF, "session_r", HEAD, 0);
    assert_eq!(code, 5, "{v}");
    assert!(d.starts_with(WAITING), "{d}");
}
