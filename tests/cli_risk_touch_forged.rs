//! The touch warning reads the lease from a repo writer's record: a read user's newer lease
//! with a broad touch does not silence it.
mod common;

use common::fake::FakeGithub;
use common::lease_note;
use serde_json::json;

const NOW: &str = "2026-10-22T10:00:00Z";

#[test]
fn a_readers_broad_lease_does_not_silence_the_warning() {
    let fake = FakeGithub::start();
    fake.pull(1, "change", "feature", "Closes #7");
    fake.issue(7, "the task", &[], "Do it.\n<!-- bosun:task {} -->");
    fake.comment(
        7,
        &lease_note("session_a", "2026-10-22T10:30:00Z", "src/cli/status.rs"),
    );
    fake.permission("mallory", "read");
    fake.comment_by(
        7,
        "mallory",
        &lease_note("session_a", "2026-10-22T10:30:00Z", "**"),
    );
    fake.pull_file(1, "src/cli/pr.rs", "modified", 1, 0, Some("@@ -1 +1 @@"));
    let (code, v) = common::run(&fake, "session_test", NOW, &["risk-check", "1"]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["out_of_touch"], json!(["src/cli/pr.rs"]), "{v}");
}
