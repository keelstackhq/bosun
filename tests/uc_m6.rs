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
}
