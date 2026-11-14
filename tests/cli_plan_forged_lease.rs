//! `plan amend` closes a dropped task unless a live lease of a repo writer holds it: a read
//! user's forged lease does not block it.
mod common;

use common::fake::FakeGithub;

const T0: &str = "2026-10-23T10:00:00Z";
const PLAN: &str = "# Plan: widget\n\nproject: widget\n\n## M1: Lifecycle\n\narea: lifecycle\n\n### Tasks\n\n- T1: State machine\n  touch: src/core/state.rs\n- T2: Stop rules\n  touch: src/core/stop.rs\n\n### Done when\n\n- UC-M1-1: a task moves planned -> merged.\n";

#[test]
fn a_readers_forged_lease_does_not_keep_a_dropped_task_open() {
    let (tmp, fake) = (tempfile::tempdir().unwrap(), FakeGithub::start());
    let file = tmp.path().join("plan.md");
    std::fs::write(&file, PLAN).unwrap();
    let run = |args: &[&str]| {
        let mut a = vec!["-C", tmp.path().to_str().unwrap(), "plan"];
        a.extend_from_slice(args);
        common::run(&fake, "session_test", T0, &a)
    };
    let f = file.to_str().unwrap();
    let (code, v) = run(&["import", f, "--full"]);
    assert_eq!(code, 0, "{v}");
    let t = {
        let s = fake.state.lock().unwrap();
        s.issues
            .iter()
            .find(|i| i.body.starts_with("<!-- bosun:plan M1/T2 -->"))
            .unwrap()
            .number
    };
    fake.permission("mallory", "read");
    fake.comment_by(
        t,
        "mallory",
        &common::lease_note("session_other", "2026-10-23T11:00:00Z", "src/core/stop.rs"),
    );
    std::fs::write(
        &file,
        PLAN.replace("- T2: Stop rules\n  touch: src/core/stop.rs\n", ""),
    )
    .unwrap();
    let (code, v) = run(&["amend", f, "--full"]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["skipped"], serde_json::json!([]), "{v}");
    assert_eq!(fake.issue_state(t).state, "closed");
}
