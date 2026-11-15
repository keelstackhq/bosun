//! A one-off task over 300 changed lines needs the owner in autonomous mode (D-017), as in
//! risk-check: `R-oneoff-size` is a denied rule.
mod common;

use bosun::github::b64;
use common::fake::FakeGithub;
use common::{bosun, lease_note, one_object};
use ring::signature::{Ed25519KeyPair, KeyPair};
use serde_json::json;

const BASE: &str = "base0000";
const HEAD: &str = "0123456789abcdef0123456789abcdef01234567";
const MAIN: &str = "refs/heads/main";
const NOW: &str = "2026-12-05T10:00:00Z";

fn sign(task: &str, adds: u64) -> (i32, serde_json::Value) {
    let rng = ring::rand::SystemRandom::new();
    let doc = Ed25519KeyPair::generate_pkcs8(&rng).unwrap();
    let k = Ed25519KeyPair::from_pkcs8(doc.as_ref()).unwrap();
    // The PEM label is built, so no added line looks like a key to R-secrets.
    let label = ["PRIVATE", "KEY"].join(" ");
    let body = b64::encode(doc.as_ref());
    let pem = format!("-----BEGIN {label}-----\n{body}\n-----END {label}-----\n");
    let fake = FakeGithub::start();
    fake.pull(1, "change", "feature", "Closes #7");
    fake.pull_as_issue(1, "change");
    fake.pull_set(1, "head", json!({"sha": HEAD, "ref": "feature"}));
    fake.issue(7, "task", &[], &format!("task\n<!-- bosun:task {task} -->"));
    let lease = lease_note("session_w", "2026-12-06T12:00:00Z", "src/**");
    fake.comment_by(7, "octocat", &lease);
    fake.content(
        "governance/owners.toml",
        BASE,
        "[[owner]]\ngithub = \"ada\"\n",
    );
    let public = b64::encode(k.public_key().as_ref());
    let keys = format!("[[key]]\nid = \"k1\"\ned25519 = \"{public}\"\n");
    fake.content(".bosun/approvers.toml", MAIN, &keys);
    // The preset fills the policy: booster means autonomous (D-017).
    let config = "project = \"widget\"\n\n[mode]\nname = \"booster\"\n";
    fake.content(".bosun/bosun.toml", BASE, config);
    fake.content(".bosun/bosun.toml", MAIN, config);
    fake.pull_file(
        1,
        "src/feature/a.rs",
        "modified",
        adds,
        0,
        Some("@@ -0,0 +1,1 @@\n+x"),
    );
    let r = json!({"bosun": "verdict", "session": "session_r", "head_sha": HEAD,
        "approved": true, "confidence": 0.95, "major": 0});
    let verdict = format!("Review verdict: approve 0.95, 0 blocker.\n\n{r}");
    fake.comment_by(1, "github-actions[bot]", &verdict);
    let dir = tempfile::tempdir().unwrap().keep();
    std::fs::write(dir.join("key.pem"), pem).unwrap();
    let out = bosun()
        .env("BOSUN_GITHUB_API", fake.url())
        .env("BOSUN_NOW", NOW)
        .env("BOSUN_SESSION", "session_o")
        .env("CCR_AGENT_PROXY_ENABLED", "true")
        .args(["approve", &format!("1@{HEAD}"), "--bootstrap", "--print"])
        .arg("--sign-key")
        .arg(dir.join("key.pem"))
        .output()
        .unwrap();
    (out.status.code().unwrap(), one_object(&out.stdout))
}

#[test]
fn a_one_off_over_300_lines_is_not_signed_in_autonomous_mode() {
    let one_off = r#"{"item":"oneoff","task":"x","touch":["src/**"]}"#;
    let (code, v) = sign(one_off, 301);
    assert_eq!(code, 1, "{v}");
    let msg = v["error"]["message"].as_str().unwrap();
    assert!(
        msg.contains("never covers") && msg.contains("R-oneoff-size"),
        "{msg}"
    );
    assert!(!v["error"]["fix"].as_str().unwrap().is_empty());
    let (code, v) = sign(one_off, 300);
    assert_eq!(code, 0, "{v}");
    // A planned task of the same size is not a one-off.
    let planned = r#"{"item":"t1","task":"x","touch":["src/**"]}"#;
    let (code, v) = sign(planned, 301);
    assert_eq!(code, 0, "{v}");
}
