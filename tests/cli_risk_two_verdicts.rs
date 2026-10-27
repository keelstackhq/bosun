//! The two-verdict approval class (spec section 8): a PR whose only matches are R-ci-review
//! passes with two independent verdict records on its head. The rules come from the base file.

mod common;

use common::fake::FakeGithub;
use common::run;
use serde_json::Value;

const NOW: &str = "2026-10-22T10:00:00Z";
const BASE: &str = "base0000";
const HEAD: &str = "sha1";
const BOT: &str = "github-actions[bot]";
const OWNERS: &str = "[[owner]]\ngithub = \"ada\"\n";
/// The base rules: a small rule file with both classes (the repo's own rules are tested in
/// `cli_risk_repo_rules.rs`).
const RULES: &str = r#"
[[rule]]
id = "R-gov"
why = "g"
paths = [".bosun/**", "governance/**"]

[[rule]]
id = "R-ci"
why = "c"
paths = [".github/workflows/ci.yml"]

[[rule]]
id = "R-gate"
why = "x"
paths = ["src/gates/**"]

[[rule]]
id = "R-ci-review"
why = "r"
approval = "two-verdicts"
paths = [".github/workflows/**", "channel/**", "deny.toml"]

[[rule]]
id = "R-deps"
why = "d"
new_dependency = true
"#;

fn check(fake: &FakeGithub) -> (i32, Value) {
    run(fake, "session_test", NOW, &["risk-check", "1"])
}

fn plus() -> String {
    "@@ -0,0 +1,1 @@\n+x".to_string()
}

/// A PR that changes `path` only.
fn pr_with(path: &str) -> FakeGithub {
    let fake = FakeGithub::start();
    fake.pull(1, "change", "feature", "Closes #7");
    fake.issue(7, "task", &[], "");
    fake.content("governance/owners.toml", BASE, OWNERS);
    fake.content(".bosun/risk.toml", BASE, RULES);
    fake.pull_file(1, path, "modified", 1, 0, Some(&plus()));
    fake
}

/// A PR that only matches R-ci-review.
fn review_pr() -> FakeGithub {
    pr_with(".github/workflows/other.yml")
}

fn verdict_line(session: &str, head: &str, approved: bool, confidence: Option<f64>) -> String {
    // The `bosun` key must come first, as bosun writes the record.
    let conf = confidence.map_or(String::new(), |c| format!(",\"confidence\":{c}"));
    format!(
        "Review verdict: x.\n\n{{\"bosun\":\"verdict\",\"session\":\"{session}\",\"head_sha\":\"{head}\",\"approved\":{approved}{conf}}}"
    )
}

fn verdict(fake: &FakeGithub, session: &str, head: &str, approved: bool, c: f64) {
    fake.comment_by(1, BOT, &verdict_line(session, head, approved, Some(c)));
}

fn last_description(fake: &FakeGithub) -> String {
    let all = fake.statuses_of(HEAD);
    let s = all.iter().rev().find(|s| s["context"] == "risk-check");
    s.expect("a risk-check status")["description"]
        .as_str()
        .unwrap()
        .to_string()
}

#[test]
fn fails_with_zero_or_one_verdict_and_passes_with_two_sessions() {
    let fake = review_pr();
    let (code, v) = check(&fake);
    assert_eq!(code, 5, "{v}");
    assert_eq!(v["data"]["requirement"], "two-verdicts");
    assert_eq!(
        last_description(&fake),
        "high risk, two verdicts needed (0/2): R-ci-review"
    );
    let fix = v["error"]["fix"].as_str().unwrap();
    assert!(fix.contains(HEAD) && fix.contains("0.9"), "{fix}");

    verdict(&fake, "s1", HEAD, true, 0.95);
    let (code, _) = check(&fake);
    assert_eq!(code, 5);
    assert_eq!(
        last_description(&fake),
        "high risk, two verdicts needed (1/2): R-ci-review"
    );

    verdict(&fake, "s2", HEAD, true, 0.9);
    let (code, v) = check(&fake);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["requirement"], "two-verdicts");
    assert_eq!(v["data"]["verdicts"], serde_json::json!(["s1", "s2"]));
    assert_eq!(
        last_description(&fake),
        "high risk (R-ci-review), approved by two verdicts: s1, s2"
    );
    let body = fake.pull_body(1);
    assert!(body.contains("two independent review verdicts"), "{body}");
    assert!(body.contains(HEAD));
}

#[test]
fn the_same_session_twice_counts_once() {
    let fake = review_pr();
    verdict(&fake, "s1", HEAD, true, 0.95);
    verdict(&fake, "s1", HEAD, true, 0.99);
    assert_eq!(check(&fake).0, 5);
    assert_eq!(
        last_description(&fake),
        "high risk, two verdicts needed (1/2): R-ci-review"
    );
}

#[test]
fn confidence_below_point_nine_or_missing_does_not_count() {
    let fake = review_pr();
    verdict(&fake, "s1", HEAD, true, 0.95);
    verdict(&fake, "s2", HEAD, true, 0.89);
    assert_eq!(check(&fake).0, 5);
    // An old record without confidence never counts.
    fake.comment_by(1, BOT, &verdict_line("s3", HEAD, true, None));
    assert_eq!(check(&fake).0, 5);
}

#[test]
fn a_record_by_a_non_bot_author_does_not_count() {
    let fake = review_pr();
    verdict(&fake, "s1", HEAD, true, 0.95);
    fake.comment_by(1, "ada", &verdict_line("s2", HEAD, true, Some(0.99)));
    fake.comment_by(1, "some-agent", &verdict_line("s3", HEAD, true, Some(0.99)));
    assert_eq!(check(&fake).0, 5);
    assert_eq!(
        last_description(&fake),
        "high risk, two verdicts needed (1/2): R-ci-review"
    );
}

#[test]
fn verdicts_of_an_older_head_do_not_count() {
    let fake = review_pr();
    verdict(&fake, "s1", "oldsha", true, 0.95);
    verdict(&fake, "s2", "oldsha", true, 0.95);
    assert_eq!(check(&fake).0, 5);
    verdict(&fake, "s1", HEAD, true, 0.95);
    assert_eq!(check(&fake).0, 5, "one on the head is not two");
}

#[test]
fn a_later_disapproval_withdraws_the_approval() {
    let fake = review_pr();
    verdict(&fake, "s1", HEAD, true, 0.95);
    verdict(&fake, "s2", HEAD, true, 0.95);
    assert_eq!(check(&fake).0, 0);
    verdict(&fake, "s2", HEAD, false, 0.95);
    assert_eq!(check(&fake).0, 5);
    // A later approval from the same session restores it.
    verdict(&fake, "s2", HEAD, true, 0.95);
    assert_eq!(check(&fake).0, 0);
}

#[test]
fn an_owner_approval_passes_the_class() {
    let fake = review_pr();
    assert_eq!(check(&fake).0, 5);
    fake.review(1, "ada", "APPROVED", HEAD);
    let (code, v) = check(&fake);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["approved_by"], "ada");
    assert_eq!(
        last_description(&fake),
        "high risk (R-ci-review), approved by ada"
    );
}

#[test]
fn owner_gated_paths_still_need_the_owner_with_two_verdicts() {
    for path in [
        ".github/workflows/ci.yml",
        "src/gates/runner.rs",
        ".bosun/risk.toml",
    ] {
        let fake = pr_with(path);
        verdict(&fake, "s1", HEAD, true, 0.99);
        verdict(&fake, "s2", HEAD, true, 0.99);
        let (code, v) = check(&fake);
        assert_eq!(code, 5, "{path}: {v}");
        assert_eq!(v["data"]["requirement"], "owner", "{path}");
        assert!(
            last_description(&fake).starts_with("high risk, owner approval needed: "),
            "{path}"
        );
    }
}

#[test]
fn a_new_dependency_alongside_a_review_path_needs_the_owner() {
    let fake = review_pr();
    fake.pull_file(1, "Cargo.toml", "modified", 1, 0, Some(&plus()));
    fake.content("Cargo.toml", BASE, "[dependencies]\nserde = \"1\"\n");
    fake.content(
        "Cargo.toml",
        HEAD,
        "[dependencies]\nserde = \"1\"\nevil = \"0.1\"\n",
    );
    verdict(&fake, "s1", HEAD, true, 0.99);
    verdict(&fake, "s2", HEAD, true, 0.99);
    let (code, v) = check(&fake);
    assert_eq!(code, 5, "{v}");
    assert_eq!(v["data"]["requirement"], "owner");
    assert_eq!(
        last_description(&fake),
        "high risk, owner approval needed: R-ci-review, R-deps"
    );
}

#[test]
fn a_file_in_both_ci_classes_needs_the_owner() {
    // ci.yml is in R-ci (owner) and under `.github/workflows/**` (two verdicts).
    let fake = pr_with(".github/workflows/ci.yml");
    verdict(&fake, "s1", HEAD, true, 0.99);
    verdict(&fake, "s2", HEAD, true, 0.99);
    let (code, v) = check(&fake);
    assert_eq!(code, 5, "{v}");
    assert_eq!(v["data"]["requirement"], "owner");
}

#[test]
fn shipped_config_is_in_the_review_class() {
    for path in ["channel/x.toml", "deny.toml"] {
        let fake = pr_with(path);
        let (code, v) = check(&fake);
        assert_eq!(code, 5, "{path}: {v}");
        assert_eq!(v["data"]["requirement"], "two-verdicts", "{path}");
    }
}

#[test]
fn a_rename_from_an_owner_path_into_a_review_path_needs_the_owner() {
    let fake = pr_with("channel/gates-copy.rs");
    fake.renamed_from(1, "src/gates/runner.rs");
    verdict(&fake, "s1", HEAD, true, 0.99);
    verdict(&fake, "s2", HEAD, true, 0.99);
    let (code, v) = check(&fake);
    assert_eq!(code, 5, "{v}");
    assert_eq!(v["data"]["requirement"], "owner");
}

#[test]
fn an_unknown_approval_value_in_the_base_rules_is_an_error_with_an_error_status() {
    let fake = review_pr();
    fake.content(
        ".bosun/risk.toml",
        BASE,
        "[[rule]]\nid = \"R-x\"\nwhy = \"w\"\npaths = [\"a\"]\napproval = \"anyone\"\n",
    );
    let (code, v) = check(&fake);
    assert_eq!(code, 1, "{v}");
    assert!(v["error"]["message"].as_str().unwrap().contains("R-x"));
    assert!(!v["error"]["fix"].as_str().unwrap().is_empty());
    // Pending while judging, then `error`: never a verdict, never a pending left behind.
    let st: Vec<_> = fake
        .statuses_of(HEAD)
        .iter()
        .map(|s| s["state"].clone())
        .collect();
    assert_eq!(st, ["pending", "error"]);
}

#[test]
fn the_unknown_and_empty_sessions_never_count() {
    let fake = review_pr();
    verdict(&fake, "unknown", HEAD, true, 0.99);
    verdict(&fake, "", HEAD, true, 0.99);
    verdict(&fake, "s1", HEAD, true, 0.99);
    assert_eq!(check(&fake).0, 5);
    assert_eq!(
        last_description(&fake),
        "high risk, two verdicts needed (1/2): R-ci-review"
    );
}

const LEASE: &str = r#"{"bosun":"lease","session":"worker","origin":"local","expires":"2026-10-22T10:30:00Z","touch":["channel/**"]}"#;

#[test]
fn a_session_with_a_lease_on_the_linked_task_does_not_count() {
    let fake = FakeGithub::start();
    fake.pull(1, "change", "feature", "Closes #7");
    fake.issue(7, "task", &[], "");
    fake.content("governance/owners.toml", BASE, OWNERS);
    fake.content(".bosun/risk.toml", BASE, RULES);
    fake.pull_file(1, "channel/x.toml", "modified", 1, 0, Some(&plus()));
    fake.comment_by(7, "ada", LEASE);
    verdict(&fake, "worker", HEAD, true, 0.99);
    verdict(&fake, "s1", HEAD, true, 0.99);
    assert_eq!(check(&fake).0, 5);
    assert_eq!(
        last_description(&fake),
        "high risk, two verdicts needed (1/2): R-ci-review"
    );
    // A released lease still marks the session as a worker.
    fake.comment_by(
        7,
        "ada",
        r#"{"bosun":"lease-end","session":"s1","reason":"released"}"#,
    );
    verdict(&fake, "s2", HEAD, true, 0.99);
    assert_eq!(check(&fake).0, 5, "s1 had a lease: s2 alone is one");
    verdict(&fake, "s3", HEAD, true, 0.99);
    let (code, v) = check(&fake);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["verdicts"], serde_json::json!(["s2", "s3"]));
}

#[test]
fn only_the_comment_shape_apply_writes_counts() {
    let fake = review_pr();
    verdict(&fake, "s1", HEAD, true, 0.99);
    let line = |s: &str| verdict_line(s, HEAD, true, Some(0.99));
    let bare = line("s2").replace("Review verdict: x.\n\n", "");
    fake.comment_by(1, BOT, &bare);
    fake.comment_by(1, BOT, &format!("{}\n\ntrailing", line("s3")));
    fake.comment_by(1, BOT, &format!("{}\n{}", line("s4"), bare));
    fake.comment_by(1, BOT, &line("s5").replace("Review verdict", "Hello"));
    assert_eq!(check(&fake).0, 5);
    assert_eq!(
        last_description(&fake),
        "high risk, two verdicts needed (1/2): R-ci-review"
    );
}

#[test]
fn a_pr_that_links_no_task_needs_the_owner_even_with_two_verdicts() {
    let fake = FakeGithub::start();
    fake.pull(1, "change", "feature", "no link");
    fake.content("governance/owners.toml", BASE, OWNERS);
    fake.content(".bosun/risk.toml", BASE, RULES);
    fake.pull_file(1, "channel/x.toml", "modified", 1, 0, Some(&plus()));
    verdict(&fake, "s1", HEAD, true, 0.99);
    verdict(&fake, "s2", HEAD, true, 0.99);
    let (code, v) = check(&fake);
    assert_eq!(code, 5, "{v}");
    assert_eq!(v["data"]["requirement"], "two-verdicts");
    assert_eq!(v["data"]["verdicts"], serde_json::json!([]));
    assert_eq!(
        last_description(&fake),
        "high risk, owner approval needed (links no task, so verdicts cannot count): R-ci-review"
    );
    assert!(
        v["error"]["fix"]
            .as_str()
            .unwrap()
            .contains("links no task")
    );
    fake.review(1, "ada", "APPROVED", HEAD);
    assert_eq!(check(&fake).0, 0, "an owner still approves it");
}

#[test]
fn a_review_ci_verdict_counts_as_one_session() {
    // Documents the residual risk: review-ci (mode b, dormant) is a session like any other.
    let fake = review_pr();
    verdict(&fake, "review-ci", HEAD, true, 0.95);
    assert_eq!(check(&fake).0, 5);
    assert_eq!(
        last_description(&fake),
        "high risk, two verdicts needed (1/2): R-ci-review"
    );
    verdict(&fake, "s1", HEAD, true, 0.95);
    let (code, v) = check(&fake);
    assert_eq!(code, 0, "{v}");
    assert_eq!(
        v["data"]["verdicts"],
        serde_json::json!(["review-ci", "s1"])
    );
}
