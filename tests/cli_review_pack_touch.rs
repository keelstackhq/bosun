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
