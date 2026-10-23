//! Use cases of M5 (spec §12): the learning loop.

mod common;

use common::fake::FakeGithub;
use common::run;

/// UC-M5-2: a blocking lesson sets process-red, and waves pause.
#[test]
fn uc_m5_2() {
    let now = "2026-10-23T10:00:00Z";
    let fake = FakeGithub::start();
    fake.issue(10, "Epic", &["bosun:state/claimed"], "");
    fake.issue(
        11,
        "task",
        &["bosun:state/ready"],
        r#"<!-- bosun:task {"item":"M5","task":"T1","epic":10,"touch":["src/a.rs"]} -->"#,
    );
    fake.issue(
        20,
        "lesson",
        &["bosun:lesson", "bosun:lesson/blocking", "bosun:process-red"],
        "",
    );
    let (code, v) = run(&fake, "session_a", now, &["wave", "plan", "10"]);
    assert_eq!(code, 4, "{v}");
    let (code, v) = run(&fake, "session_a", now, &["status"]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["process_red"], 20);
}
