mod common;

use common::fake::FakeGithub;
use common::{lease_note, run};
use serde_json::json;

const T0: &str = "2026-10-22T10:00:00Z";

fn task(touch: &str, kind: &str) -> String {
    format!(
        r#"Do the thing.

<!-- bosun:task {{"item":"M1","task":"T1","touch":[{touch}],"kind":"{kind}"}} -->"#
    )
}

fn ready(fake: &FakeGithub, n: u64, touch: &str) {
    fake.issue(
        n,
        "task",
        &["bosun:state/ready"],
        &task(&format!("\"{touch}\""), "feature"),
    );
}

#[test]
fn claim_writes_a_lease_and_moves_to_claimed() {
    let fake = FakeGithub::start();
    ready(&fake, 10, "src/core/**");
    let (code, v) = run(&fake, "session_a", T0, &["claim", "10"]);
    assert_eq!(code, 0, "{v}");
    let comments = fake.comments_of(10);
    assert_eq!(comments.len(), 1);
    assert!(
        comments[0]
            .body
            .contains(r#""bosun":"lease","session":"session_a""#)
    );
    assert_eq!(v["data"]["issue"], 10);
    assert_eq!(v["data"]["comment_id"], comments[0].id);
    assert_eq!(v["data"]["lease"]["session"], "session_a");
    assert_eq!(v["data"]["lease"]["expires"], "2026-10-22T10:30:00Z");
    assert_eq!(v["data"]["lease"]["touch"], json!(["src/core/**"]));
    let labels = fake.labels_of(10);
    assert!(labels.contains(&"bosun:state/claimed".into()));
    assert!(!labels.contains(&"bosun:state/ready".into()));
}

#[test]
fn overlapping_touch_with_a_live_lease_is_exit_3() {
    let fake = FakeGithub::start();
    ready(&fake, 10, "src/core/**");
    fake.issue(11, "other", &["bosun:state/building"], "");
    fake.comment(
        11,
        &lease_note("session_z", "2026-10-22T10:20:00Z", "src/core/*.rs"),
    );
    let (code, v) = run(
        &fake,
        "session_a",
        T0,
        &["claim", "10", "--touch", "src/core/lease.rs"],
    );
    assert_eq!(code, 3, "{v}");
    assert_eq!(v["error"]["kind"], "conflict");
    let msg = v["error"]["message"].as_str().unwrap();
    assert!(msg.contains("#11") && msg.contains("session_z") && msg.contains("src/core/*.rs"));
    assert_eq!(v["data"]["conflicts"].as_array().unwrap().len(), 1);
    assert!(fake.comments_of(10).is_empty());
    assert!(fake.labels_of(10).contains(&"bosun:state/ready".into()));
}

#[test]
fn disjoint_touch_and_expired_leases_do_not_conflict() {
    let fake = FakeGithub::start();
    ready(&fake, 10, "src/core/**");
    fake.issue(11, "other", &["bosun:state/claimed"], "");
    fake.comment(
        11,
        &lease_note("session_z", "2026-10-22T10:20:00Z", "docs/**"),
    );
    fake.issue(12, "stale", &["bosun:state/claimed"], "");
    fake.comment(
        12,
        &lease_note("session_y", "2026-10-22T09:59:00Z", "src/core/**"),
    );
    let (code, v) = run(&fake, "session_a", T0, &["claim", "10"]);
    assert_eq!(code, 0, "{v}");
}

#[test]
fn reclaim_by_the_holder_is_idempotent() {
    let fake = FakeGithub::start();
    ready(&fake, 10, "src/**");
    let (_, first) = run(&fake, "session_a", T0, &["claim", "10"]);
    let (code, second) = run(&fake, "session_a", "2026-10-22T10:05:00Z", &["claim", "10"]);
    assert_eq!(code, 0, "{second}");
    assert_eq!(fake.comments_of(10).len(), 1);
    assert_eq!(first["data"]["comment_id"], second["data"]["comment_id"]);
    assert_eq!(first["data"]["lease"], second["data"]["lease"]);
    let (code, v) = run(&fake, "session_b", "2026-10-22T10:05:00Z", &["claim", "10"]);
    assert_eq!(code, 3, "{v}");
    assert!(
        v["error"]["message"]
            .as_str()
            .unwrap()
            .contains("#10 is claimed")
    );
}

#[test]
fn area_owned_by_another_live_session_is_exit_3() {
    let fake = FakeGithub::start();
    fake.issue(
        10,
        "task",
        &["bosun:state/ready", "bosun:area/lifecycle"],
        &task("\"src/a.rs\"", "feature"),
    );
    run(&fake, "session_z", T0, &["join", "--area", "lifecycle"]);
    let (code, v) = run(&fake, "session_a", T0, &["claim", "10"]);
    assert_eq!(code, 3, "{v}");
    assert!(
        v["error"]["message"]
            .as_str()
            .unwrap()
            .contains("session_z")
    );
    let (code, v) = run(&fake, "session_z", T0, &["claim", "10"]);
    assert_eq!(code, 0, "the owner may claim: {v}");
}

#[test]
fn an_expired_area_owner_does_not_block() {
    let fake = FakeGithub::start();
    fake.issue(
        10,
        "task",
        &["bosun:state/ready", "bosun:area/lifecycle"],
        &task("\"src/a.rs\"", "feature"),
    );
    run(
        &fake,
        "session_z",
        "2026-10-22T08:00:00Z",
        &["join", "--area", "lifecycle"],
    );
    assert_eq!(run(&fake, "session_a", T0, &["claim", "10"]).0, 0);
}

#[test]
fn halt_and_main_red_stop_claims_except_mainfix() {
    let fake = FakeGithub::start();
    ready(&fake, 10, "src/a.rs");
    fake.variable("BOSUN_HALT", "1");
    assert_eq!(run(&fake, "session_a", T0, &["claim", "10"]).0, 4);

    let fake = FakeGithub::start();
    ready(&fake, 10, "src/a.rs");
    fake.issue(
        11,
        "fix main",
        &["bosun:state/ready"],
        &task("\"src/b.rs\"", "mainfix"),
    );
    fake.issue(99, "main is red", &["bosun:main-red"], "");
    let (code, v) = run(&fake, "session_a", T0, &["claim", "10"]);
    assert_eq!(code, 4, "{v}");
    assert!(v["error"]["message"].as_str().unwrap().contains("#99"));
    assert_eq!(run(&fake, "session_a", T0, &["claim", "11"]).0, 0);
}

#[test]
fn claim_refuses_bad_input() {
    let fake = FakeGithub::start();
    let (code, _) = run(&fake, "session_a", T0, &["claim", "404", "--touch", "a"]);
    assert_eq!(code, 1);
    fake.issue(10, "task", &["bosun:state/ready"], "no metadata");
    let (code, v) = run(&fake, "session_a", T0, &["claim", "10"]);
    assert_eq!(code, 2, "{v}");
    let (code, _) = run(&fake, "session_a", T0, &["claim", "10", "--touch", "../x"]);
    assert_eq!(code, 2);
    fake.issue(11, "busy", &["bosun:state/building"], "");
    let (code, v) = run(&fake, "session_a", T0, &["claim", "11", "--touch", "a"]);
    assert_eq!(code, 3);
    assert!(
        v["error"]["message"]
            .as_str()
            .unwrap()
            .contains("#11 is building")
    );
}
