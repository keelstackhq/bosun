mod common;

use common::fake::FakeGithub;
use common::run;
use serde_json::json;

const T0: &str = "2026-10-22T10:00:00Z";

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

fn result(session: &str) -> String {
    format!("{{\"bosun\":\"result\",\"session\":\"{session}\",\"outcome\":\"merged\"}}")
}

fn seed(fake: &FakeGithub) {
    fake.issue(1, "bosun registry", &["bosun:registry"], "");
    fake.comment(1, &join("done_child", "spawned", "session_a"));
    fake.comment(1, &join("busy_child", "spawned", "session_a"));
    fake.comment(1, &join("other_child", "spawned", "session_z"));
    // A relay root has no parent: never archived here.
    fake.comment(1, &join("old_root", "local", ""));
    fake.issue(9, "Epic", &[], "");
    for s in ["done_child", "other_child", "old_root"] {
        fake.comment(9, &result(s));
    }
}

#[test]
fn status_and_reap_archive_children_with_a_result() {
    for cmd in ["status", "reap"] {
        let fake = FakeGithub::start();
        seed(&fake);
        let (code, v) = run(&fake, "session_a", T0, &[cmd]);
        assert_eq!(code, 0, "{v}");
        assert_eq!(
            v["next"],
            json!([{"tool": "archive_session", "args": {"session_id": "done_child"}}]),
            "{cmd}"
        );
    }
}

#[test]
fn no_archive_without_a_result() {
    for cmd in ["status", "reap"] {
        let fake = FakeGithub::start();
        fake.issue(1, "bosun registry", &["bosun:registry"], "");
        fake.comment(1, &join("busy_child", "spawned", "session_a"));
        let (_, v) = run(&fake, "session_a", T0, &[cmd]);
        assert_eq!(v["next"], json!([]), "{cmd}");
    }
}
