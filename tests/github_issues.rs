//! Each `Github` method against the fake, plus pagination, retries and error messages.

mod common;

use bosun::github::{Github, IssuePatch, IssueState, LiveGithub, PatchState, Variable};
use common::TOKEN_CANARY;
use common::fake::FakeGithub;
use serde_json::json;

fn setup() -> (FakeGithub, LiveGithub) {
    let fake = FakeGithub::start();
    let gh = LiveGithub::with(&fake.url(), TOKEN_CANARY, "acme", "widget", 0);
    (fake, gh)
}

#[test]
fn list_issues_filters_labels_state_and_pull_requests() {
    let (fake, gh) = setup();
    fake.issue(1, "one", &["bosun:state/ready", "bug"], "b1");
    fake.issue(2, "two", &["bug"], "");
    fake.pull_as_issue(3, "a pr");
    fake.issue(4, "four", &["bosun:state/ready"], "");
    gh.update_issue(
        4,
        IssuePatch {
            state: Some(PatchState::Closed),
            ..Default::default()
        },
    )
    .unwrap();
    let got = gh
        .list_issues(&["bosun:state/ready"], IssueState::Open)
        .unwrap();
    assert_eq!(got.iter().map(|i| i.number).collect::<Vec<_>>(), vec![1]);
    assert_eq!(got[0].labels, vec!["bosun:state/ready", "bug"]);
    assert_eq!(gh.list_issues(&[], IssueState::All).unwrap().len(), 3);
    assert_eq!(gh.list_issues(&["bug"], IssueState::Open).unwrap().len(), 2);
    // label names are percent-encoded on the wire
    let reqs = fake.requests();
    assert!(reqs.iter().all(|r| !r.path.contains(' ')));
}

#[test]
fn issue_get_create_update() {
    let (fake, gh) = setup();
    assert!(gh.get_issue(9).unwrap().is_none());
    let made = gh.create_issue("title", "body", &["a:b/c"]).unwrap();
    assert_eq!(
        (made.number, made.body_text(), made.state.as_str()),
        (1, "body", "open")
    );
    assert_eq!(fake.labels_of(1), vec!["a:b/c"]);
    let patched = gh
        .update_issue(
            1,
            IssuePatch {
                title: Some("new".into()),
                body: Some("nb".into()),
                state: Some(PatchState::Closed),
                state_reason: Some("completed".into()),
            },
        )
        .unwrap();
    assert_eq!(
        (patched.title.as_str(), patched.state.as_str()),
        ("new", "closed")
    );
    assert_eq!(fake.issue_state(1).body, "nb");
    assert_eq!(gh.get_issue(1).unwrap().unwrap().title, "new");
}

#[test]
fn labels_add_remove_ensure() {
    let (fake, gh) = setup();
    fake.issue(1, "t", &["x"], "");
    gh.add_labels(1, &["bosun:state/ready", "y"]).unwrap();
    assert_eq!(fake.labels_of(1), vec!["x", "bosun:state/ready", "y"]);
    gh.remove_label(1, "bosun:state/ready").unwrap();
    gh.remove_label(1, "bosun:state/ready").unwrap(); // already gone: fine
    assert_eq!(fake.labels_of(1), vec!["x", "y"]);
    gh.ensure_label("bosun:state/ready", "0e8a16", "ready")
        .unwrap();
    gh.ensure_label("bosun:state/ready", "0e8a16", "ready")
        .unwrap(); // 422 already_exists: fine
    assert_eq!(fake.label_names(), vec!["bosun:state/ready"]);
}

#[test]
fn comments_add_update_list() {
    let (fake, gh) = setup();
    fake.issue(1, "t", &[], "");
    let c = gh.add_comment(1, "hello").unwrap();
    assert_eq!(
        (c.id, c.body.as_str(), c.user.as_str()),
        (101, "hello", "octocat")
    );
    let u = gh.update_comment(c.id, "edited").unwrap();
    assert_eq!(u.body, "edited");
    assert_ne!(u.updated_at, u.created_at);
    assert_eq!(fake.comments_of(1)[0].body, "edited");
    assert_eq!(gh.add_comment(1, "second").unwrap().id, 102);
    assert_eq!(gh.list_comments(1).unwrap().len(), 2);
}

#[test]
fn variable_found_and_missing() {
    let (fake, gh) = setup();
    fake.variable("BOSUN_HALT", "1");
    assert_eq!(
        gh.variable("BOSUN_HALT").unwrap(),
        Variable::Set("1".into())
    );
    assert_eq!(gh.variable("NOPE").unwrap(), Variable::Unset);
    fake.fail_path("/actions/variables/HIDDEN", 403);
    assert_eq!(gh.variable("HIDDEN").unwrap(), Variable::Forbidden);
}

const PROXY_403: &str = "Access to this GitHub API path is not permitted through this proxy.";

fn proxy_blocks_collaborators(fake: &FakeGithub, me: &str) {
    fake.viewer(me);
    fake.fail_path_message("/permission", 403, PROXY_403);
}

#[test]
fn permission_of_self_comes_from_the_repo_when_collaborators_is_blocked() {
    let (fake, gh) = setup();
    proxy_blocks_collaborators(&fake, "me");
    for (perms, want) in [
        (
            json!({"admin": true, "maintain": true, "push": true, "pull": true}),
            "admin",
        ),
        (
            json!({"admin": false, "maintain": true, "push": true, "pull": true}),
            "write",
        ),
        (json!({"admin": false, "push": true, "pull": true}), "write"),
        (
            json!({"admin": false, "push": false, "triage": true, "pull": true}),
            "read",
        ),
        (json!({"admin": false, "push": false, "pull": true}), "read"),
    ] {
        fake.repo_set("permissions", perms);
        assert_eq!(gh.permission("me").unwrap().as_deref(), Some(want));
    }
    fake.repo_set(
        "permissions",
        json!({"admin": false, "push": false, "pull": false}),
    );
    assert_eq!(gh.permission("me").unwrap(), None);
    // the login is looked up once per client
    let users = fake.requests().iter().filter(|r| r.path == "/user").count();
    assert_eq!(users, 1);
}

#[test]
fn permission_of_another_user_degrades_to_unknown_when_collaborators_is_blocked() {
    let (fake, gh) = setup();
    proxy_blocks_collaborators(&fake, "me");
    fake.repo_set(
        "permissions",
        json!({"admin": true, "push": true, "pull": true}),
    );
    // not an error and never write: `unknown`
    assert_eq!(
        gh.permission("someone").unwrap().as_deref(),
        Some("unknown")
    );
}

#[test]
fn permission_uses_collaborators_when_it_answers() {
    let (fake, gh) = setup();
    fake.viewer("me");
    fake.permission("me", "read");
    fake.repo_set(
        "permissions",
        json!({"admin": true, "push": true, "pull": true}),
    );
    assert_eq!(gh.permission("me").unwrap().as_deref(), Some("read"));
    fake.permission("other", "admin");
    assert_eq!(gh.permission("other").unwrap().as_deref(), Some("admin"));
    assert!(fake.requests().iter().all(|r| r.path != "/user"));
}

#[test]
fn the_blocked_collaborators_path_is_asked_once_per_client() {
    let (fake, gh) = setup();
    proxy_blocks_collaborators(&fake, "me");
    for u in ["a", "b", "c"] {
        assert_eq!(gh.permission(u).unwrap().as_deref(), Some("unknown"));
    }
    let asked = fake
        .requests()
        .iter()
        .filter(|r| r.path.ends_with("/permission"))
        .count();
    assert_eq!(asked, 1);
}
