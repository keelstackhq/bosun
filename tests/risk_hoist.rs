//! Governance paths outside `.bosun/` (spec §8). Every `hoist.toml` is R-gov: hoist reads a
//! package's file for its `[publish]` and `[deploy]` targets, the deploys it mints tokens for.
//! Only the root file holds `[github]`, the required checks (and later the App pins) that
//! `hoist apply` puts in the ruleset, so only the root file is on the critical floor. The
//! agent-instruction files steer every agent, so they are R-gov too. The pasted copy of
//! `docs/setup-script.sh` runs as root in cloud environments, so the file is R-ci. R-gov is
//! made delegate-able here so that `data.critical` shows what the floor still forces to the
//! owner.
mod common;

use common::fake::FakeGithub;
use common::run;
use serde_json::{Value, json};

fn check_with(rules: &str, path: &str, previous: Option<&str>) -> (i32, Value) {
    let fake = FakeGithub::start();
    fake.pull(1, "change", "feature", "");
    fake.content(
        "governance/owners.toml",
        "base0000",
        "[[owner]]\ngithub = \"ada\"\n",
    );
    fake.content(".bosun/risk.toml", "base0000", rules);
    let patch = "@@ -1,1 +1,1 @@\n-required_checks = [\"ci-ok\", \"risk-check\"]\n+required_checks = [\"ci-ok\"]";
    fake.pull_file(1, path, "modified", 1, 1, Some(patch));
    if let Some(p) = previous {
        fake.renamed_from(1, p);
    }
    run(
        &fake,
        "session_test",
        "2026-10-28T10:00:00Z",
        &["risk-check", "1"],
    )
}

/// The built-in rules, with R-gov made delegate-able.
fn builtin() -> String {
    include_str!("../src/core/risk_default.toml")
        .replace("id = \"R-gov\"", "id = \"R-gov\"\napproval = \"delegate\"")
}

fn check(path: &str) -> (i32, Value) {
    check_with(&builtin(), path, None)
}

fn rules(v: &Value) -> Vec<&str> {
    let ids = v["data"]["matches"].as_array().unwrap();
    ids.iter().map(|m| m["rule"].as_str().unwrap()).collect()
}

fn critical(v: &Value) -> String {
    v["data"]["critical"].to_string()
}

#[test]
fn the_root_hoist_toml_is_r_gov_and_critical() {
    for path in ["hoist.toml", "Hoist.toml"] {
        let (code, v) = check(path);
        assert_eq!((code, rules(&v)), (5, vec!["R-gov"]), "{path}: {v}");
        assert!(critical(&v).contains(path), "{path} is critical: {v}");
    }
}

#[test]
fn a_hoist_toml_renamed_away_is_still_r_gov_and_critical() {
    let (code, v) = check_with(&builtin(), "docs/old-hoist.toml", Some("hoist.toml"));
    assert_eq!((code, rules(&v)), (5, vec!["R-gov"]), "{v}");
    assert!(critical(&v).contains("docs/old-hoist.toml"), "{v}");
}

#[test]
fn a_package_hoist_toml_is_r_gov_but_not_critical() {
    for path in ["crates/foo/hoist.toml", "deploy/web/Hoist.toml"] {
        let (code, v) = check(path);
        assert_eq!((code, rules(&v)), (5, vec!["R-gov"]), "{path}: {v}");
        assert!(v["data"]["critical"].is_null(), "{path}: {v}");
    }
}

#[test]
fn agent_instruction_files_are_r_gov() {
    #[rustfmt::skip]
    let paths = ["AGENTS.md", "CLAUDE.md", "docs/agents/org-decisions.md",
        "crates/foo/AGENTS.md", "crates/foo/CLAUDE.md"];
    for path in paths {
        let (code, v) = check(path);
        assert_eq!((code, rules(&v)), (5, vec!["R-gov"]), "{path}: {v}");
    }
}

#[test]
fn the_setup_script_is_r_ci_and_the_builtin_rules_need_the_owner() {
    let (code, v) = check("docs/setup-script.sh");
    assert_eq!((code, rules(&v)), (5, vec!["R-ci"]), "{v}");
    assert_eq!(v["data"]["matches"][0]["approval"], json!("owner"), "{v}");
}

#[test]
fn bosuns_own_rules_let_a_delegate_approve_the_setup_script() {
    let repo = include_str!("../.bosun/risk.toml");
    let (code, v) = check_with(repo, "docs/setup-script.sh", None);
    assert_eq!((code, rules(&v)), (5, vec!["R-ci"]), "{v}");
    assert_eq!(
        v["data"]["matches"][0]["approval"],
        json!("delegate"),
        "{v}"
    );
    assert!(v["data"]["critical"].is_null(), "not on the floor: {v}");
}

#[test]
fn other_files_are_not() {
    for path in [
        "src/hoist.rs",
        "docs/hoist.md",
        "crates/foo/docs/setup-script.sh",
        "docs/agents.md",
    ] {
        let (code, v) = check(path);
        assert_eq!((code, rules(&v)), (0, vec![]), "{path}: {v}");
        assert!(v["data"]["critical"].is_null(), "{path}: {v}");
    }
}
