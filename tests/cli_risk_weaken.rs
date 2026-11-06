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
fn a_file_renamed_into_a_gate_path_is_judged_against_the_builtin() {
    let fake = pr();
    let weak = "[[rule]]\nid = \"R-gov\"\nwhy = \"w\"\npaths = [\"x\"]\n";
    fake.pull_file(1, ".bosun/risk.toml", "renamed", 0, 0, None);
    fake.renamed_from(1, "docs/weak.toml");
    fake.content("docs/weak.toml", BASE, weak);
    fake.content(".bosun/risk.toml", HEAD, weak);
    let e = evidence(&fake);
    assert!(
        e.contains(".bosun/risk.toml: R-secrets rule present → (removed)"),
        "{e}"
    );
}

#[test]
fn repo_rules_without_r_weaken_still_send_a_weakening_to_the_owner() {
    let fake = pr();
    let rules = "[[rule]]\nid = \"R-x\"\nwhy = \"w\"\npaths = [\"nothing/**\"]\n";
    fake.content(".bosun/risk.toml", BASE, rules);
    budget(&fake, 150, 300);
    let (code, v) = check(&fake);
    assert_eq!(code, 5, "{v}");
    assert_eq!(v["data"]["requirement"], "owner", "{v}");
    let m = v["data"]["matches"].to_string();
    assert!(
        m.contains("R-weaken") && m.contains("daily_usd 150 → 300"),
        "{v}"
    );
}

#[test]
fn a_weakening_needs_the_owner_even_if_the_rules_delegate_it() {
    for class in ["delegate", "two-verdicts"] {
        let fake = pr();
        fake.content(
            ".bosun/risk.toml",
            BASE,
            &format!("[[rule]]\nid = \"R-weaken\"\nwhy = \"w\"\ncapability = \"weaken\"\napproval = \"{class}\"\n"),
        );
        budget(&fake, 150, 300);
        let (code, v) = check(&fake);
        assert_eq!(code, 5, "{v}");
        assert_eq!(v["data"]["requirement"], "owner", "{class}: {v}");
    }
}
