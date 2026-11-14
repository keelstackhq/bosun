mod common;

use common::fake::FakeGithub;
use common::run;

const T0: &str = "2026-10-22T10:00:00Z";
const PROXY_403: &str = "Access to this GitHub API path is not permitted through this proxy.";

/// The cloud proxy answers 403 for the collaborators endpoint: no command may fail on it.
#[test]
fn no_command_fails_on_the_proxys_collaborators_403() {
    let fake = FakeGithub::start();
    fake.fail_path_message("/permission", 403, PROXY_403);
    let cmds: &[&[&str]] = &[
        &["join"],
        &["status"],
        &["reap"],
        &["budget", "check"],
        &["digest"],
        &["plan", "amend", "docs/superpowers/plans/none.md", "--dry-run"],
        &["heal"],
        &["wave", "tick", "1"],
    ];
    for args in cmds {
        let (_, v) = run(&fake, "session_a", T0, args);
        let msg = v["error"]["message"].as_str().unwrap_or("");
        assert!(
            !msg.contains("/permission") && !msg.contains("proxy"),
            "{args:?}: {v}"
        );
    }
}
