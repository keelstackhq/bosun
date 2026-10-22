//! The pull request methods of `Github` against the fake.

mod common;

use bosun::github::{Github, IssueState, LiveGithub};
use common::TOKEN_CANARY;
use common::fake::FakeGithub;
use serde_json::json;

fn setup() -> (FakeGithub, LiveGithub) {
    let fake = FakeGithub::start();
    let gh = LiveGithub::with(&fake.url(), TOKEN_CANARY, "acme", "widget", 0);
    (fake, gh)
}

#[test]
fn get_pull_maps_refs_flags_and_labels() {
    let (fake, gh) = setup();
    assert!(gh.get_pull(9).unwrap().is_none());
    fake.pull(7, "feat: x", "claude/x", "the body");
    fake.pull_set(7, "draft", json!(true));
    fake.pull_set(7, "merged", json!(true));
    fake.pull_set(7, "labels", json!([{"name": "bosun:state/review"}]));
    let p = gh.get_pull(7).unwrap().unwrap();
    assert_eq!((p.number, p.node_id.as_str()), (7, "PR_node7"));
    assert_eq!(
        (p.head_sha.as_str(), p.head_ref.as_str()),
        ("sha7", "claude/x")
    );
    assert_eq!(
        (p.base_sha.as_str(), p.base_ref.as_str()),
        ("base0000", "main")
    );
    assert_eq!((p.title.as_str(), p.body.as_str()), ("feat: x", "the body"));
    assert!(p.merged && p.draft);
    assert_eq!((p.state.as_str(), p.user.as_str()), ("open", "octocat"));
    assert_eq!(p.labels, vec!["bosun:state/review"]);
}

#[test]
fn null_body_is_empty() {
    let (fake, gh) = setup();
    fake.pull(1, "t", "b", "");
    fake.pull_set(1, "body", json!(null));
    assert_eq!(gh.get_pull(1).unwrap().unwrap().body, "");
}

#[test]
fn list_pulls_filters_head_and_state_across_pages() {
    let (fake, gh) = setup();
    for n in 1..=5 {
        fake.pull(n, "t", &format!("claude/b{n}"), "");
    }
    fake.pull_set(5, "state", json!("closed"));
    let open = gh.list_pulls(None, IssueState::Open).unwrap();
    assert_eq!(
        open.iter().map(|p| p.number).collect::<Vec<_>>(),
        [1, 2, 3, 4]
    );
    assert!(open.iter().all(|p| !p.merged));
    assert_eq!(gh.list_pulls(None, IssueState::All).unwrap().len(), 5);
    let one = gh
        .list_pulls(Some("acme:claude/b3"), IssueState::Open)
        .unwrap();
    assert_eq!(one.iter().map(|p| p.number).collect::<Vec<_>>(), [3]);
    assert!(
        gh.list_pulls(Some("acme:nope"), IssueState::Open)
            .unwrap()
            .is_empty()
    );
    // the head filter is percent-encoded on the wire
    assert!(fake.requests().iter().all(|r| !r.path.contains(' ')));
}

#[test]
fn pull_files_reads_every_page() {
    let (fake, gh) = setup();
    fake.pull(1, "t", "b", "");
    fake.pull_file(1, "src/a.rs", "modified", 3, 1, Some("@@ -1 +1 @@"));
    fake.pull_file(1, "src/b.rs", "added", 10, 0, Some("@@"));
    fake.pull_file(1, "img.png", "added", 0, 0, None);
    fake.pull_file(1, "src/c.rs", "renamed", 0, 0, None);
    fake.renamed_from(1, "src/old.rs");
    fake.pull_file(1, "src/d.rs", "removed", 0, 7, Some("@@"));
    let files = gh.pull_files(1).unwrap();
    assert_eq!(files.len(), 5);
    assert_eq!(
        (
            files[0].filename.as_str(),
            files[0].additions,
            files[0].deletions
        ),
        ("src/a.rs", 3, 1)
    );
    assert_eq!(files[0].patch.as_deref(), Some("@@ -1 +1 @@"));
    assert_eq!(files[2].patch, None);
    assert_eq!(files[3].previous_filename.as_deref(), Some("src/old.rs"));
    assert_eq!(files[3].status, "renamed");
    assert!(gh.pull_files(99).unwrap().is_empty());
}

#[test]
fn create_pull_and_update_body() {
    let (fake, gh) = setup();
    fake.issue(1, "an issue", &[], "");
    let p = gh
        .create_pull("feat: y", "claude/y", "main", "first")
        .unwrap();
    assert_eq!(
        (p.number, p.head_ref.as_str(), p.base_ref.as_str()),
        (2, "claude/y", "main")
    );
    assert_eq!(p.body, "first");
    gh.update_pull_body(2, "second").unwrap();
    assert_eq!(fake.pull_body(2), "second");
    assert_eq!(gh.get_pull(2).unwrap().unwrap().body, "second");
    assert!(gh.update_pull_body(99, "x").is_err());
}

#[test]
fn list_reviews_reads_every_page() {
    let (fake, gh) = setup();
    fake.pull(1, "t", "b", "");
    fake.review(1, "alice", "CHANGES_REQUESTED", "sha-old");
    fake.review(1, "bob", "COMMENTED", "sha-old");
    fake.review(1, "alice", "APPROVED", "sha1");
    let r = gh.list_reviews(1).unwrap();
    assert_eq!(r.len(), 3);
    assert_eq!(
        (
            r[2].user.as_str(),
            r[2].state.as_str(),
            r[2].commit_id.as_str()
        ),
        ("alice", "APPROVED", "sha1")
    );
    assert!(gh.list_reviews(2).unwrap().is_empty());
}
