//! `bosun result` names the green PRs that wait for a merge and messages the parent (C8).

mod common;

use common::fake::FakeGithub;
use common::{lease_note, run};
use serde_json::json;

const NOW: &str = "2026-10-23T10:00:00Z";
const LATER: &str = "2026-10-23T12:00:00Z";
const CHILD: &str = "session_child";

#[test]
fn a_clean_pr_of_the_epic_is_named_to_the_parent() {
    let fake = FakeGithub::start();
    fake.issue(9, "Epic", &["bosun:state/claimed"], "");
    fake.comment(9, &lease_note(CHILD, LATER, "src/**"));
    fake.pull(42, "feat", "claude/x", "Closes #9");
    fake.pull_set(42, "mergeable_state", json!("clean"));
    fake.pull(43, "other", "claude/y", "Closes #77");
    fake.pull_set(43, "mergeable_state", json!("clean"));
    let (code, v) = run(&fake, CHILD, NOW, &["join", "--parent", "session_root"]);
    assert_eq!(code, 0, "{v}");
    let (code, v) = run(
        &fake,
        CHILD,
        NOW,
        &["result", "9", "--outcome", "partial", "--pr", "42"],
    );
    assert_eq!(code, 0, "{v}");
    assert_eq!(
        v["data"]["ready_to_merge"],
        json!([{"pr": 42, "head_sha": "sha42"}])
    );
    let next = &v["next"][0];
    assert_eq!(next["tool"], "send_message", "{v}");
    assert_eq!(next["args"]["session_id"], "session_root");
    let m = next["args"]["message"].as_str().unwrap();
    assert!(m.contains("#42") && !m.contains("#43"), "{m}");
    assert!(m.len() <= 500);
}
