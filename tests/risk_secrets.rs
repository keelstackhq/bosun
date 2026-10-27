//! R-secrets covers sops files and age keys, and `.circleci/**` is CI and part of the critical
//! floor (spec §8). The rules are the shipped built-ins, with R-secrets and R-ci made
//! delegate-able so that `data.critical` shows what the floor still forces to the owner.
mod common;

use common::fake::FakeGithub;
use common::run;
use serde_json::Value;

fn check(path: &str, line: &str) -> (i32, Value) {
    let rules = include_str!("../src/core/risk_default.toml")
        .replace(
            "id = \"R-secrets\"",
            "id = \"R-secrets\"\napproval = \"delegate\"",
        )
        .replace("id = \"R-ci\"", "id = \"R-ci\"\napproval = \"delegate\"");
    let fake = FakeGithub::start();
    fake.pull(1, "change", "feature", "");
    fake.content(
        "governance/owners.toml",
        "base0000",
        "[[owner]]\ngithub = \"ada\"\n",
    );
    fake.content(".bosun/risk.toml", "base0000", &rules);
    let patch = format!("@@ -0,0 +1,1 @@\n+{line}");
    fake.pull_file(1, path, "added", 1, 0, Some(&patch));
    run(
        &fake,
        "session_test",
        "2026-10-27T10:00:00Z",
        &["risk-check", "1"],
    )
}

/// The matched rule ids, and `data.critical` as text.
fn seen(v: &Value) -> (Vec<&str>, String) {
    let ids = v["data"]["matches"].as_array().unwrap();
    let ids = ids.iter().map(|m| m["rule"].as_str().unwrap()).collect();
    (ids, v["data"]["critical"].to_string())
}

#[test]
fn secret_paths_and_key_lines_are_r_secrets_and_critical() {
    // The key is built at run time and is not valid: never a real one.
    let key = "AGE-SECRET-KEY-1".to_string() + &"Q".repeat(58);
    #[rustfmt::skip]
    let cases = [
        ("secrets.toml", "x: 1"), (".sops.yaml", "x: 1"), ("deploy/.sops.yaml", "x: 1"),
        ("access.toml", "x: 1"), ("secrets/prod.sops.yaml", "x: 1"), ("a/b/c.sops.json", "x: 1"),
        ("prod.sops.yml", "x: 1"), ("a/prod.sops.yml", "x: 1"), ("a/b.sops.env", "x: 1"),
        ("deploy/secrets.toml", "x: 1"), ("Secrets.toml", "x: 1"), (".SOPS.yaml", "x: 1"),
        ("svc/Access.toml", "x: 1"), ("A/B.SOPS.JSON", "x: 1"),
        ("notes.txt", &key), ("ci.sh", "SOPS_AGE_KEY=x"), ("ci.sh", "SOPS_AGE_KEY: x"),
    ];
    for (path, line) in cases {
        let (code, v) = check(path, line);
        let (ids, critical) = seen(&v);
        assert_eq!((code, ids), (5, vec!["R-secrets"]), "{path}: {v}");
        assert!(
            critical.contains("R-secrets") && !v.to_string().contains(&key),
            "{path}: {v}"
        );
    }
    let (code, v) = check("ci.sh", "echo SOPS_AGE_KEY_FILE");
    assert_eq!(code, 0, "a mention is not an assignment: {v}");
}

#[test]
fn other_ci_providers_are_r_ci_and_critical() {
    #[rustfmt::skip]
    let paths = [".gitlab-ci.yml", ".gitlab/ci/build.yml", ".buildkite/pipeline.yml",
        "Jenkinsfile", "ci/JENKINSFILE", "azure-pipelines.yml", "azure-pipelines.yaml",
        "bitbucket-pipelines.yml", ".travis.yml", ".drone.yml",
        ".github/actions/setup/action.yml", ".CircleCI/config.yml"];
    for path in paths {
        let (_, v) = check(path, "x: 1");
        let (ids, critical) = seen(&v);
        assert_eq!(ids, vec!["R-ci"], "{path}: {v}");
        assert!(critical.contains(path), "{path} is critical: {v}");
    }
}

#[test]
fn circleci_config_is_r_ci_and_critical() {
    let (code, v) = check(".circleci/config.yml", "version: 2.1");
    let (ids, critical) = seen(&v);
    assert_eq!((code, ids), (5, vec!["R-ci"]), "{v}");
    assert!(critical.contains(".circleci/config.yml"), "{v}");
}
