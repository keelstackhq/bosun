//! `bosun result` lists the epic's merged tasks whatever the outcome, and `partial` hands an
//! epic back with part of its tasks merged (F13).

mod common;

use common::fake::FakeGithub;
use common::{lease_note, run};
use serde_json::json;

const NOW: &str = "2026-10-23T10:00:00Z";
const CHILD: &str = "session_child";
const LATER: &str = "2026-10-23T12:00:00Z";

fn task(fake: &FakeGithub, n: u64, epic: u64, state: &str) {
    fake.issue(
        n,
        "task",
        &["bosun:source/plan", &format!("bosun:state/{state}")],
        &format!(
            r#"<!-- bosun:task {{"item":"M1","task":"T{n}","epic":{epic},"touch":["src/a.rs"]}} -->"#
        ),
    );
}

/// Epic #9 with seven tasks: three merged (one closed), one queued, the rest open; plus a merged
/// task of another epic.
fn setup() -> FakeGithub {
    let fake = FakeGithub::start();
    fake.issue(9, "Epic", &["bosun:state/claimed"], "");
    fake.comment(9, &lease_note(CHILD, LATER, "src/**"));
    task(&fake, 11, 9, "merged");
    task(&fake, 12, 9, "merged");
    task(&fake, 13, 9, "merged");
    fake.close_issue(13, NOW);
    task(&fake, 14, 9, "queued");
    task(&fake, 15, 9, "ready");
    task(&fake, 16, 9, "building");
    task(&fake, 17, 9, "blocked");
    task(&fake, 20, 8, "merged");
    fake
}

#[test]
fn needs_human_still_lists_the_merged_tasks() {
    let fake = setup();
    let (code, v) = run(
        &fake,
        CHILD,
        NOW,
        &["result", "9", "--outcome", "needs-human"],
    );
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["tasks_merged"], json!([11, 12, 13]));
    assert_eq!(v["data"]["task_state"], "needs-human");
}

#[test]
fn partial_hands_the_epic_back_to_ready() {
    let fake = setup();
    let (code, v) = run(&fake, CHILD, NOW, &["result", "9", "--outcome", "partial"]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["outcome"], "partial");
    assert_eq!(v["data"]["tasks_merged"], json!([11, 12, 13]));
    assert_eq!(v["data"]["task_state"], "ready");
    assert_eq!(v["data"]["lease_ended"], true);
    assert!(fake.labels_of(9).contains(&"bosun:state/ready".to_string()));
    assert!(
        fake.comments_of(9)
            .iter()
            .any(|c| c.body.contains(r#""outcome":"partial""#))
    );
}

#[test]
fn partial_never_moves_a_merged_epic() {
    let fake = FakeGithub::start();
    fake.issue(9, "Epic", &["bosun:state/merged"], "");
    let (code, v) = run(&fake, CHILD, NOW, &["result", "9", "--outcome", "partial"]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["task_state"], "merged");
    assert_eq!(v["data"]["tasks_merged"], json!([]));
    assert_eq!(fake.labels_of(9), ["bosun:state/merged".to_string()]);
}
