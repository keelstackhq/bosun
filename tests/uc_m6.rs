//! Use cases of M6 (spec §12): Slack.
mod common;

use common::fake::FakeGithub;
use common::{bosun, one_object};
use serde_json::json;

/// UC-M6-1: notify refuses a shared channel or a channel with guests.
#[test]
fn uc_m6_1() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join(".bosun")).unwrap();
    std::fs::write(
        dir.path().join(".bosun/bosun.toml"),
        "[slack]\nchannel = \"C1\"\n",
    )
    .unwrap();
    let gh = FakeGithub::start();
    gh.issue(3, "main red", &["bosun:main-red"], "");
    for (info, reason) in [
        (
            json!({"id":"C1","is_ext_shared":true,"is_org_shared":false,"is_shared":true,"guests":0}),
            "channel shared",
        ),
        (
            json!({"id":"C1","is_ext_shared":false,"is_org_shared":false,"is_shared":false,"guests":1}),
            "channel has guests",
        ),
    ] {
        let out = bosun()
            .current_dir(dir.path())
            .env("BOSUN_GITHUB_API", gh.url())
            .args(["notify", "--channel-info", "-"])
            .write_stdin(info.to_string())
            .output()
            .unwrap();
        let v = one_object(&out.stdout);
        assert_eq!(out.status.code(), Some(5), "{v}");
        assert_eq!(v["data"]["reason"], reason);
        assert_eq!(v["next"], json!([]), "nothing to post");
    }

    // The contrast: a private channel without guests gets its post.
    let good =
        json!({"id":"C1","is_ext_shared":false,"is_org_shared":false,"is_shared":false,"guests":0});
    let out = bosun()
        .current_dir(dir.path())
        .env("BOSUN_GITHUB_API", gh.url())
        .args(["notify", "--channel-info", "-"])
        .write_stdin(good.to_string())
        .output()
        .unwrap();
    let v = one_object(&out.stdout);
    assert_eq!(out.status.code(), Some(0), "{v}");
    assert_eq!(v["next"][0]["tool"], "slack_post");
    assert_eq!(v["next"][0]["args"]["channel"], "C1");
}

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
