mod common;

use common::bosun;
use common::fake::FakeGithub;

const NOW: &str = "2026-10-23T10:00:00Z";

fn result(fake: &FakeGithub, cloud: bool) -> i32 {
    let mut cmd = bosun();
    cmd.env("BOSUN_GITHUB_API", fake.url())
        .env("BOSUN_SESSION", "session_x")
        .env("BOSUN_NOW", NOW)
        .args(["result", "9", "--outcome", "blocked", "--pr", "42"]);
    if cloud {
        cmd.env("CCR_AGENT_PROXY_ENABLED", "1");
    }
    cmd.output().unwrap().status.code().unwrap()
}

fn asked_collaborators(fake: &FakeGithub) -> bool {
    fake.requests()
        .iter()
        .any(|r| r.path.contains("/collaborators/"))
}

#[test]
fn a_cloud_session_never_asks_for_collaborator_permissions() {
    let fake = FakeGithub::start();
    fake.issue(9, "Epic", &["bosun:state/claimed"], "");
    fake.pull(42, "work", "claude/t9", "Closes #9");
    assert_eq!(result(&fake, true), 0);
    assert!(!asked_collaborators(&fake));
}

#[test]
fn outside_the_cloud_the_same_run_does_ask() {
    let fake = FakeGithub::start();
    fake.issue(9, "Epic", &["bosun:state/claimed"], "");
    fake.pull(42, "work", "claude/t9", "Closes #9");
    assert_eq!(result(&fake, false), 0);
    assert!(asked_collaborators(&fake));
}
