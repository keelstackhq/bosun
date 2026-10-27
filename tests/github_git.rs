//! Git commit, recursive tree and merge-base reads against the fake.

mod common;

use bosun::github::{Github, LiveGithub};
use common::TOKEN_CANARY;
use common::fake::FakeGithub;
use serde_json::json;

fn setup() -> (FakeGithub, LiveGithub) {
    let fake = FakeGithub::start();
    let gh = LiveGithub::with(&fake.url(), TOKEN_CANARY, "acme", "widget", 0);
    (fake, gh)
}

#[test]
fn commit_reads_tree_and_parents_and_maps_404_to_none() {
    let (fake, gh) = setup();
    fake.git_commit("c1", "t1", &["p1", "p2"]);
    let c = gh.commit("c1").unwrap().unwrap();
    assert_eq!(
        (c.sha.as_str(), c.tree.as_str(), c.parents),
        ("c1", "t1", vec!["p1".to_string(), "p2".to_string()])
    );
    assert_eq!(gh.commit("nope").unwrap(), None);
}

#[test]
fn tree_reads_entries_recursively_and_maps_404_to_none() {
    let (fake, gh) = setup();
    fake.git_tree(
        "t1",
        &[
            ("src", "040000", "tree", "s1"),
            ("src/a.rs", "100644", "blob", "b1"),
            ("vendor/x", "160000", "commit", "m1"),
        ],
        false,
    );
    let t = gh.tree("t1").unwrap().unwrap();
    assert_eq!((t.sha.as_str(), t.truncated), ("t1", false));
    let got: Vec<_> = t
        .entries
        .iter()
        .map(|e| {
            (
                e.path.as_str(),
                e.mode.as_str(),
                e.kind.as_str(),
                e.sha.as_str(),
            )
        })
        .collect();
    assert_eq!(
        got,
        [
            ("src", "040000", "tree", "s1"),
            ("src/a.rs", "100644", "blob", "b1"),
            ("vendor/x", "160000", "commit", "m1"),
        ]
    );
    assert_eq!(gh.tree("nope").unwrap(), None);
}

#[test]
fn tree_reports_truncation() {
    let (fake, gh) = setup();
    fake.git_tree("t2", &[("a", "100644", "blob", "b")], true);
    assert!(gh.tree("t2").unwrap().unwrap().truncated);
}

#[test]
fn tree_entry_missing_a_field_is_an_error() {
    let (fake, gh) = setup();
    fake.git_tree_raw(
        "t3",
        json!({"sha": "t3", "truncated": false,
            "tree": [{"path": "a", "type": "blob", "sha": "b"}]}),
    );
    assert!(gh.tree("t3").is_err());
    fake.git_tree_raw("t4", json!({"sha": "t4", "tree": []}));
    assert!(
        gh.tree("t4").is_err(),
        "a missing truncated flag is an error"
    );
}

#[test]
fn compare_parses_the_merge_base() {
    let (fake, gh) = setup();
    fake.compare_file("main", "feat", "a.rs", "@@");
    assert_eq!(gh.compare("main", "feat").unwrap().merge_base, None);
    fake.compare_merge_base("main", "feat", "mb1");
    let c = gh.compare("main", "feat").unwrap();
    assert_eq!(c.merge_base.as_deref(), Some("mb1"));
    assert!(!c.truncated);
}

#[test]
fn compare_is_truncated_at_the_300_file_cap() {
    let (fake, gh) = setup();
    for i in 0..299 {
        fake.compare_file("main", "small", &format!("f{i}.rs"), "@@");
    }
    for i in 0..300 {
        fake.compare_file("main", "big", &format!("f{i}.rs"), "@@");
    }
    let small = gh.compare("main", "small").unwrap();
    assert_eq!((small.files.len(), small.truncated), (299, false));
    let big = gh.compare("main", "big").unwrap();
    assert_eq!((big.files.len(), big.truncated), (300, true));
}
