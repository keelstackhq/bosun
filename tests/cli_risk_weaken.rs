//! R-weaken: a PR that loosens a gate is high risk and needs the owner (spec §8).
mod common;

use common::fake::FakeGithub;
use common::run;
use serde_json::Value;

const NOW: &str = "2026-10-22T10:00:00Z";
const BASE: &str = "base0000";
const HEAD: &str = "sha1";

fn pr() -> FakeGithub {
    let fake = FakeGithub::start();
    fake.pull(1, "change", "feature", "");
    fake.content(
        "governance/owners.toml",
        BASE,
        "[[owner]]\ngithub = \"ada\"\n",
    );
    fake.content(
        ".bosun/risk.toml",
        BASE,
        "[[rule]]\nid = \"R-weaken\"\nwhy = \"w\"\ncapability = \"weaken\"\n",
    );
    fake
}

fn check(fake: &FakeGithub) -> (i32, Value) {
    run(fake, "session_test", NOW, &["risk-check", "1"])
}

fn budget(fake: &FakeGithub, old: u32, new: u32) {
    let patch = format!("@@ -1,2 +1,2 @@\n [budget]\n-daily_usd = {old}\n+daily_usd = {new}");
    fake.pull_file(1, ".bosun/limits.toml", "modified", 1, 1, Some(&patch));
    fake.content(
        ".bosun/limits.toml",
        BASE,
        &format!("[budget]\ndaily_usd = {old}\n"),
    );
    fake.content(
        ".bosun/limits.toml",
        HEAD,
        &format!("[budget]\ndaily_usd = {new}\n"),
    );
}

#[test]
fn a_raised_cap_is_high_and_needs_the_owner() {
    let fake = pr();
    budget(&fake, 150, 300);
    let (code, v) = check(&fake);
    assert_eq!(code, 5, "{v}");
    assert_eq!(v["data"]["level"], "high");
    assert_eq!(v["data"]["requirement"], "owner");
    let m = &v["data"]["matches"][0];
    assert_eq!(m["rule"], "R-weaken");
    let evidence = m["evidence"].to_string();
    assert!(evidence.contains("daily_usd 150 → 300"), "{evidence}");
}

#[test]
fn continue_on_error_in_a_workflow_is_a_weakening() {
    let fake = pr();
    let patch = "@@ -1,2 +1,3 @@\n   test:\n+    continue-on-error: true\n     runs-on: x";
    fake.pull_file(1, ".github/workflows/ci.yml", "modified", 1, 0, Some(patch));
    let (code, v) = check(&fake);
    assert_eq!(code, 5, "{v}");
    assert_eq!(v["data"]["matches"][0]["rule"], "R-weaken");
}

#[test]
fn a_deleted_test_file_is_a_weakening() {
    let fake = pr();
    fake.pull_file(
        1,
        "tests/cli_a.rs",
        "removed",
        0,
        9,
        Some("@@ -1,1 +0,0 @@\n-#[test]"),
    );
    let (code, v) = check(&fake);
    assert_eq!(code, 5, "{v}");
    let evidence = v["data"]["matches"][0]["evidence"].to_string();
    assert!(
        evidence.contains("test file present → (deleted)"),
        "{evidence}"
    );
}

#[test]
fn a_lowered_cap_is_normal() {
    let fake = pr();
    budget(&fake, 300, 150);
    let (code, v) = check(&fake);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["level"], "normal");
    assert!(
        !v["data"]["matches"].to_string().contains("R-weaken"),
        "{v}"
    );
}

fn evidence(fake: &FakeGithub) -> String {
    let (code, v) = check(fake);
    assert_eq!(code, 5, "{v}");
    assert_eq!(v["data"]["matches"][0]["rule"], "R-weaken", "{v}");
    v["data"]["matches"][0]["evidence"].to_string()
}

#[test]
fn a_gate_file_renamed_away_counts_as_removed() {
    let fake = pr();
    fake.pull_file(1, "deny.toml.bak", "renamed", 0, 0, None);
    fake.renamed_from(1, "deny.toml");
    fake.content("deny.toml", BASE, "[sources]\nunknown-git = \"deny\"\n");
    let e = evidence(&fake);
    assert!(
        e.contains("deny.toml: sources.unknown-git deny → (removed)"),
        "{e}"
    );
}

#[test]
fn a_new_profile_override_is_compared_with_the_builtin() {
    let fake = pr();
    let text = include_str!("../profiles/rust.toml").replace("\"--check\"", "\"--all\"");
    fake.pull_file(
        1,
        ".bosun/profiles/rust.toml",
        "added",
        1,
        0,
        Some("@@\n+x"),
    );
    fake.content(".bosun/profiles/rust.toml", HEAD, &text);
    let e = evidence(&fake);
    assert!(
        e.contains(".bosun/profiles/rust.toml: gates.fmt.cmd"),
        "{e}"
    );
}

#[test]
fn a_file_renamed_into_a_gate_path_is_judged_against_the_builtin() {
    let fake = pr();
    let text = include_str!("../profiles/rust.toml").replace("\"--check\"", "\"--all\"");
    fake.pull_file(1, ".bosun/profiles/rust.toml", "renamed", 0, 0, None);
    fake.renamed_from(1, "docs/weak.toml");
    fake.content("docs/weak.toml", BASE, &text);
    fake.content(".bosun/profiles/rust.toml", HEAD, &text);
    let e = evidence(&fake);
    assert!(
        e.contains(".bosun/profiles/rust.toml: gates.fmt.cmd"),
        "{e}"
    );
}

#[test]
fn tests_moved_between_files_are_no_weakening_but_a_dropped_inline_test_is() {
    let fake = pr();
    fake.pull_file(
        1,
        "tests/a.rs",
        "modified",
        0,
        1,
        Some("@@\n-#[test]\n-assert!(x);"),
    );
    fake.pull_file(
        1,
        "tests/b.rs",
        "modified",
        1,
        0,
        Some("@@\n+#[test]\n+assert!(x);"),
    );
    let (code, v) = check(&fake);
    assert_eq!(code, 0, "{v}");
    assert!(
        !v["data"]["matches"].to_string().contains("R-weaken"),
        "{v}"
    );

    let fake = pr();
    fake.pull_file(1, "src/a.rs", "modified", 0, 1, Some("@@\n-#[test]"));
    let e = evidence(&fake);
    assert!(e.contains("tests (all files) 1 → 0"), "{e}");
}

#[test]
fn test_counts_cover_skips_assertions_ts_and_unseen_removals() {
    let case = |name: &str, status: &str, patch: Option<&str>, want: &str| {
        let fake = pr();
        fake.pull_file(1, name, status, 1, 1, patch);
        let e = evidence(&fake);
        assert!(e.contains(want), "{name}: {e}");
    };
    case(
        "tests/a.rs",
        "modified",
        Some("@@\n #[test]\n+#[ignore]"),
        "skips (all files) 0 → 1",
    );
    case(
        "tests/a.rs",
        "modified",
        Some("@@\n-assert!(x);\n+let _ = x;"),
        "assertions (all files) 1 → 0",
    );
    case(
        "web/a.test.ts",
        "modified",
        Some("@@\n-it('a', () => {});"),
        "tests (all files) 1 → 0",
    );
    case(
        "src/big.rs",
        "removed",
        None,
        "src/big.rs: diff unavailable",
    );
}
