mod common;

use common::fake::FakeGithub;
use common::run;
use serde_json::json;

const T0: &str = "2026-10-22T10:00:00Z";

/// PR 42 reviewed at head A, now at head B; `a.rs` is the PR's own file.
fn setup() -> FakeGithub {
    let fake = FakeGithub::start();
    fake.pull_as_issue(42, "the pr");
    fake.pull(42, "the pr", "claude/x", "no link");
    fake.pull_file(42, "a.rs", "added", 1, 0, Some("@@ pr a"));
    fake.comment(
        42,
        r#"{"bosun":"verdict","session":"s","head_sha":"A","approved":true}"#,
    );
    fake.status("A", "review-verdict", "success", "x");
    fake.pull_set(
        42,
        "head",
        json!({"sha": "B", "ref": "claude/x", "label": "acme:claude/x"}),
    );
    fake.compare_file("A", "B", "a.rs", "@@ merged");
    fake
}

/// pulls/files stops at 3000 files: fewer files than `changed_files` means the PR's own list
/// is incomplete, so no delta can be built from it.
#[test]
fn a_capped_pull_file_list_gives_a_full_review() {
    let fake = setup();
    fake.pull_set(42, "changed_files", json!(3001));
    let (code, v) = run(&fake, "s", T0, &["review-pack", "42"]);
    assert_eq!(code, 0, "{v}");
    assert!(v["data"]["since_sha"].is_null(), "{v}");
    assert!(
        v["data"]["delta_fallback"]
            .as_str()
            .is_some_and(|w| w.contains("3000")),
        "{v}"
    );
    assert_eq!(v["data"]["files"].as_array().unwrap().len(), 1, "{v}");
}

#[test]
fn a_complete_file_list_still_gives_a_delta() {
    let fake = setup();
    fake.pull_set(42, "changed_files", json!(1));
    let (code, v) = run(&fake, "s", T0, &["review-pack", "42"]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["since_sha"], "A", "{v}");
    assert!(v["data"]["delta_fallback"].is_null(), "{v}");
}

/// A compare full of main's files cannot flood the pack: 50 names and the full count.
#[test]
fn delta_not_in_pr_is_capped_at_fifty_with_a_count() {
    let fake = setup();
    for i in 0..60 {
        fake.compare_file("A", "B", &format!("main/{i:02}.rs"), "@@ main");
    }
    let (code, v) = run(&fake, "s", T0, &["review-pack", "42"]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["since_sha"], "A", "{v}");
    assert_eq!(
        v["data"]["delta_not_in_pr"].as_array().unwrap().len(),
        50,
        "{v}"
    );
    assert_eq!(v["data"]["delta_not_in_pr_count"], 60, "{v}");
}
