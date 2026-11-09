//! `spawn --record` reads the spawner's own join from every comment (it must work when the
//! permission lookup fails), so a forged join can only raise the child's depth, never lower it.
mod common;

use common::fake::FakeGithub;
use common::{bosun, lease_note, one_object};

const LATER: &str = "2026-10-22T12:00:00Z";
const EPIC: &str = "bosun:state/ready";

fn join(session: &str, depth: u32) -> String {
    format!(
        r#"{{"bosun":"join","session":"{session}","origin":"spawned","depth":{depth},"expires":"{LATER}"}}"#
    )
}

#[test]
fn a_forged_low_depth_join_does_not_lower_the_childs_depth() {
    let fake = FakeGithub::start();
    fake.issue(1, "bosun registry", &["bosun:registry"], "");
    fake.comment(1, &join("session_test", 2));
    fake.permission("mallory", "read");
    fake.comment_by(1, "mallory", &join("session_test", 0));
    fake.issue(
        10,
        "Lifecycle epic",
        &[EPIC],
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
    let child = fake
        .comments_of(1)
        .into_iter()
        .map(|c| c.body)
        .find(|b| b.contains("\"session\":\"session_child\""))
        .expect("child join");
    assert!(child.contains("\"depth\":3"), "{child}");
}
