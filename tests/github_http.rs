//! The HTTP client against the fake GitHub, without the `Github` trait.

mod common;

use bosun::github::http::Http;
use common::TOKEN_CANARY;
use common::fake::FakeGithub;
use serde_json::{Value, json};

const ISSUE: &str = "/repos/acme/widget/issues/1";
const COMMENTS: &str = "/repos/acme/widget/issues/1/comments?per_page=100";

fn setup() -> (FakeGithub, Http) {
    let fake = FakeGithub::start();
    let http = Http::new(&fake.url(), TOKEN_CANARY, 0);
    (fake, http)
}

#[test]
fn get_json_reads_and_maps_404_to_none() {
    let (fake, http) = setup();
    fake.issue(1, "t", &[], "b");
    assert_eq!(http.get_json(ISSUE).unwrap().unwrap()["title"], "t");
    assert!(
        http.get_json("/repos/acme/widget/issues/9")
            .unwrap()
            .is_none()
    );
}

#[test]
fn send_json_writes() {
    let (fake, http) = setup();
    fake.issue(1, "t", &[], "");
    let v = http
        .send_json("PATCH", ISSUE, &json!({"title": "new"}))
        .unwrap();
    assert_eq!(v["title"], "new");
    assert_eq!(fake.issue_state(1).title, "new");
}

#[test]
fn get_all_follows_links_across_three_pages() {
    let (fake, http) = setup();
    fake.issue(1, "t", &[], "");
    for i in 0..5 {
        fake.comment(1, &format!("c{i}"));
    }
    let items = http.get_all(COMMENTS).unwrap();
    let bodies: Vec<&str> = items.iter().map(|c| c["body"].as_str().unwrap()).collect();
    assert_eq!(bodies, ["c0", "c1", "c2", "c3", "c4"]);
    let pages = fake
        .requests()
        .iter()
        .filter(|r| r.path.ends_with("/comments"))
        .count();
    assert_eq!(pages, 3);
}

#[test]
fn a_500_then_200_is_retried() {
    let (fake, http) = setup();
    fake.issue(1, "t", &[], "");
    fake.fail_next(500, 1);
    assert_eq!(http.get_json(ISSUE).unwrap().unwrap()["number"], 1);
    assert_eq!(fake.requests().len(), 2);
}

#[test]
fn persistent_500_gives_up_after_three_attempts() {
    let (fake, http) = setup();
    fake.fail_next(500, 10);
    let e = http.get_json(ISSUE).unwrap_err();
    assert_eq!(e.message, format!("GitHub GET {ISSUE}: 500 fake failure"));
    assert_eq!(fake.requests().len(), 3);
}

#[test]
fn posts_are_not_retried() {
    let (fake, http) = setup();
    fake.fail_next(500, 1);
    let body: Value = json!({"title": "t"});
    assert!(
        http.send_json("POST", "/repos/acme/widget/issues", &body)
            .is_err()
    );
    assert_eq!(fake.requests().len(), 1);
}

#[test]
fn errors_say_what_failed_and_never_carry_the_token() {
    let (fake, http) = setup();
    let e = http.send_json("PATCH", ISSUE, &json!({})).unwrap_err();
    assert_eq!(e.message, format!("GitHub PATCH {ISSUE}: 404 Not Found"));
    assert_eq!(e.fix, "check BOSUN_REPO and the issue/PR number");

    fake.fail_next(403, 1);
    let e = http.send_json("POST", ISSUE, &json!({})).unwrap_err();
    assert_eq!(e.fix, "the token lacks permission or hit a rate limit");

    fake.fail_next(401, 1);
    let e = http.get_json(ISSUE).unwrap_err();
    assert_eq!(e.message, format!("GitHub GET {ISSUE}: 401 fake failure"));
    assert_eq!(e.fix, "the token was rejected; check GITHUB_TOKEN");

    let dead = Http::new("http://127.0.0.1:9", TOKEN_CANARY, 0);
    let e = dead.get_json(ISSUE).unwrap_err();
    for text in [&e.message, &e.fix, &e.to_string(), &format!("{dead:?}")] {
        assert!(!text.contains(TOKEN_CANARY), "{text}");
    }
}

#[test]
fn requests_carry_the_documented_headers() {
    let (fake, http) = setup();
    http.get_json(ISSUE).unwrap();
    let [auth, accept, version, agent] = fake.requests()[0].headers.clone();
    assert_eq!(auth.unwrap(), format!("Bearer {TOKEN_CANARY}"));
    assert_eq!(accept.unwrap(), "application/vnd.github+json");
    assert_eq!(version.unwrap(), "2022-11-28");
    assert!(agent.unwrap().starts_with("bosun/"));
}

#[test]
fn requests_without_authorization_are_401() {
    let fake = FakeGithub::start();
    let url = format!("{}{ISSUE}", fake.url());
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .http_status_as_error(false)
        .build()
        .into();
    assert_eq!(agent.get(&url).call().unwrap().status().as_u16(), 401);
}
