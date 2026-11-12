//! R-revert: a revert PR (marker `<!-- bosun:revert N -->`) is normal only when #N was not an
//! owner-class PR and the revert is its exact inverse (spec §8).
mod common;

use common::fake::FakeGithub;
use common::run;
use serde_json::Value;

const NOW: &str = "2026-10-22T10:00:00Z";
const OWNERS: &str = "[[owner]]\ngithub = \"ada\"\n";
const FWD: &str = "@@ -1,2 +1,2 @@\n ctx\n-old\n+new";
const BACK: &str = "@@ -1,2 +1,2 @@\n ctx\n-new\n+old";

/// Merged PR #2 changes `path` with `FWD`; PR #1 is its revert carrying `body` and `back`.
fn setup(body: &str, path: &str, back: &str) -> FakeGithub {
    let fake = FakeGithub::start();
    fake.content("governance/owners.toml", "base0000", OWNERS);
    fake.pull(2, "forward", "feature", "");
    fake.pull_merged(2, "merge2");
    fake.pull_file(2, path, "modified", 1, 1, Some(FWD));
    fake.pull(1, "Revert #2", "bosun/revert-2", body);
    fake.pull_file(1, path, "modified", 1, 1, Some(back));
    for n in [1, 2] {
        fake.pull_set(n, "changed_files", 1.into());
    }
    fake
}

fn rules(fake: &FakeGithub) -> (i32, Vec<String>, Value) {
    let (code, v) = run(fake, "session_test", NOW, &["risk-check", "1"]);
    let ids = v["data"]["matches"].as_array().cloned().unwrap_or_default();
    let ids = ids
        .iter()
        .map(|m| m["rule"].as_str().unwrap().into())
        .collect();
    (code, ids, v)
}

#[test]
fn an_exact_inverse_of_a_normal_pr_is_normal() {
    let fake = setup("<!-- bosun:revert 2 -->", "src/a.rs", BACK);
    let (code, ids, v) = rules(&fake);
    assert_eq!(code, 0, "{v}");
    assert!(!ids.contains(&"R-revert".into()), "{ids:?}");
}

#[test]
fn not_the_inverse_needs_the_owner() {
    let other = "@@ -1,2 +1,2 @@\n ctx\n-new\n+else";
    let fake = setup("<!-- bosun:revert 2 -->", "src/a.rs", other);
    let (code, ids, v) = rules(&fake);
    assert_eq!(code, 5, "{v}");
    assert!(ids.contains(&"R-revert".into()), "{ids:?}");
    let m = v["data"]["matches"].as_array().unwrap();
    let m = m.iter().find(|m| m["rule"] == "R-revert").unwrap();
    assert_eq!(m["approval"], "owner");
    assert!(
        m["evidence"]
            .to_string()
            .contains("not the exact inverse of #2"),
        "{m}"
    );
}

#[test]
fn reverting_an_owner_class_pr_needs_the_owner() {
    let fake = setup("<!-- bosun:revert 2 -->", "governance/x.toml", BACK);
    let (code, ids, v) = rules(&fake);
    assert_eq!(code, 5, "{v}");
    assert!(ids.contains(&"R-revert".into()), "{ids:?}");
    assert!(v.to_string().contains("#2 needed the owner"), "{v}");
}

#[test]
fn without_the_marker_the_normal_rules_apply() {
    let other = "@@ -1,2 +1,2 @@\n ctx\n-new\n+else";
    let fake = setup("Reverts #2", "src/a.rs", other);
    let (code, ids, v) = rules(&fake);
    assert_eq!(code, 0, "{v}");
    assert!(!ids.contains(&"R-revert".into()), "{ids:?}");
}

#[test]
fn a_wrong_target_needs_the_owner() {
    let fake = setup("<!-- bosun:revert 9 -->", "src/a.rs", BACK);
    let (code, ids, v) = rules(&fake);
    assert_eq!(code, 5, "{v}");
    assert!(ids.contains(&"R-revert".into()), "{ids:?}");
}

#[test]
fn a_linked_tasks_owner_capability_on_the_reverted_pr_needs_the_owner() {
    let fake = setup("<!-- bosun:revert 2 -->", "src/a.rs", BACK);
    fake.pull_set(2, "body", "Closes #7".into());
    let meta = "<!-- bosun:task {\"capabilities\":[\"prod-write\"]} -->";
    fake.issue(7, "task", &[], meta);
    let (code, ids, v) = rules(&fake);
    assert_eq!(code, 5, "{v}");
    assert!(ids.contains(&"R-revert".into()), "{ids:?}");
    assert!(
        v.to_string().contains("#2 needed the owner (R-prod-write)"),
        "{v}"
    );
}

#[test]
fn a_reverted_manifest_change_counts_as_a_new_dependency() {
    let fake = setup("<!-- bosun:revert 2 -->", "web/package.json", BACK);
    let (_, ids, v) = rules(&fake);
    assert!(ids.contains(&"R-revert".into()), "{ids:?}");
    assert!(
        v.to_string().contains("#2 needed the owner (R-deps)"),
        "{v}"
    );
}
