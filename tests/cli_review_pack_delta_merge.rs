mod common;

use common::fake::FakeGithub;
use common::run;
use serde_json::json;

const T0: &str = "2026-10-22T10:00:00Z";

/// PR 42 reviewed at head A, now at head B; `a.rs` and `b.rs` are the PR's own files.
fn setup() -> FakeGithub {
    let fake = FakeGithub::start();
    fake.pull_as_issue(42, "the pr");
    fake.pull(42, "the pr", "claude/x", "no link");
    fake.pull_file(42, "a.rs", "added", 1, 0, Some("@@ pr a"));
    fake.pull_file(42, "b.rs", "added", 2, 0, Some("@@ pr b"));
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
    fake
}

/// After the branch merged main, the delta lists only the PR's own files, with the PR's patches.
#[test]
fn delta_after_a_main_merge_keeps_the_prs_own_files() {
    let fake = setup();
    fake.compare_file("A", "B", "b.rs", "@@ merged");
    fake.compare_file("A", "B", "main_only.rs", "@@ main");
    let (code, v) = run(&fake, "s", T0, &["review-pack", "42"]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["since_sha"], "A", "{v}");
    let files = v["data"]["files"].as_array().unwrap();
    assert_eq!(files.len(), 1, "{v}");
    assert_eq!(files[0]["filename"], "b.rs");
    assert_eq!(files[0]["patch"], "@@ pr b");
    assert_eq!(files[0]["additions"], 2);
}

/// Only main changed since the last review: an empty but valid pack.
#[test]
fn delta_with_only_main_changes_is_empty() {
    let fake = setup();
    fake.compare_file("A", "B", "main_only.rs", "@@ main");
    let (code, v) = run(&fake, "s", T0, &["review-pack", "42"]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["files"].as_array().unwrap().len(), 0, "{v}");
    assert!(v["data"]["chunks"].is_array(), "{v}");
}
