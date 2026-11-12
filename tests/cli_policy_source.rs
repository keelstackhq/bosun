//! `data.policy_source` (spec §18): `bosun approve --bootstrap` names who set the policy.
mod common;

use bosun::github::b64;
use common::fake::FakeGithub;
use common::{bosun, one_object};
use ring::signature::{Ed25519KeyPair, KeyPair};
use serde_json::{Value, json};

const BASE: &str = "base0000";
const HEAD: &str = "0123456789abcdef0123456789abcdef01234567";
const MAIN: &str = "refs/heads/main";

fn sign(config: &str, now: &str) -> (i32, Value) {
    let rng = ring::rand::SystemRandom::new();
    let doc = Ed25519KeyPair::generate_pkcs8(&rng).unwrap();
    let k = Ed25519KeyPair::from_pkcs8(doc.as_ref()).unwrap();
    let pem = format!(
        "-----BEGIN PRIVATE KEY-----\n{}\n-----END PRIVATE KEY-----\n",
        b64::encode(doc.as_ref())
    );
    let fake = FakeGithub::start();
    fake.pull(1, "change", "feature", "");
    fake.pull_as_issue(1, "change");
    fake.pull_set(1, "head", json!({"sha": HEAD, "ref": "feature"}));
    fake.content(
        "governance/owners.toml",
        BASE,
        "[[owner]]\ngithub = \"ada\"\n",
    );
    let public = b64::encode(k.public_key().as_ref());
    let keys = format!("[[key]]\nid = \"k1\"\ned25519 = \"{public}\"\n");
    fake.content(".bosun/approvers.toml", MAIN, &keys);
    fake.content(".bosun/bosun.toml", BASE, config);
    fake.content(".bosun/bosun.toml", MAIN, config);
    let patch = "@@ -0,0 +1,1 @@\n+x";
    fake.pull_file(1, ".claude/settings.json", "modified", 1, 0, Some(patch));
    let dir = tempfile::tempdir().unwrap().keep();
    std::fs::write(dir.join("key.pem"), pem).unwrap();
    let out = bosun()
        .env("BOSUN_GITHUB_API", fake.url())
        .env("BOSUN_NOW", now)
        .env("CCR_AGENT_PROXY_ENABLED", "true")
        .args(["approve", &format!("1@{HEAD}"), "--bootstrap", "--print"])
        .arg("--sign-key")
        .arg(dir.join("key.pem"))
        .output()
        .unwrap();
    (out.status.code().unwrap(), one_object(&out.stdout))
}

#[test]
fn approve_names_the_policy_source() {
    let (code, v) = sign(
        "[approvals]\npolicy = \"orchestrator\"\n",
        "2026-12-01T00:00:00Z",
    );
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["policy_source"], "explicit");
    let (code, v) = sign(
        "[approvals]\nbootstrap_until = \"2026-11-30\"\n",
        "2026-11-10T10:00:00Z",
    );
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["policy_source"], "default");
}
