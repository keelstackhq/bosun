mod common;

use common::fake::FakeGithub;
use common::{bosun, lease_note, one_object, run};
use serde_json::Value;

const NOW: &str = "2026-10-23T10:00:00Z";
const CHILD: &str = "session_child";
const LATER: &str = "2026-10-23T12:00:00Z";

fn bodies(fake: &FakeGithub, issue: u64) -> Vec<String> {
    fake.comments_of(issue)
        .into_iter()
        .map(|c| c.body)
        .collect()
}

fn registry_number(fake: &FakeGithub) -> u64 {
    let s = fake.state.lock().unwrap();
    s.issues
        .iter()
        .find(|i| i.labels.contains(&"bosun:registry".to_string()))
        .map(|i| i.number)
        .expect("registry issue")
}

#[test]
fn a_result_ends_the_lease_leaves_the_registry_and_wakes_the_parent() {
    let fake = FakeGithub::start();
    fake.issue(9, "Epic", &["bosun:state/claimed"], "");
    fake.comment(9, &lease_note(CHILD, LATER, "src/**"));
    let (code, v) = run(&fake, CHILD, NOW, &["join", "--parent", "session_root"]);
    assert_eq!(code, 0, "{v}");

    let (code, v) = run(
        &fake,
        CHILD,
        NOW,
        &["result", "9", "--outcome", "merged", "--pr", "42"],
    );
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["command"], "result");
    let d = &v["data"];
    assert_eq!((d["epic"].as_u64(), d["pr"].as_u64()), (Some(9), Some(42)));
    assert_eq!(d["outcome"], "merged");
    assert_eq!(
        (&d["lease_ended"], &d["left"]),
        (&Value::Bool(true), &Value::Bool(true))
    );
    assert_eq!(v["next"], serde_json::json!([]));

    let epic = bodies(&fake, 9);
    assert!(
        epic.iter()
            .any(|b| b.contains("Result of session_child: merged, PR #42"))
    );
    assert!(epic.iter().any(|b| {
        b.contains(r#"{"bosun":"result","session":"session_child","outcome":"merged","pr":42}"#)
    }));
    assert!(
        epic.iter()
            .any(|b| b.contains(r#""bosun":"lease-end""#) && b.contains(r#""reason":"result""#))
    );
    let reg = bodies(&fake, registry_number(&fake));
    assert!(
        reg.iter()
            .any(|b| b.contains(r#""bosun":"leave""#) && b.contains(r#""reason":"done""#))
    );

    let out = bosun()
        .env("BOSUN_GITHUB_API", fake.url())
        .env("BOSUN_WAIT_BASE_MS", "20")
        .args(["wait", "--child", CHILD, "--timeout", "0"])
        .output()
        .unwrap();
    let w = one_object(&out.stdout);
    let result = &w["data"]["after"]["result"];
    assert_eq!(result["outcome"], "merged", "{w}");
    assert_eq!(result["pr"], 42);
}

#[test]
fn a_session_without_lease_or_join_still_posts_its_result() {
    let fake = FakeGithub::start();
    fake.issue(9, "Epic", &["bosun:state/claimed"], "");
    let (code, v) = run(&fake, CHILD, NOW, &["result", "9", "--outcome", "blocked"]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["lease_ended"], false);
    assert_eq!(v["data"]["left"], false);
    assert_eq!(v["data"]["pr"], Value::Null);
    let epic = bodies(&fake, 9);
    assert_eq!(epic.len(), 1);
    assert!(epic[0].contains("Result of session_child: blocked"));
    assert!(!epic[0].contains("PR #"));
}

#[test]
fn a_bad_outcome_is_a_usage_error() {
    let fake = FakeGithub::start();
    fake.issue(9, "Epic", &[], "");
    let (code, _) = run(&fake, CHILD, NOW, &["result", "9", "--outcome", "done"]);
    assert_eq!(code, 2);
    assert!(fake.comments_of(9).is_empty());
}

#[test]
fn a_missing_epic_is_exit_1() {
    let fake = FakeGithub::start();
    let (code, v) = run(&fake, CHILD, NOW, &["result", "9", "--outcome", "merged"]);
    assert_eq!(code, 1, "{v}");
}

#[test]
fn a_halted_child_can_still_report() {
    let fake = FakeGithub::start();
    fake.issue(9, "Epic", &[], "");
    fake.content(
        "governance/owners.toml",
        "main",
        "[[owner]]\ngithub = \"ada\"\n",
    );
    fake.issue(5, "halt", &["bosun:halt"], "");
    fake.issue_by(5, "ada");
    let (code, v) = run(&fake, CHILD, NOW, &["claim", "9"]);
    assert_eq!(code, 4, "{v}");
    let (code, v) = run(
        &fake,
        CHILD,
        NOW,
        &["result", "9", "--outcome", "needs-human"],
    );
    assert_eq!(code, 0, "{v}");
}

/// A result is a report, not a transition (spec §5): the epic's state comes from GitHub and the
/// commands that act (`pr`/`reap` on a merge, `fail` and the stop rules), so it stays as it was.
#[test]
fn a_result_leaves_the_epic_state_alone() {
    for outcome in ["merged", "blocked", "needs-human"] {
        let fake = FakeGithub::start();
        fake.issue(9, "Epic", &["bosun:state/review"], "");
        fake.comment(9, &lease_note(CHILD, LATER, "src/**"));
        let (code, v) = run(&fake, CHILD, NOW, &["result", "9", "--outcome", outcome]);
        assert_eq!(code, 0, "{v}");
        let s = fake.state.lock().unwrap();
        let epic = s.issues.iter().find(|i| i.number == 9).unwrap();
        assert_eq!(epic.labels, ["bosun:state/review"], "{outcome}");
    }
}
