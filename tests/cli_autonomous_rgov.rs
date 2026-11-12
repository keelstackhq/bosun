//! D-017 + modes PR 1b: `approve --bootstrap` runs the base rules, so an allowlisted R-gov file
//! (`src/core/session_rules.rs`) is refused before signing, as risk-check refuses it.
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

fn sign(file: &str) -> (i32, serde_json::Value) {
    let rng = ring::rand::SystemRandom::new();
    let doc = Ed25519KeyPair::generate_pkcs8(&rng).unwrap();
    let k = Ed25519KeyPair::from_pkcs8(doc.as_ref()).unwrap();
    let pem = format!(
        "-----BEGIN PRIVATE KEY-----\n{}\n-----END PRIVATE KEY-----\n",
        b64::encode(doc.as_ref())
    );
    let fake = FakeGithub::start();
    fake.pull(1, "change", "feature", "Closes #7");
    fake.pull_as_issue(1, "change");
    fake.pull_set(1, "head", json!({"sha": HEAD, "ref": "feature"}));
    fake.issue(7, "task", &[], "task\n<!-- bosun:task {} -->");
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
    fake.pull_file(1, file, "modified", 1, 0, Some("@@ -0,0 +1,1 @@\n+x"));
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
fn an_allowlisted_r_gov_file_is_not_signed_under_a_preset_autonomous_policy() {
    let (code, v) = sign("src/core/session_rules.rs");
    assert_eq!(code, 1, "{v}");
    let msg = v["error"]["message"].as_str().unwrap();
    assert!(
        msg.contains("never covers") && msg.contains("R-gov"),
        "{msg}"
    );
    let (code, v) = sign("src/feature/a.rs");
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["policy_source"], "preset");
}
