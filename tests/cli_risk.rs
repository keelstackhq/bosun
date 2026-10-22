mod common;

use common::fake::FakeGithub;
use common::run;
use serde_json::Value;

const NOW: &str = "2026-10-22T10:00:00Z";
const BASE: &str = "base0000";
const HEAD: &str = "sha1";
const OWNERS: &str = "[[owner]]\ngithub = \"ada\"\nslack = \"U1\"\n\n[[owner]]\ngithub = \"bo\"\n";

fn check(fake: &FakeGithub) -> (i32, Value) {
    run(fake, "session_test", NOW, &["risk-check", "1"])
}

/// PR 1 (head `sha1`, base `base0000`) with the owners file at the base.
fn pr(body: &str) -> FakeGithub {
    let fake = FakeGithub::start();
    fake.pull(1, "change", "feature", body);
    fake.content("governance/owners.toml", BASE, OWNERS);
    fake
}

fn plus(line: &str) -> String {
    format!("@@ -0,0 +1,1 @@\n+{line}")
}

fn last_status(fake: &FakeGithub) -> Value {
    let all = fake.statuses_of(HEAD);
    let s = all
        .iter()
        .rev()
        .find(|s| s["context"] == "risk-check")
        .expect("a risk-check status");
    s.clone()
}

fn rule_ids(v: &Value) -> Vec<String> {
    v["data"]["matches"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m["rule"].as_str().unwrap().to_string())
        .collect()
}

#[test]
fn normal_pr_passes_with_a_success_status() {
    let fake = pr("Closes #7");
    fake.pull_file(1, "src/a.rs", "modified", 1, 0, Some(&plus("let x = 1;")));
    let (code, v) = check(&fake);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["level"], "normal");
    assert_eq!(v["data"]["head_sha"], HEAD);
    assert_eq!(last_status(&fake)["state"], "success");
    assert!(!fake.pull_body(1).contains("bosun:risk"));
}

#[test]
fn pr_that_weakens_the_rules_is_judged_by_the_base_rules() {
    let fake = pr("");
    fake.content(
        ".bosun/risk.toml",
        BASE,
        "[[rule]]\nid = \"R-gov\"\nwhy = \"Governance.\"\npaths = [\".bosun/**\"]\n",
    );
    // At head the rule is deleted; the base copy is what counts.
    fake.content(".bosun/risk.toml", HEAD, "");
    fake.pull_file(1, ".bosun/risk.toml", "modified", 0, 3, None);
    let (code, v) = check(&fake);
    assert_eq!(code, 5, "{v}");
    assert_eq!(v["data"]["level"], "high");
    assert_eq!(rule_ids(&v), ["R-gov"]);
    assert!(v["error"]["fix"].as_str().unwrap().contains(HEAD));
    assert_eq!(last_status(&fake)["state"], "failure");
    assert!(
        last_status(&fake)["description"]
            .as_str()
            .unwrap()
            .contains("R-gov")
    );
}

#[test]
fn bad_base_rules_are_an_error_naming_the_rule() {
    let fake = pr("");
    fake.content(
        ".bosun/risk.toml",
        BASE,
        "[[rule]]\nid = \"R-bad\"\nwhy = \"w\"\ncontent = \"(\"\n",
    );
    fake.pull_file(1, "src/a.rs", "modified", 1, 0, None);
    let (code, v) = check(&fake);
    assert_eq!(code, 1, "{v}");
    assert!(v["error"]["message"].as_str().unwrap().contains("R-bad"));
}

#[test]
fn new_crate_matches_r_deps() {
    let fake = pr("");
    fake.pull_file(1, "Cargo.toml", "modified", 1, 0, None);
    fake.content("Cargo.toml", BASE, "[dependencies]\nserde = \"1\"\n");
    fake.content(
        "Cargo.toml",
        HEAD,
        "[dependencies]\nserde = \"1\"\nevil = \"0.1\"\n",
    );
    let (code, v) = check(&fake);
    assert_eq!(code, 5, "{v}");
    assert_eq!(rule_ids(&v), ["R-deps"]);
    assert_eq!(
        v["data"]["matches"][0]["evidence"][0],
        "new dependency evil"
    );
}

#[test]
fn version_bump_of_an_existing_crate_is_normal() {
    let fake = pr("");
    fake.pull_file(1, "Cargo.toml", "modified", 1, 1, None);
    fake.content("Cargo.toml", BASE, "[dependencies]\nserde = \"1\"\n");
    fake.content("Cargo.toml", HEAD, "[dependencies]\nserde = \"2\"\n");
    let (code, _) = check(&fake);
    assert_eq!(code, 0);
}

#[test]
fn added_secret_line_matches_removed_one_does_not() {
    let fake = pr("");
    fake.pull_file(1, "a.env", "modified", 1, 0, Some(&plus("API_KEY = abc")));
    let (code, v) = check(&fake);
    assert_eq!(code, 5, "{v}");
    assert_eq!(rule_ids(&v), ["R-secrets"]);
    assert!(!v.to_string().contains("abc"));

    let fake = pr("");
    let removed = "@@ -1,1 +0,0 @@\n-API_KEY = abc";
    fake.pull_file(1, "a.env", "modified", 0, 1, Some(removed));
    let (code, v) = check(&fake);
    assert_eq!(code, 0, "{v}");
}

#[test]
fn capability_from_the_linked_task_or_a_label() {
    let fake = pr("Closes #7");
    fake.issue(
        7,
        "task",
        &[],
        "<!-- bosun:task {\"capabilities\":[\"prod-write\"]} -->",
    );
    fake.pull_file(1, "src/a.rs", "modified", 1, 0, None);
    let (code, v) = check(&fake);
    assert_eq!((code, rule_ids(&v)), (5, vec!["R-prod-write".to_string()]));

    let fake = pr("");
    fake.pull_set(
        1,
        "labels",
        serde_json::json!([{"name": "bosun:capability/prod-write"}]),
    );
    fake.pull_file(1, "src/a.rs", "modified", 1, 0, None);
    let (code, v) = check(&fake);
    assert_eq!((code, rule_ids(&v)), (5, vec!["R-prod-write".to_string()]));
}

#[test]
fn owner_approval_counts_only_on_the_head_sha() {
    let fake = pr("");
    fake.pull_file(1, ".github/workflows/ci.yml", "modified", 1, 0, None);
    fake.review(1, "ada", "APPROVED", "oldsha");
    let (code, v) = check(&fake);
    assert_eq!(code, 5, "{v}");
    assert_eq!(last_status(&fake)["state"], "failure");

    fake.review(1, "mallory", "APPROVED", HEAD);
    let (code, _) = check(&fake);
    assert_eq!(code, 5, "a non-owner does not count");

    fake.review(1, "ada", "APPROVED", HEAD);
    let (code, v) = check(&fake);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["level"], "high");
    assert_eq!(v["data"]["approved_by"], "ada");
    assert_eq!(last_status(&fake)["state"], "success");
}

#[test]
fn a_comment_review_is_not_an_approval() {
    let fake = pr("");
    fake.pull_file(1, ".github/workflows/ci.yml", "modified", 1, 0, None);
    fake.review(1, "ada", "COMMENTED", HEAD);
    assert_eq!(check(&fake).0, 5);
}

#[test]
fn risk_note_is_replaced_not_duplicated() {
    let fake = pr("Closes #7\n\nKeep this text.");
    fake.pull_file(1, ".github/workflows/ci.yml", "modified", 1, 0, None);
    check(&fake);
    check(&fake);
    let body = fake.pull_body(1);
    assert_eq!(body.matches("<!-- bosun:risk -->").count(), 1, "{body}");
    assert_eq!(body.matches("<!-- /bosun:risk -->").count(), 1);
    assert!(body.contains("Keep this text."));
    assert!(body.contains("R-ci") && body.contains("CI decides what is verified."));
    assert!(body.contains("review the head SHA sha1 and approve"));
}

#[test]
fn missing_pull_is_an_error_with_a_fix() {
    let fake = FakeGithub::start();
    let (code, v) = check(&fake);
    assert_eq!(code, 1);
    assert!(v["error"]["fix"].as_str().is_some());
}
