//! The review pack shows the touch list of the task's lease, which a re-claim may have widened
//! beyond the task metadata (F16).

mod common;

use common::fake::FakeGithub;
use common::{lease_note, run};
use serde_json::json;

const T0: &str = "2026-10-22T10:00:00Z";

fn setup() -> FakeGithub {
    let fake = FakeGithub::start();
    fake.issue(
        7,
        "the task",
        &["bosun:state/review"],
        "Do it.\n<!-- bosun:task {\"item\":\"M1\",\"task\":\"T1\",\"touch\":[\"src/a.rs\"]} -->",
    );
    fake.pull_as_issue(42, "the pr");
    fake.pull(42, "the pr", "claude/x", "Closes #7");
    fake.pull_file(42, "src/b.rs", "added", 5, 0, Some("@@ b"));
    fake
}

#[test]
fn a_widened_lease_shows_in_the_pack() {
    let fake = setup();
    fake.comment(7, &lease_note("s/T1", "2026-10-22T10:20:00Z", "src/a.rs"));
    // `bosun claim 7 --touch src/b.rs` writes a newer lease with the union.
    fake.comment(
        7,
        r#"Claimed by s/T1.

{"bosun":"lease","session":"s/T1","origin":"local","expires":"2026-10-22T10:30:00Z","touch":["src/a.rs","src/b.rs"]}"#,
    );
    let (code, v) = run(&fake, "s", T0, &["review-pack", "42"]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["task"]["meta"]["touch"], json!(["src/a.rs"]));
    assert_eq!(
        v["data"]["task"]["lease_touch"],
        json!(["src/a.rs", "src/b.rs"])
    );
}

#[test]
fn without_a_lease_the_lease_touch_is_empty() {
    let fake = setup();
    let (code, v) = run(&fake, "s", T0, &["review-pack", "42"]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["task"]["lease_touch"], json!([]));
}

fn grow(fake: &FakeGithub, session: &str, touch: &str) {
    fake.comment(
        7,
        &format!(
            "Claimed by {session}.\n\n{{\"bosun\":\"lease\",\"session\":\"{session}\",\"origin\":\"local\",\"expires\":\"2026-10-22T10:30:00Z\",\"touch\":[{touch}]}}"
        ),
    );
}

#[test]
fn touch_added_lists_the_claimants_globs_beyond_the_task() {
    let fake = setup();
    fake.comment(7, &lease_note("s/T1", "2026-10-22T10:20:00Z", "src/a.rs"));
    grow(&fake, "s/T1", r#""src/a.rs","src/b.rs""#);
    fake.comment(7, r#"{"bosun":"task-pr","session":"s/T1","pr":42}"#);
    let (code, v) = run(&fake, "s", T0, &["review-pack", "42"]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["task"]["claimant"], "s/T1");
    assert_eq!(v["data"]["task"]["touch_added"], json!(["src/b.rs"]));
    assert!(
        v["data"]["instructions"]
            .as_str()
            .unwrap()
            .contains("touch_added")
    );
}

#[test]
fn only_the_claimants_lease_records_from_writers_count() {
    let fake = setup();
    fake.comment(7, &lease_note("s/T1", "2026-10-22T10:20:00Z", "src/a.rs"));
    fake.comment(7, r#"{"bosun":"task-pr","session":"s/T1","pr":42}"#);
    // Another session's lease record does not widen the claimant's touch.
    grow(&fake, "other/T9", r#""src/**""#);
    // Nor does a lease record of the claimant posted by a read-only user.
    fake.permission("outsider", "read");
    fake.comment_by(
        7,
        "outsider",
        r#"{"bosun":"lease","session":"s/T1","origin":"local","expires":"2026-10-22T10:30:00Z","touch":["secrets/**"]}"#,
    );
    let (code, v) = run(&fake, "s", T0, &["review-pack", "42"]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["task"]["lease_touch"], json!(["src/a.rs"]));
    assert_eq!(v["data"]["task"]["touch_added"], json!([]));
}
