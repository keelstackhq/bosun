//! The two-verdict approval class (spec section 8): a PR whose only matches are R-ci-review
//! passes with two verdict records of different sessions at confidence >= 0.9 on its head.

mod common;

use common::fake::FakeGithub;
use common::run;
use serde_json::Value;

const NOW: &str = "2026-10-22T10:00:00Z";
const BASE: &str = "base0000";
const HEAD: &str = "sha1";
const BOT: &str = "github-actions[bot]";
const OWNERS: &str = "[[owner]]\ngithub = \"ada\"\n";

fn check(fake: &FakeGithub) -> (i32, Value) {
    run(fake, "session_test", NOW, &["risk-check", "1"])
}

fn plus() -> String {
    "@@ -0,0 +1,1 @@\n+x".to_string()
}

/// A PR that changes `path` only.
fn pr_with(path: &str) -> FakeGithub {
    let fake = FakeGithub::start();
    fake.pull(1, "change", "feature", "");
    fake.content("governance/owners.toml", BASE, OWNERS);
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
        "src/core/risk.rs",
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
    let fake = pr_with(".github/workflows/risk.yml");
    verdict(&fake, "s1", HEAD, true, 0.99);
    verdict(&fake, "s2", HEAD, true, 0.99);
    let (code, v) = check(&fake);
    assert_eq!(code, 5, "{v}");
    assert_eq!(v["data"]["requirement"], "owner");
}

#[test]
fn shipped_config_is_in_the_review_class() {
    for path in ["channel/x.toml", "deny.toml", "rust-toolchain.toml"] {
        let fake = pr_with(path);
        let (code, v) = check(&fake);
        assert_eq!(code, 5, "{path}: {v}");
        assert_eq!(v["data"]["requirement"], "two-verdicts", "{path}");
    }
}
