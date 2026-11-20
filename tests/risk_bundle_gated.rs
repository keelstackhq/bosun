//! D-021: risk-check has no R-bundle; a bundle with a gated path is high by that path's rule.

mod common;

use common::fake::FakeGithub;
use common::run;
use serde_json::Value;

const NOW: &str = "2026-10-22T10:00:00Z";
const BASE: &str = "base0000";

fn check(body: &str, path: &str) -> (i32, Value) {
    let fake = FakeGithub::start();
    fake.pull(1, "change", "feature", body);
    fake.issue(7, "task", &[], "");
    fake.content(
        "governance/owners.toml",
        BASE,
        "[[owner]]\ngithub = \"ada\"\n",
    );
    fake.pull_file(1, path, "modified", 1, 0, Some("@@ -0,0 +1,1 @@\n+x"));
    run(&fake, "session_test", NOW, &["risk-check", "1"])
}

fn rules(v: &Value) -> Vec<String> {
    v["data"]["matches"]
        .as_array()
        .map(|a| {
            a.iter()
                .map(|m| m["rule"].as_str().unwrap().to_string())
                .collect()
        })
        .unwrap_or_default()
}

#[test]
fn a_bundle_changing_a_gated_file_is_judged_by_the_path_rule_alone() {
    // D-021: gated work may bundle; the gated path still rates high, no R-bundle on top.
    let (code, v) = check("Closes #7\nCloses #8", "hoist.toml");
    assert_eq!(code, 5, "{v}");
    let r = rules(&v);
    assert!(!r.is_empty() && !r.contains(&"R-bundle".to_string()), "{v}");
    assert_eq!(v["data"]["level"], "high", "{v}");
    assert_eq!(v["data"]["requirement"], "owner", "{v}");
}

#[test]
fn two_closes_on_ordinary_code_is_normal() {
    let (code, v) = check("Closes #7\nCloses #8", "src/a.rs");
    assert_eq!(code, 0, "{v}");
    assert!(rules(&v).is_empty(), "{v}");
}

/// A pull #1 changing only `src/a.rs`, with task #7 (plain) and #8 (`risk: high`, no risk path).
fn high_world(title: &str, body: &str) -> FakeGithub {
    let fake = FakeGithub::start();
    fake.pull(1, title, "feature", body);
    fake.issue(
        7,
        "lead",
        &[],
        r#"<!-- bosun:task {"item":"M1","task":"T1"} -->"#,
    );
    fake.issue(
        8,
        "bundled",
        &[],
        r#"<!-- bosun:task {"item":"M1","task":"T2","risk":"high"} -->"#,
    );
    fake.content(
        "governance/owners.toml",
        BASE,
        "[[owner]]\ngithub = \"ada\"\n",
    );
    fake.pull_file(1, "src/a.rs", "modified", 1, 0, Some("@@ -0,0 +1,1 @@\n+x"));
    fake
}

fn needs_owner_by_plan(v: &Value) {
    assert!(rules(v).contains(&"R-plan".to_string()), "{v}");
    assert_eq!(v["data"]["requirement"], "owner", "{v}");
}

#[test]
fn a_bundled_risk_high_task_that_is_not_the_lead_still_needs_the_owner() {
    // D-021 lets gated work bundle: every linked task counts, not only the first.
    let fake = high_world("change", "Closes #7\nCloses #8");
    let (code, v) = run(&fake, "session_test", NOW, &["risk-check", "1"]);
    assert_eq!(code, 5, "{v}");
    needs_owner_by_plan(&v);
}

#[test]
fn a_closing_ref_in_the_title_or_a_commit_cannot_skip_r_plan() {
    let fake = high_world("change (closes #8)", "Closes #7");
    let (_, v) = run(&fake, "session_test", NOW, &["risk-check", "1"]);
    needs_owner_by_plan(&v);

    let fake = high_world("change", "Closes #7");
    fake.pull_commit_shas(1, &["c1"]);
    fake.git_commit_message("c1", "wip\n\nFixes #8");
    let (_, v) = run(&fake, "session_test", NOW, &["risk-check", "1"]);
    needs_owner_by_plan(&v);
}

#[test]
fn a_commit_list_at_the_cap_fails_closed_to_the_owner() {
    // The author can pad a PR past GitHub's 250-commit list to hide a closing ref.
    let fake = high_world("change", "Closes #7");
    let shas: Vec<String> = (0..250).map(|n| format!("c{n}")).collect();
    fake.pull_commit_shas(1, &shas.iter().map(String::as_str).collect::<Vec<_>>());
    for sha in &shas {
        fake.git_commit_message(sha, "wip");
    }
    let (_, v) = run(&fake, "session_test", NOW, &["risk-check", "1"]);
    needs_owner_by_plan(&v);
    // An unreadable commit (an API error) is skipped, not escalated.
    let fake = high_world("change", "Closes #7");
    fake.pull_commit_shas(1, &["gone"]);
    let (code, v) = run(&fake, "session_test", NOW, &["risk-check", "1"]);
    assert_eq!(code, 0, "{v}");
}
