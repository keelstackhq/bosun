//! Each `Github` method against the fake, plus pagination, retries and error messages.

mod common;

use bosun::github::{Github, IssuePatch, IssueState, LiveGithub};
use common::TOKEN_CANARY;
use common::fake::FakeGithub;

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
            state: Some(IssueState::Closed),
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
                state: Some(IssueState::Closed),
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
fn lists_follow_links_across_three_pages() {
    let (fake, gh) = setup();
    fake.issue(1, "t", &[], "");
    for i in 0..5 {
        fake.comment(1, &format!("c{i}"));
    }
    let got = gh.list_comments(1).unwrap();
    assert_eq!(
        got.iter().map(|c| c.body.as_str()).collect::<Vec<_>>(),
        ["c0", "c1", "c2", "c3", "c4"]
    );
    let pages = fake
        .requests()
        .iter()
        .filter(|r| r.path.ends_with("/comments"))
        .count();
    assert_eq!(pages, 3);
}

#[test]
fn variable_found_and_missing() {
    let (fake, gh) = setup();
    fake.variable("BOSUN_HALT", "1");
    assert_eq!(gh.variable("BOSUN_HALT").unwrap().as_deref(), Some("1"));
    assert_eq!(gh.variable("NOPE").unwrap(), None);
}

#[test]
fn a_500_then_200_is_retried() {
    let (fake, gh) = setup();
    fake.issue(1, "t", &[], "");
    fake.fail_next(500, 1);
    assert_eq!(gh.get_issue(1).unwrap().unwrap().number, 1);
    assert_eq!(fake.requests().len(), 2);
}

#[test]
fn persistent_500_gives_up_after_three_attempts() {
    let (fake, gh) = setup();
    fake.fail_next(500, 10);
    let e = gh.get_issue(1).unwrap_err();
    assert_eq!(
        e.message,
        "GitHub GET /repos/acme/widget/issues/1: 500 fake failure"
    );
    assert_eq!(fake.requests().len(), 3);
}

#[test]
fn posts_are_not_retried() {
    let (fake, gh) = setup();
    fake.fail_next(500, 1);
    assert!(gh.create_issue("t", "", &[]).is_err());
    assert_eq!(fake.requests().len(), 1);
}

#[test]
fn errors_say_what_failed_and_never_carry_the_token() {
    let (fake, gh) = setup();
    let e = gh.update_issue(7, IssuePatch::default()).unwrap_err();
    assert_eq!(
        e.message,
        "GitHub PATCH /repos/acme/widget/issues/7: 404 Not Found"
    );
    assert_eq!(e.fix, "check BOSUN_REPO and the issue/PR number");
    fake.fail_next(403, 1);
    let e = gh.add_comment(1, "x").unwrap_err();
    assert_eq!(e.fix, "the token lacks permission or hit a rate limit");

    fake.fail_next(401, 1);
    let e = gh.get_issue(1).unwrap_err();
    assert_eq!(
        e.message,
        "GitHub GET /repos/acme/widget/issues/1: 401 fake failure"
    );
    assert_eq!(e.fix, "the token was rejected; check GITHUB_TOKEN");

    let dead = LiveGithub::with("http://127.0.0.1:9", TOKEN_CANARY, "acme", "widget", 0);
    let e = dead.get_issue(1).unwrap_err();
    for text in [&e.message, &e.fix, &e.to_string()] {
        assert!(!text.contains(TOKEN_CANARY), "{text}");
    }
}

#[test]
fn requests_carry_the_documented_headers() {
    let (fake, gh) = setup();
    gh.get_issue(1).unwrap();
    let [auth, accept, version, agent] = fake.requests()[0].headers.clone();
    assert_eq!(auth.unwrap(), format!("Bearer {TOKEN_CANARY}"));
    assert_eq!(accept.unwrap(), "application/vnd.github+json");
    assert_eq!(version.unwrap(), "2022-11-28");
    assert!(agent.unwrap().starts_with("bosun/"));
}

#[test]
fn requests_without_authorization_are_401() {
    let fake = FakeGithub::start();
    let url = format!("{}/repos/a/b/issues/1", fake.url());
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .http_status_as_error(false)
        .build()
        .into();
    assert_eq!(agent.get(&url).call().unwrap().status().as_u16(), 401);
}
