//! `spawn --record` decides whether the spawner is registered from its joins only: a read
//! user's forged `leave` for the spawner must not make the record fail.
mod common;

use common::fake::FakeGithub;
use common::{bosun, lease_note, one_object};

const LATER: &str = "2026-10-22T12:00:00Z";

#[test]
fn a_forged_leave_does_not_unregister_the_spawner() {
    let fake = FakeGithub::start();
    fake.issue(1, "bosun registry", &["bosun:registry"], "");
    fake.comment(
        1,
        &format!(
            r#"{{"bosun":"join","session":"session_test","origin":"spawned","depth":0,"expires":"{LATER}"}}"#
        ),
    );
    fake.permission("mallory", "read");
    fake.comment_by(
        1,
        "mallory",
        r#"{"bosun":"leave","session":"session_test","reason":"released"}"#,
    );
    fake.issue(
        10,
        "Lifecycle epic",
        &["bosun:state/ready"],
        "<!-- bosun:task {\"item\":\"E1\",\"task\":\"E1\"} -->",
    );
    fake.comment(10, &lease_note("session_test", LATER, "x"));
    let out = bosun()
        .env("BOSUN_GITHUB_API", fake.url())
        .args(["spawn", "10", "--record", "--from", "-"])
        .write_stdin(r#"{"id":"session_child"}"#)
        .output()
        .unwrap();
    let v = one_object(&out.stdout);
    assert_eq!(out.status.code(), Some(0), "{v}");
}
