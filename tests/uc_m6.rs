//! Use cases of M6 (spec §12): Slack.
mod common;

use common::bosun;
use common::fake::FakeGithub;
use common::one_object;

/// UC-M6-2: an answer from a non-owner Slack ID does not count.
#[test]
fn uc_m6_2() {
    let fake = FakeGithub::start();
    fake.issue(4, "task", &["bosun:state/needs-human"], "body");
    fake.comment(4, r#"{"bosun":"ask","session":"s","question":"Why?"}"#);
    fake.content(
        "governance/owners.toml",
        "main",
        "[[owner]]\ngithub = \"ada\"\nslack = \"U0ADA1\"\n",
    );
    let out = bosun()
        .env("BOSUN_GITHUB_API", fake.url())
        .args(["answer", "4", "--slack-user", "U0OTHER"])
        .write_stdin("Yes.")
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(5));
    assert_eq!(
        one_object(&out.stdout)["data"]["reason"],
        "not an owner Slack ID"
    );
    assert_eq!(fake.labels_of(4), ["bosun:state/needs-human"]);
    assert_eq!(fake.comments_of(4).len(), 1);
}
