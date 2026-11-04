mod common;

use common::fake::FakeGithub;
use common::{lease_note, run};
use serde_json::json;

const LAPSED: &str = "2026-10-22T10:30:00Z";
const LATER: &str = "2026-10-22T11:00:00Z";

fn lapsed_queued(fake: &FakeGithub, holder: &str) -> u64 {
    fake.issue(10, "task", &["bosun:state/queued"], "Do it.");
    fake.comment(10, &lease_note(holder, LAPSED, "src/a/**"))
}

#[test]
fn a_session_reclaims_its_own_queued_task_after_the_lease_lapsed() {
    let fake = FakeGithub::start();
    let id = lapsed_queued(&fake, "session_a");
    let (code, v) = run(&fake, "session_a", LATER, &["claim", "10"]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["lease"]["expires"], "2026-10-22T11:30:00Z");
    assert_eq!(v["data"]["lease"]["touch"], json!(["src/a/**"]));
    assert_eq!(v["data"]["comment_id"], id);
    assert!(fake.labels_of(10).contains(&"bosun:state/queued".into()));
    // The same record was renewed in place, and it is live now.
    let body = fake
        .comments_of(10)
        .into_iter()
        .find(|c| c.id == id)
        .unwrap()
        .body;
    assert!(body.contains("2026-10-22T11:30:00Z"), "{body}");
}

#[test]
fn another_session_cannot_take_over_a_lapsed_queued_task() {
    let fake = FakeGithub::start();
    lapsed_queued(&fake, "session_a");
    let (code, v) = run(&fake, "session_b", LATER, &["claim", "10"]);
    assert_eq!(code, 3, "{v}");
}

#[test]
fn a_lapsed_own_lease_is_not_renewed_over_another_live_lease() {
    let fake = FakeGithub::start();
    lapsed_queued(&fake, "session_a");
    fake.issue(11, "other", &["bosun:state/building"], "");
    fake.comment(
        11,
        &lease_note("session_z", "2026-10-22T11:20:00Z", "src/a/x.rs"),
    );
    let (code, v) = run(&fake, "session_a", LATER, &["claim", "10"]);
    assert_eq!(code, 3, "{v}");
    assert_eq!(v["data"]["conflicts"][0]["issue"], 11);
}
