//! `hoist.toml` holds the required checks (and later the App pins) that `hoist apply` puts in
//! the ruleset, so it is R-gov and part of the critical floor; `docs/setup-script.sh` runs as
//! root in cloud environments, so it is R-ci (spec §8). Only the root files count: nothing
//! reads another. R-gov is made delegate-able here so that `data.critical` shows what the floor
//! still forces to the owner.
mod common;

use common::fake::FakeGithub;
use common::run;
use serde_json::{Value, json};

fn check(path: &str) -> (i32, Value) {
    let rules = include_str!("../src/core/risk_default.toml")
        .replace("id = \"R-gov\"", "id = \"R-gov\"\napproval = \"delegate\"");
    let fake = FakeGithub::start();
    fake.pull(1, "change", "feature", "");
    fake.content(
        "governance/owners.toml",
        "base0000",
        "[[owner]]\ngithub = \"ada\"\n",
    );
    fake.content(".bosun/risk.toml", "base0000", &rules);
    let patch = "@@ -1,1 +1,1 @@\n-required_checks = [\"ci-ok\", \"risk-check\"]\n+required_checks = [\"ci-ok\"]";
    fake.pull_file(1, path, "modified", 1, 1, Some(patch));
    run(
        &fake,
        "session_test",
        "2026-10-28T10:00:00Z",
        &["risk-check", "1"],
    )
}

fn rules(v: &Value) -> Vec<&str> {
    let ids = v["data"]["matches"].as_array().unwrap();
    ids.iter().map(|m| m["rule"].as_str().unwrap()).collect()
}

#[test]
fn hoist_toml_is_r_gov_and_critical() {
    for path in ["hoist.toml", "Hoist.toml"] {
        let (code, v) = check(path);
        assert_eq!((code, rules(&v)), (5, vec!["R-gov"]), "{path}: {v}");
        let critical = v["data"]["critical"].to_string();
        assert!(critical.contains(path), "{path} is critical: {v}");
    }
}

#[test]
fn the_setup_script_is_r_ci_and_needs_the_owner() {
    let (code, v) = check("docs/setup-script.sh");
    assert_eq!((code, rules(&v)), (5, vec!["R-ci"]), "{v}");
    assert_eq!(v["data"]["matches"][0]["approval"], json!("owner"), "{v}");
}

#[test]
fn other_files_and_nested_copies_are_not() {
    for path in [
        "src/hoist.rs",
        "docs/hoist.md",
        "crates/foo/hoist.toml",
        "crates/foo/docs/setup-script.sh",
    ] {
        let (code, v) = check(path);
        assert_eq!((code, rules(&v)), (0, vec![]), "{path}: {v}");
        assert!(v["data"]["critical"].is_null(), "{path}: {v}");
    }
}
