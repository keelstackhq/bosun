//! `bosun result` closes the epic it reports on when all its tasks are done (spec section 4).

mod common;

use common::fake::FakeGithub;
use common::run;
use serde_json::json;

const NOW: &str = "2026-11-05T10:00:00Z";
const CHILD: &str = "session_child";

fn epic(fake: &FakeGithub, state: &str) {
    let body = r#"E: epic <!-- bosun:epic {"item":"E1"} -->"#;
    fake.issue(
        9,
        "E1",
        &["bosun:source/plan", &format!("bosun:state/{state}")],
        body,
    );
}

fn task(fake: &FakeGithub, n: u64, state: &str) {
    let body = format!(r#"<!-- bosun:task {{"item":"E1","task":"T{n}","epic":9}} -->"#);
    let labels = ["bosun:source/plan", &format!("bosun:state/{state}")];
    fake.issue(n, "task", &labels, &body);
}

fn result(fake: &FakeGithub, outcome: &str) -> serde_json::Value {
    let (code, v) = run(fake, CHILD, NOW, &["result", "9", "--outcome", outcome]);
    assert_eq!(code, 0, "{v}");
    v
}

#[test]
fn result_closes_an_epic_whose_tasks_are_all_merged() {
    let fake = FakeGithub::start();
    epic(&fake, "claimed");
    fake.comment(
        9,
        &common::lease_note(CHILD, "2026-11-05T11:00:00Z", "x/**"),
    );
    task(&fake, 10, "merged");
    task(&fake, 11, "merged");
    let v = result(&fake, "merged");
    assert_eq!(v["data"]["epic_closed"], json!(true), "{v}");
    let closed = fake.issue_state(9);
    assert_eq!(closed.state, "closed");
    assert_eq!(closed.state_reason.as_deref(), Some("completed"));
    let l = fake.labels_of(9);
    assert!(l.contains(&"bosun:state/merged".into()) && !l.contains(&"bosun:state/claimed".into()));
}

#[test]
fn an_open_ready_task_keeps_the_epic_open() {
    let fake = FakeGithub::start();
    epic(&fake, "claimed");
    task(&fake, 10, "merged");
    task(&fake, 11, "ready");
    let v = result(&fake, "merged");
    assert_eq!(v["data"]["epic_closed"], json!(false), "{v}");
    assert_eq!(fake.issue_state(9).state, "open");
}

#[test]
fn another_sessions_live_lease_keeps_the_epic_open() {
    let fake = FakeGithub::start();
    epic(&fake, "claimed");
    fake.comment(
        9,
        &common::lease_note("orch", "2026-11-05T11:00:00Z", "x/**"),
    );
    task(&fake, 10, "merged");
    let v = result(&fake, "merged");
    assert_eq!(v["data"]["epic_closed"], json!(false), "{v}");
    assert_eq!(fake.issue_state(9).state, "open");
}

#[test]
fn tasks_updated_just_now_still_close_via_result() {
    let fake = FakeGithub::start();
    epic(&fake, "claimed");
    task(&fake, 10, "merged");
    fake.issue_updated(10, NOW);
    let v = result(&fake, "merged");
    assert_eq!(v["data"]["epic_closed"], json!(true), "{v}");
    assert_eq!(fake.issue_state(9).state, "closed");
}

#[test]
fn a_blocked_outcome_keeps_the_epic_open() {
    let fake = FakeGithub::start();
    epic(&fake, "claimed");
    task(&fake, 10, "merged");
    let v = result(&fake, "blocked");
    assert_eq!(v["data"]["epic_closed"], json!(false), "{v}");
    assert_eq!(fake.issue_state(9).state, "open");
}

#[test]
fn a_late_blocked_result_on_a_closed_epic_is_reported() {
    let fake = FakeGithub::start();
    epic(&fake, "merged");
    fake.close_issue_as(9, NOW, "completed");
    task(&fake, 10, "merged");
    let v = result(&fake, "blocked");
    assert_eq!(v["data"]["epic_already_closed"], json!(true), "{v}");
    let why = v["data"]["state_skipped"].as_str().unwrap_or_default();
    assert!(why.contains("merged"), "{v}");
    assert_eq!(v["data"]["task_state"], json!("merged"), "{v}");
}

#[test]
fn another_childs_expired_lease_without_a_result_keeps_the_epic_open() {
    let fake = FakeGithub::start();
    epic(&fake, "claimed");
    fake.comment(
        9,
        r#"{"bosun":"lease","session":"session_other","origin":"spawned","expires":"2026-11-05T09:00:00Z","touch":[]}"#,
    );
    task(&fake, 10, "merged");
    let v = result(&fake, "merged");
    assert_eq!(v["data"]["epic_closed"], json!(false), "{v}");
    assert_eq!(fake.issue_state(9).state, "open");
}
