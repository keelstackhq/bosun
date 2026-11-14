mod common;

use common::fake::FakeGithub;
use common::run;

const NOW: &str = "2026-10-22T10:00:00Z";

#[test]
fn progress_is_a_summary_unless_full() {
    let fake = FakeGithub::start();
    let (code, v) = run(&fake, "session_a", NOW, &["progress"]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["summary"], true);
    assert!(v["data"].get("tasks").is_none());
    let (_, v) = run(&fake, "session_a", NOW, &["progress", "--full"]);
    assert!(v["data"]["tasks"].is_array());
    assert!(v["data"].get("summary").is_none());
}

#[test]
fn doctor_lists_only_what_needs_action_unless_full() {
    let fake = FakeGithub::start();
    let (_, short) = run(&fake, "session_a", NOW, &["doctor"]);
    let (_, full) = run(&fake, "session_a", NOW, &["doctor", "--full"]);
    assert_eq!(short["data"]["summary"], true);
    let shown = short["data"]["items"].as_array().unwrap();
    assert!(shown.iter().all(|i| i["status"] != "ok"), "{short}");
    let all = full["data"]["items"].as_array().unwrap().len();
    assert_eq!(
        shown.len() as u64 + short["data"]["items_ok"].as_u64().unwrap(),
        all as u64
    );
}
