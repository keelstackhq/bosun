mod common;

use common::fake::FakeGithub;
use common::run;
use serde_json::{Value, json};

const T0: &str = "2026-10-22T10:00:00Z";
const BUILDING: &str = "bosun:state/building";

fn join(session: &str, origin: &str, parent: &str) -> String {
    let p = if parent.is_empty() {
        String::new()
    } else {
        format!(",\"parent\":\"{parent}\"")
    };
    format!(
        "{{\"bosun\":\"join\",\"session\":\"{session}\",\"origin\":\"{origin}\"{p},\"depth\":2,\"expires\":\"2026-10-22T10:20:00Z\"}}"
    )
}

fn result(session: &str, outcome: &str) -> String {
    format!("{{\"bosun\":\"result\",\"session\":\"{session}\",\"outcome\":\"{outcome}\"}}")
}

fn lease(session: &str, expires: &str) -> String {
    format!(
        "{{\"bosun\":\"lease\",\"session\":\"{session}\",\"origin\":\"spawned\",\"expires\":\"{expires}\",\"touch\":[\"src/**\"]}}"
    )
}

fn registry(fake: &FakeGithub) {
    fake.issue(1, "bosun registry", &["bosun:registry"], "");
    fake.comment(1, &join("child", "spawned", "session_a"));
}

fn archived(fake: &FakeGithub, cmd: &str) -> Value {
    let (code, v) = run(fake, "session_a", T0, &[cmd]);
    assert_eq!(code, 0, "{v}");
    v["next"].clone()
}

fn archive_child() -> Value {
    json!([{"tool": "archive_session", "args": {"session_id": "child"}}])
}

#[test]
fn final_result_archives_only_the_callers_children() {
    for cmd in ["status", "reap"] {
        let fake = FakeGithub::start();
        registry(&fake);
        fake.comment(1, &join("busy_child", "spawned", "session_a"));
        fake.comment(1, &join("other_child", "spawned", "session_z"));
        // A relay root has no parent: never archived here.
        fake.comment(1, &join("old_root", "local", ""));
        fake.issue(9, "Epic", &[], "");
        for s in ["child", "other_child", "old_root", "session_a"] {
            fake.comment(9, &result(s, "needs-human"));
        }
        assert_eq!(archived(&fake, cmd), archive_child(), "{cmd}");
    }
}

#[test]
fn no_result_or_partial_result_archives_nothing() {
    for cmd in ["status", "reap"] {
        let fake = FakeGithub::start();
        registry(&fake);
        assert_eq!(archived(&fake, cmd), json!([]), "{cmd} no result");
        fake.issue(9, "Epic", &[], "");
        fake.comment(9, &result("child", "partial"));
        assert_eq!(archived(&fake, cmd), json!([]), "{cmd} partial");
    }
}

#[test]
fn newest_result_decides() {
    let fake = FakeGithub::start();
    registry(&fake);
    fake.issue(9, "Epic", &[], "");
    fake.comment(9, &result("child", "merged"));
    fake.comment(9, &result("child", "partial"));
    assert_eq!(archived(&fake, "status"), json!([]));
}

#[test]
fn a_later_join_means_it_works_again() {
    let fake = FakeGithub::start();
    registry(&fake);
    fake.issue(9, "Epic", &[], "");
    fake.comment(9, &result("child", "merged"));
    fake.comment(1, &join("child", "spawned", "session_a"));
    assert_eq!(archived(&fake, "status"), json!([]));
}

#[test]
fn a_live_lease_after_the_result_means_it_works_again() {
    let fake = FakeGithub::start();
    registry(&fake);
    fake.issue(9, "Epic", &[], "");
    fake.comment(9, &result("child", "merged"));
    fake.issue(5, "Task", &[BUILDING], "");
    fake.comment(5, &lease("child", "2026-10-22T11:00:00Z"));
    assert_eq!(archived(&fake, "status"), json!([]));
    // The same lease, expired, no longer counts.
    let fake = FakeGithub::start();
    registry(&fake);
    fake.issue(9, "Epic", &[], "");
    fake.comment(9, &result("child", "merged"));
    fake.issue(5, "Task", &[BUILDING], "");
    fake.comment(5, &lease("child", "2026-10-22T09:00:00Z"));
    assert_eq!(archived(&fake, "status"), archive_child());
}

#[test]
fn a_lease_older_than_the_result_does_not_block() {
    let fake = FakeGithub::start();
    registry(&fake);
    fake.issue(5, "Task", &[BUILDING], "");
    fake.comment(5, &lease("child", "2026-10-22T11:00:00Z"));
    fake.issue(9, "Epic", &[], "");
    fake.comment(9, &result("child", "blocked"));
    assert_eq!(archived(&fake, "status"), archive_child());
}
