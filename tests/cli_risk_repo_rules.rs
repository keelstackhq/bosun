//! This repo's own `.bosun/risk.toml`, put on the fake's base branch: which PRs need the owner
//! and which need two verdicts (spec section 8).

mod common;

use common::fake::FakeGithub;
use common::run;
use serde_json::Value;
use std::fs;
use std::path::Path;

const NOW: &str = "2026-10-22T10:00:00Z";
const BASE: &str = "base0000";
const OWNERS: &str = "[[owner]]\ngithub = \"ada\"\n";

fn rules() -> String {
    fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join(".bosun/risk.toml")).unwrap()
}

fn pr_touching(path: &str, previous: Option<&str>) -> FakeGithub {
    let fake = FakeGithub::start();
    fake.pull(1, "change", "feature", "Closes #7");
    fake.issue(7, "task", &[], "");
    fake.content("governance/owners.toml", BASE, OWNERS);
    fake.content(".bosun/risk.toml", BASE, &rules());
    // The base is bosun: its approval code is critical.
    fake.content("src/cli/risk_check.rs", BASE, "// risk-check");
    let status = if previous.is_some() {
        "renamed"
    } else {
        "modified"
    };
    fake.pull_file(1, path, status, 1, 0, Some("@@ -0,0 +1,1 @@\n+x"));
    if let Some(p) = previous {
        fake.renamed_from(1, p);
    }
    fake
}

fn check(fake: &FakeGithub) -> (i32, Value) {
    run(fake, "session_test", NOW, &["risk-check", "1"])
}

fn requirement(path: &str, previous: Option<&str>) -> String {
    let fake = pr_touching(path, previous);
    let (code, v) = check(&fake);
    assert_eq!(code, 5, "{path}: {v}");
    v["data"]["requirement"].as_str().unwrap().to_string()
}

#[test]
fn channel_needs_two_verdicts() {
    for path in ["channel/x", "channel/pieces/a/b.toml"] {
        assert_eq!(requirement(path, None), "two-verdicts", "{path}");
    }
}

#[test]
fn governance_and_the_approval_system_need_the_owner() {
    for path in [
        ".github/workflows/risk.yml",
        ".github/workflows/verdict.yml",
        ".github/workflows/release.yml",
        ".github/workflows/other.yaml",
        ".github/workflows/ci.yml",
        "src/anthropic.rs",
        "src/core/lessons.rs",
        "src/core/budget.rs",
        "src/sessions/usage.rs",
        "src/gates/runner.rs",
        "src/config.rs",
        "Cargo.toml",
        "Cargo.lock",
        "build.rs",
        "crates/x/build.rs",
        ".cargo/config.toml",
        "docs/lessons.toml",
        "src/cli/review_pack.rs",
        "REVIEW.md",
        "src/github/http.rs",
        "src/cli/mod.rs",
        "src/core/records.rs",
        "src/core/risk.rs",
        "src/core/risk_default.toml",
        "npm/bosun/package.json",
        ".bosun/risk.toml",
    ] {
        assert_eq!(requirement(path, None), "owner", "{path}");
    }
}

#[test]
fn ci_and_gate_inputs_outside_workflows_and_code_can_be_delegated() {
    for path in [
        "deny.toml",
        "rust-toolchain.toml",
        "rust-toolchain",
        ".config/nextest.toml",
        "profiles/rust.toml",
    ] {
        assert_eq!(requirement(path, None), "delegate", "{path}");
    }
}

#[test]
fn a_rename_out_of_gate_code_into_channel_needs_the_owner() {
    assert_eq!(requirement("channel/x.rs", Some("src/gates/x.rs")), "owner");
    // A delegate never stands in for the two verdicts of R-ci-review.
    assert_eq!(
        requirement("channel/x.toml", Some("profiles/x.toml")),
        "owner"
    );
}

#[test]
fn ordinary_code_is_normal_risk() {
    let fake = pr_touching("src/cli/claim.rs", None);
    let (code, v) = check(&fake);
    assert_eq!(code, 0, "{v}");
}
