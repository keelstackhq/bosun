//! R-bundle counts closing refs in the PR title and the commit messages, not only the body.

mod common;

use common::fake::FakeGithub;
use common::run;
use serde_json::Value;

const NOW: &str = "2026-10-22T10:00:00Z";
const BASE: &str = "base0000";

fn check(title: &str, body: &str, commits: &[(&str, &str)]) -> Value {
    let fake = FakeGithub::start();
    fake.pull(1, title, "feature", body);
    fake.content(
        "governance/owners.toml",
        BASE,
        "[[owner]]\ngithub = \"ada\"\n",
    );
    fake.pull_file(
        1,
        "hoist.toml",
        "modified",
        1,
        0,
        Some("@@ -0,0 +1,1 @@\n+x"),
    );
    fake.pull_commit_shas(1, &commits.iter().map(|c| c.0).collect::<Vec<_>>());
    for (sha, msg) in commits {
        fake.git_commit_message(sha, msg);
    }
    run(&fake, "session_test", NOW, &["risk-check", "1"]).1
}

fn bundle(v: &Value) -> bool {
    v["data"]["matches"]
        .as_array()
        .is_some_and(|a| a.iter().any(|m| m["rule"] == "R-bundle"))
}

#[test]
fn title_ref_plus_body_ref_is_r_bundle() {
    assert!(bundle(&check("fix: x (closes #6)", "Closes #5", &[])));
}

#[test]
fn commit_ref_plus_body_ref_is_r_bundle() {
    assert!(bundle(&check(
        "x",
        "Closes #5",
        &[("c1", "wip\n\nFixes #7")]
    )));
}

#[test]
fn same_issue_in_title_and_commit_counts_once() {
    assert!(!bundle(&check(
        "x (closes #5)",
        "Closes #5",
        &[("c1", "Fixes #5")]
    )));
}

#[test]
fn another_repos_ref_in_the_title_does_not_count() {
    assert!(!bundle(&check(
        "x (fixes keelstackhq/splice#40)",
        "Closes #5",
        &[]
    )));
}

#[test]
fn unreadable_commit_fails_closed() {
    let fake = FakeGithub::start();
    fake.pull(1, "x", "feature", "Closes #5");
    fake.content(
        "governance/owners.toml",
        BASE,
        "[[owner]]\ngithub = \"ada\"\n",
    );
    fake.pull_file(
        1,
        "hoist.toml",
        "modified",
        1,
        0,
        Some("@@ -0,0 +1,1 @@\n+x"),
    );
    fake.pull_commit_shas(1, &["gone"]);
    let v = run(&fake, "session_test", NOW, &["risk-check", "1"]).1;
    assert!(bundle(&v), "{v}");
}
