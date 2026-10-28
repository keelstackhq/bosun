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
    fake.pull_file(
        1,
        ".bosun/risk.toml",
        "modified",
        0,
        3,
        Some("@@ -1,3 +0,0 @@\n-x"),
    );
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
    fake.pull_file(1, "src/a.rs", "modified", 1, 0, Some(&plus("x")));
    let (code, v) = check(&fake);
    assert_eq!(code, 1, "{v}");
    assert!(v["error"]["message"].as_str().unwrap().contains("R-bad"));
}

/// Only the dependency rule: Cargo.toml is also gate code under the repo's own rules.
fn deps_only(fake: &FakeGithub) {
    fake.content(
        ".bosun/risk.toml",
        BASE,
        "[[rule]]\nid = \"R-deps\"\nwhy = \"d\"\nnew_dependency = true\n",
    );
}

#[test]
fn new_crate_matches_r_deps() {
    let fake = pr("");
    deps_only(&fake);
    fake.pull_file(1, "Cargo.toml", "modified", 1, 0, Some(&plus("x")));
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
    deps_only(&fake);
    fake.pull_file(
        1,
        "Cargo.toml",
        "modified",
        1,
        1,
        Some("@@ -1 +1 @@\n-a\n+b"),
    );
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
    fake.pull_file(1, "src/a.rs", "modified", 1, 0, Some(&plus("x")));
    let (code, v) = check(&fake);
    assert_eq!((code, rule_ids(&v)), (5, vec!["R-prod-write".to_string()]));

    let fake = pr("");
    fake.pull_set(
        1,
        "labels",
        serde_json::json!([{"name": "bosun:capability/prod-write"}]),
    );
    fake.pull_file(1, "src/a.rs", "modified", 1, 0, Some(&plus("x")));
    let (code, v) = check(&fake);
    assert_eq!((code, rule_ids(&v)), (5, vec!["R-prod-write".to_string()]));
}

#[test]
fn owner_approval_counts_only_on_the_head_sha() {
    let fake = pr("");
    fake.pull_file(
        1,
        ".github/workflows/ci.yml",
        "modified",
        1,
        0,
        Some(&plus("x")),
    );
    fake.review(1, "ada", "APPROVED", "oldsha");
    let (code, v) = check(&fake);
    assert_eq!(code, 5, "{v}");
    assert_eq!(last_status(&fake)["state"], "failure");
    // No review event runs the risk workflow: a review counts on the next run.
    let fix = v["error"]["fix"].as_str().unwrap();
    assert!(fix.contains("then comment `/bosun recheck`"), "{fix}");

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

/// No review event runs the risk workflow, so a withdrawal shows only on the next run (the
/// owner comments `/bosun recheck`): that run must fail, for a request for changes after the
/// approval and for the approval itself dismissed (GitHub then reports it DISMISSED).
#[test]
fn a_withdrawn_review_approval_fails_the_next_run() {
    let world = || {
        let fake = pr("");
        fake.pull_file(
            1,
            ".github/workflows/ci.yml",
            "modified",
            1,
            0,
            Some(&plus("x")),
        );
        fake
    };
    let fake = world();
    fake.review(1, "ada", "APPROVED", HEAD);
    assert_eq!(check(&fake).0, 0);
    assert_eq!(last_status(&fake)["state"], "success");
    fake.review(1, "ada", "CHANGES_REQUESTED", HEAD);
    let (code, v) = check(&fake);
    assert_eq!(
        code, 5,
        "request-changes withdraws the review approval: {v}"
    );
    assert_eq!(last_status(&fake)["state"], "failure");

    let fake = world();
    fake.review(1, "ada", "DISMISSED", HEAD);
    let (code, v) = check(&fake);
    assert_eq!(code, 5, "a dismissed approval is no approval: {v}");
    assert_eq!(last_status(&fake)["state"], "failure");
}

#[test]
fn a_comment_review_is_not_an_approval() {
    let fake = pr("");
    fake.pull_file(
        1,
        ".github/workflows/ci.yml",
        "modified",
        1,
        0,
        Some(&plus("x")),
    );
    fake.review(1, "ada", "COMMENTED", HEAD);
    assert_eq!(check(&fake).0, 5);
}

#[test]
fn risk_note_is_replaced_not_duplicated() {
    let fake = pr("Closes #7\n\nKeep this text.");
    fake.pull_file(
        1,
        ".github/workflows/ci.yml",
        "modified",
        1,
        0,
        Some(&plus("x")),
    );
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

const PROD_TASK: &str = "<!-- bosun:task {\"capabilities\":[\"prod-write\"]} -->";

fn code_change(fake: &FakeGithub) {
    fake.pull_file(1, "src/a.rs", "modified", 1, 0, Some(&plus("let x = 1;")));
}

#[test]
fn decoy_closes_before_the_real_task_still_sees_prod_write() {
    let fake = pr("Closes #8\nFixes #7");
    fake.issue(8, "decoy", &[], "no metadata");
    fake.issue(7, "task", &[], PROD_TASK);
    code_change(&fake);
    let (code, v) = check(&fake);
    assert_eq!((code, rule_ids(&v)), (5, vec!["R-prod-write".to_string()]));
}

#[test]
fn marker_only_task_link_counts() {
    let fake = pr("work\n\n<!-- bosun:task-pr 7 -->");
    fake.issue(7, "task", &[], PROD_TASK);
    code_change(&fake);
    let (code, v) = check(&fake);
    assert_eq!((code, rule_ids(&v)), (5, vec!["R-prod-write".to_string()]));
}

#[test]
fn more_changed_files_than_listed_is_high() {
    let fake = pr("");
    code_change(&fake);
    fake.pull_set(1, "changed_files", serde_json::json!(3001));
    let (code, v) = check(&fake);
    assert_eq!((code, rule_ids(&v)), (5, vec!["R-unseen".to_string()]));
    let first = v["data"]["matches"][0]["evidence"][0].as_str().unwrap();
    assert!(first.contains("3001"), "{first}");
}

#[test]
fn text_file_without_a_patch_is_high_but_binary_is_not() {
    let fake = pr("");
    fake.pull_file(1, "src/huge.rs", "modified", 5000, 10, None);
    let (code, v) = check(&fake);
    assert_eq!((code, rule_ids(&v)), (5, vec!["R-unseen".to_string()]));

    // The name is the author's choice: changed lines without a patch are unseen.
    let fake = pr("");
    fake.pull_file(1, "payload.bin", "added", 5000, 0, None);
    let (code, v) = check(&fake);
    assert_eq!((code, rule_ids(&v)), (5, vec!["R-unseen".to_string()]));

    let fake = pr("");
    fake.pull_file(1, "logo.png", "added", 0, 0, None);
    fake.pull_set(1, "changed_files", serde_json::json!(1));
    let (code, v) = check(&fake);
    assert_eq!(code, 0, "{v}");
}

fn high_pr() -> FakeGithub {
    let fake = pr("");
    fake.pull_file(
        1,
        ".github/workflows/ci.yml",
        "modified",
        1,
        0,
        Some(&plus("x")),
    );
    fake
}

const HEAD40: &str = "0123456789abcdef0123456789abcdef01234567";

#[test]
fn an_owner_comment_approves_exactly_the_head_sha() {
    let fake = high_pr();
    fake.pull_set(
        1,
        "head",
        serde_json::json!({"sha": HEAD40, "ref": "feature"}),
    );
    fake.comment_by(
        1,
        "ada",
        "/bosun approve 1111111111111111111111111111111111111111",
    );
    fake.comment_by(1, "mallory", &format!("/bosun approve {HEAD40}"));
    fake.comment_by(1, "ada", &format!("please /bosun approve {HEAD40}"));
    assert_eq!(
        check(&fake).0,
        5,
        "old sha, non-owner and extra text do not count"
    );

    // An edited comment does not count: someone with write access may have changed it.
    fake.comment_by(1, "bo", &format!("/bosun approve {HEAD40}"));
    fake.state
        .lock()
        .unwrap()
        .comments
        .last_mut()
        .unwrap()
        .updated_at = "2026-10-22T11:00:00Z".into();
    assert_eq!(check(&fake).0, 5, "an edited comment does not count");

    fake.comment_by(1, "Ada", &format!("  /bosun approve {HEAD40}\n"));
    let (code, v) = check(&fake);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["approved_by"], "ada");
    let s = fake.statuses_of(HEAD40);
    let s = s
        .iter()
        .rev()
        .find(|s| s["context"] == "risk-check")
        .unwrap();
    assert_eq!(s["state"], "success");
    assert!(
        s["description"]
            .as_str()
            .unwrap()
            .contains("approved by ada"),
        "{s}"
    );

    fake.review(1, "ada", "CHANGES_REQUESTED", HEAD40);
    assert_eq!(check(&fake).0, 5, "request-changes withdraws the comment");
}

#[test]
fn an_approval_written_through_an_app_or_without_the_app_field_does_not_count() {
    let fake = high_pr();
    fake.pull_set(
        1,
        "head",
        serde_json::json!({"sha": HEAD40, "ref": "feature"}),
    );
    // Agent sessions act as the owner's login through the Claude app.
    fake.comment_by(1, "ada", &format!("/bosun approve {HEAD40}"));
    fake.last_comment(|c| c.via_app = Some("claude".into()));
    assert_eq!(check(&fake).0, 5, "a comment via an app is not the owner");
    // A response without the field is no proof of a user token: fail closed.
    fake.comment_by(1, "ada", &format!("/bosun approve {HEAD40}"));
    fake.last_comment(|c| c.no_app_field = true);
    assert_eq!(check(&fake).0, 5, "a missing app field does not count");
    fake.comment_by(1, "ada", &format!("/bosun approve {HEAD40}"));
    assert_eq!(check(&fake).0, 0, "a plain user token counts");
}
