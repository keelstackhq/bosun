mod common;

use common::fake::FakeGithub;
use common::{lease_note, run};

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
        &[
            "plan",
            "amend",
            "docs/superpowers/plans/none.md",
            "--dry-run",
        ],
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

/// Behind the 403 another login is `unknown`, not a stranger: its lease still blocks a claim and
/// its spend still counts (dropping them would loosen the lease and spend limits).
#[test]
fn unknown_logins_still_hold_leases_and_spend() {
    let fake = FakeGithub::start();
    fake.fail_path_message("/permission", 403, PROXY_403);
    fake.viewer("me");
    fake.issue(10, "task", &["bosun:state/queued"], "Do it.");
    let live = lease_note("session_a", "2026-10-22T12:00:00Z", "src/a/**");
    fake.comment_by(10, "bob", &live);
    let (code, v) = run(&fake, "session_b", T0, &["claim", "10"]);
    assert_eq!(code, 3, "another login's live lease blocks the claim: {v}");

    fake.issue(1, "bosun registry", &["bosun:registry"], "");
    let spend = r#"{"bosun":"spend","session":"s","usd":150.0,"at":"2026-10-22T03:00:00Z"}"#;
    fake.comment_by(1, "bob", spend);
    let (code, v) = run(&fake, "session_b", T0, &["budget", "check"]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["spent_today_usd"], 150.0, "{v}");
}
