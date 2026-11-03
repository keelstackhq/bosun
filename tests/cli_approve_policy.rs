//! `[approvals] policy` (D-014, spec B6): when an orchestrator approval (the bootstrap-form
//! signature) counts in risk-check and when `bosun approve --bootstrap` signs.
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
const AFTER: &str = "2026-12-01T00:00:00Z";
const PLAIN: &str = "@@ -0,0 +1,1 @@\n+x";
const ENV_DIFF: &str = "@@ -1,1 +1,1 @@\n-    environment: release\n+x";
const POLICIES: [&str; 3] = ["owner", "owner-gated", "orchestrator"];

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

/// `[approvals]` with an optional policy and an optional `bootstrap_until`.
fn config(policy: Option<&str>, until: Option<&str>) -> String {
    let mut c = "project = \"widget\"\n\n[approvals]\n".to_string();
    for (k, v) in [("policy", policy), ("bootstrap_until", until)] {
        if let Some(v) = v {
            c.push_str(&format!("{k} = \"{v}\"\n"));
        }
    }
    c
}

/// Open PR 1 at HEAD changing `file`, owner `ada`, key `k1` enrolled, `config` at base and main.
fn pr(k: &Key, file: &str, patch: &str, config: &str) -> FakeGithub {
    let fake = FakeGithub::start();
    fake.pull(1, "change", "feature", "");
    fake.pull_as_issue(1, "change");
    fake.pull_set(1, "head", json!({"sha": HEAD, "ref": "feature"}));
    fake.content(
        "governance/owners.toml",
        BASE,
        "[[owner]]\ngithub = \"ada\"\n",
    );
    let public = b64::encode(k.0.public_key().as_ref());
    let keys = format!("[[key]]\nid = \"k1\"\ned25519 = \"{public}\"\n");
    fake.content(".bosun/approvers.toml", MAIN, &keys);
    fake.content(".bosun/bosun.toml", BASE, config);
    fake.content(".bosun/bosun.toml", MAIN, config);
    fake.pull_file(1, file, "modified", 1, 0, Some(patch));
    fake
}

fn post(fake: &FakeGithub, k: &Key) {
    let msg = format!("bosun-bootstrap v1 acme/widget#1 {HEAD}");
    let s = b64::encode(k.0.sign(msg.as_bytes()).as_ref());
    fake.comment_by(
        1,
        "claude",
        &format!("/bosun approve {HEAD} bootstrap\nbosun-sig ed25519 k1 {s}"),
    );
    fake.last_comment(|c| c.via_app = Some("claude".into()));
}

/// Whether an orchestrator signature counts for `file` under `config` at `now`.
fn counts(file: &str, patch: &str, config: &str, now: &str) -> bool {
    let k = key();
    counts_on(&pr(&k, file, patch, config), &k, now)
}

fn counts_on(fake: &FakeGithub, k: &Key, now: &str) -> bool {
    post(fake, k);
    let (code, v) = run(fake, "session_test", now, &["risk-check", "1"]);
    assert!(code == 0 || code == 5, "{v}");
    code == 0
        && v["data"]["approved_by"]
            .as_str()
            .is_some_and(|b| b.starts_with("key k1 ("))
}

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

#[test]
fn owner_gated_counts_on_r_ci_outside_workflows_only() {
    for cfg in [config(Some("owner-gated"), None)] {
        for file in ["deny.toml", "rust-toolchain.toml"] {
            assert!(counts(file, PLAIN, &cfg, AFTER), "{file}");
        }
        assert!(!counts(".claude/settings.json", PLAIN, &cfg, AFTER));
        for file in [
            ".github/workflows/ci.yml",
            ".gitlab-ci.yml",
            "ci/Jenkinsfile",
            ".drone.yml",
        ] {
            assert!(!counts(file, PLAIN, &cfg, AFTER), "{file}");
        }
    }
}

#[test]
fn policy_owner_ends_with_the_window() {
    for file in ["deny.toml", ".claude/settings.json"] {
        let cfg = config(Some("owner"), Some("2026-11-30"));
        assert!(counts(file, PLAIN, &cfg, IN), "{file} in the window");
        assert!(!counts(file, PLAIN, &cfg, AFTER), "{file} after it");
    }
    // The window is capped at D-012's last day.
    let cfg = config(Some("owner"), Some("2027-06-30"));
    assert!(counts("deny.toml", PLAIN, &cfg, "2026-11-30T12:00:00Z"));
    assert!(!counts("deny.toml", PLAIN, &cfg, AFTER));
}

#[test]
fn environment_release_branch_rule_and_secret_paths_always_need_the_owner() {
    for policy in POLICIES {
        for now in [IN, AFTER] {
            let cfg = config(Some(policy), Some("2026-11-30"));
            for (file, patch) in [
                (".github/workflows/ci.yml", ENV_DIFF),
                (".github/workflows/release.yml", PLAIN),
                ("hoist.toml", PLAIN),
                ("config/secrets.toml", PLAIN),
                (".bosun/bosun.toml", PLAIN),
            ] {
                assert!(!counts(file, patch, &cfg, now), "{policy} {now} {file}");
            }
        }
    }
}

#[test]
fn policy_orchestrator_counts_on_gov_but_never_on_workflows() {
    let cfg = config(Some("orchestrator"), None);
    assert!(counts(".claude/settings.json", PLAIN, &cfg, AFTER));
    // A workflow PR is owner-class in every policy, in the window too: bosun cannot tell its
    // permissions, environments or publish steps reliably (fail closed).
    for policy in POLICIES {
        for (cfg, now) in [
            (config(Some(policy), Some("2026-11-30")), IN),
            (config(Some(policy), None), AFTER),
        ] {
            for file in [
                ".github/workflows/ci.yml",
                ".github/actions/setup/action.yml",
                ".gitlab-ci.yml",
            ] {
                assert!(!counts(file, PLAIN, &cfg, now), "{policy} {now} {file}");
            }
        }
    }
}

#[test]
fn a_missing_policy_means_owner() {
    // Without `policy`, after the window nothing counts, R-ci-only PRs included.
    for cfg in [config(None, None), config(None, Some("2026-11-30"))] {
        assert!(!counts("deny.toml", PLAIN, &cfg, AFTER));
    }
    // Inside the window D-012 still holds.
    assert!(counts(
        "deny.toml",
        PLAIN,
        &config(None, Some("2026-11-30")),
        IN
    ));
}

#[test]
fn hoist_toml_at_any_depth_needs_the_owner() {
    for policy in POLICIES {
        let cfg = config(Some(policy), Some("2026-11-30"));
        assert!(!counts("infra/hoist.toml", PLAIN, &cfg, IN), "{policy}");
    }
}

#[test]
fn an_unknown_policy_is_a_parse_error_and_counts_nothing() {
    let cfg = config(Some("nobody"), Some("2026-11-30"));
    let k = key();
    let fake = pr(&k, "deny.toml", PLAIN, &cfg);
    let (code, v) = sign(&fake, &k, IN);
    assert_eq!(code, 1, "{v}");
    assert!(!counts("deny.toml", PLAIN, &cfg, IN));
}

#[test]
fn approve_bootstrap_signs_by_policy_after_the_window() {
    let k = key();
    let line = |v: &Value| v["data"]["body"].as_str().unwrap().to_string();
    // owner: refuses, naming D-012 and the policy.
    let fake = pr(
        &k,
        "deny.toml",
        PLAIN,
        &config(Some("owner"), Some("2026-11-30")),
    );
    let (code, v) = sign(&fake, &k, AFTER);
    assert_eq!(code, 1, "{v}");
    let text = v["error"].to_string();
    assert!(text.contains("D-012") && text.contains("owner"), "{text}");
    // orchestrator: signs with the policy note and data.policy.
    let fake = pr(
        &k,
        ".claude/settings.json",
        PLAIN,
        &config(Some("orchestrator"), None),
    );
    let (code, v) = sign(&fake, &k, AFTER);
    assert_eq!(code, 0, "{v}");
    assert!(line(&v).contains("\nSigned under approval policy orchestrator (D-014)\n"));
    assert_eq!(v["data"]["policy"], "orchestrator");
    assert_eq!(v["data"]["bootstrap_until"], Value::Null);
    // owner-gated: only when the trusted risk-check lists R-ci alone.
    for (ids, code) in [("R-ci", 0), ("R-gov", 1), ("R-ci, R-gov", 1)] {
        let fake = pr(&k, "deny.toml", PLAIN, &config(Some("owner-gated"), None));
        let text = format!("high risk, owner approval needed: {ids}");
        fake.status(HEAD, "risk-check", "failure", &text);
        let (c, v) = sign(&fake, &k, AFTER);
        assert_eq!(c, code, "{ids}: {v}");
        if code == 0 {
            assert!(line(&v).contains("Signed under approval policy owner-gated (D-014)"));
            assert_eq!(v["data"]["policy"], "owner-gated");
        } else {
            assert!(v["error"]["fix"].to_string().contains("D-012"));
        }
    }
    // An untrusted creator's status does not count.
    let fake = pr(&k, "deny.toml", PLAIN, &config(Some("owner-gated"), None));
    fake.status_by(
        HEAD,
        "risk-check",
        "failure",
        "high risk, owner approval needed: R-ci",
        "mallory",
    );
    assert_eq!(sign(&fake, &k, AFTER).0, 1);
    // A workflow PR is refused even when the status says R-ci.
    let fake = pr(
        &k,
        ".github/workflows/ci.yml",
        PLAIN,
        &config(Some("owner-gated"), None),
    );
    fake.status(
        HEAD,
        "risk-check",
        "failure",
        "high risk, owner approval needed: R-ci",
    );
    assert_eq!(sign(&fake, &k, AFTER).0, 1);
    // No policy is `owner`: refused even when the status says R-ci.
    let fake = pr(&k, "deny.toml", PLAIN, &config(None, None));
    let text = "high risk, owner approval needed: R-ci";
    fake.status(HEAD, "risk-check", "failure", text);
    assert_eq!(sign(&fake, &k, AFTER).0, 1);
    // Inside the window nothing changes.
    let fake = pr(
        &k,
        ".claude/settings.json",
        PLAIN,
        &config(None, Some("2026-11-30")),
    );
    let (code, v) = sign(&fake, &k, IN);
    assert_eq!(code, 0, "{v}");
    assert!(line(&v).contains("Signed in bootstrap mode (D-012, until 2026-11-30)"));
    assert_eq!(v["data"]["bootstrap_until"], "2026-11-30");
    assert_eq!(v["data"]["policy"], "owner");
}

#[test]
fn a_local_composite_action_is_a_workflow_file() {
    // `uses: ./tools/x` runs `tools/x/action.yml` inside a workflow: owner-class like one.
    for action in ["tools/x/action.yml", "action.yaml", "ci/Setup/Action.YML"] {
        let k = key();
        let cfg = config(Some("orchestrator"), None);
        let fake = pr(&k, ".claude/settings.json", PLAIN, &cfg);
        fake.pull_file(1, action, "modified", 1, 0, Some(PLAIN));
        assert!(!counts_on(&fake, &k, AFTER), "{action}");
    }
    // Without it the same PR counts.
    let cfg = config(Some("orchestrator"), None);
    assert!(counts(".claude/settings.json", PLAIN, &cfg, AFTER));
}
