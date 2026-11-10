//! Join and lease records count only from writers: a reader's or outsider's forged record
//! neither blocks a claim nor makes a session look live.
mod common;

use common::fake::FakeGithub;
use common::{lease_note, run};
use serde_json::json;

const T0: &str = "2026-10-22T10:00:00Z";

fn join_note(session: &str) -> String {
    format!(
        "{{\"bosun\":\"join\",\"session\":\"{session}\",\"origin\":\"spawned\",\"area\":\"gates\",\"depth\":1,\"expires\":\"2026-10-22T10:20:00Z\"}}"
    )
}

#[test]
fn a_forged_lease_does_not_block_a_claim() {
    let fake = FakeGithub::start();
    fake.permission("mallory", "read");
    fake.issue(10, "t", &["bosun:state/ready"], "Do it.");
    fake.comment_by(
        10,
        "mallory",
        &lease_note("session_x", "2026-10-22T10:20:00Z", "src/a/**"),
    );
    let (code, v) = run(
        &fake,
        "session_a",
        T0,
        &["claim", "10", "--touch", "src/a/**"],
    );
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["lease"]["touch"], json!(["src/a/**"]));
}

#[test]
fn a_forged_lease_is_not_listed_in_status() {
    let fake = FakeGithub::start();
    fake.permission("mallory", "none");
    fake.issue(10, "t", &["bosun:state/building"], "");
    fake.comment_by(
        10,
        "mallory",
        &lease_note("session_x", "2026-10-22T10:20:00Z", "src/a/**"),
    );
    let (code, v) = run(&fake, "session_a", T0, &["status"]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["leases"], json!([]));
}

#[test]
fn a_forged_join_is_not_a_live_session() {
    let fake = FakeGithub::start();
    fake.permission("mallory", "read");
    fake.issue(1, "bosun registry", &["bosun:registry"], "");
    fake.comment_by(1, "mallory", &join_note("session_x"));
    fake.comment(1, &join_note("session_b"));
    let (code, v) = run(&fake, "session_a", T0, &["status"]);
    assert_eq!(code, 0, "{v}");
    let live = v["data"]["live_sessions"].as_array().unwrap();
    assert_eq!(live.len(), 1, "{v}");
    assert_eq!(live[0]["session"], "session_b");
}
