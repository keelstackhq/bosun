mod common;

use common::fake::FakeGithub;
use common::run;

const NOW: &str = "2026-10-22T10:00:00Z";
const OWNERS: &str = "[[owner]]\ngithub = \"ada\"\n\n[[owner]]\ngithub = \"bo\"\nslack = \"U2\"\n";

#[test]
fn ask_mentions_the_owners_and_sets_needs_human() {
    let fake = FakeGithub::start();
    fake.issue(4, "task", &["bosun:state/building"], "body");
    fake.content("governance/owners.toml", "main", OWNERS);
    let (code, v) = run(
        &fake,
        "session_a",
        NOW,
        &["ask", "4", "May I drop the old API?"],
    );
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["issue"], 4);
    assert_eq!(v["data"]["owners"], serde_json::json!(["ada", "bo"]));
    let comments = fake.comments_of(4);
    assert_eq!(comments.len(), 1);
    assert_eq!(v["data"]["comment_id"], comments[0].id);
    let body = &comments[0].body;
    assert!(
        body.starts_with("@ada @bo May I drop the old API?"),
        "{body}"
    );
    assert!(
        body.contains(
            r#"{"bosun":"ask","session":"session_a","question":"May I drop the old API?"}"#
        )
    );
    assert_eq!(fake.labels_of(4), ["bosun:state/needs-human"]);
}

#[test]
fn ask_without_owners_is_an_error() {
    let fake = FakeGithub::start();
    fake.issue(4, "task", &["bosun:state/building"], "body");
    let (code, v) = run(&fake, "session_a", NOW, &["ask", "4", "Why?"]);
    assert_eq!(code, 1, "{v}");
    assert!(
        v["error"]["fix"]
            .as_str()
            .unwrap()
            .contains("governance/owners.toml")
    );
    assert!(fake.comments_of(4).is_empty());
}

#[test]
fn ask_respects_the_kill_switch() {
    let fake = FakeGithub::start();
    fake.issue(4, "task", &["bosun:state/building"], "body");
    fake.content("governance/owners.toml", "main", OWNERS);
    fake.variable("BOSUN_HALT", "1");
    assert_eq!(run(&fake, "session_a", NOW, &["ask", "4", "Why?"]).0, 4);
}

#[test]
fn ask_on_a_missing_issue_is_an_error() {
    let fake = FakeGithub::start();
    fake.content("governance/owners.toml", "main", OWNERS);
    assert_eq!(run(&fake, "session_a", NOW, &["ask", "9", "Why?"]).0, 1);
}
