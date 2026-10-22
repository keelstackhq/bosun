mod common;

use common::fake::FakeGithub;
use common::run;

const NOW: &str = "2026-10-22T10:00:00Z";

#[test]
fn msg_returns_one_send_message_with_the_issue_prefix() {
    let fake = FakeGithub::start();
    let (code, v) = run(
        &fake,
        "session_test",
        NOW,
        &["msg", "session_child", "rebase on main", "--issue", "12"],
    );
    assert_eq!(code, 0, "{v}");
    let next = v["next"].as_array().unwrap();
    assert_eq!(next.len(), 1);
    assert_eq!(next[0]["tool"], "send_message");
    assert_eq!(next[0]["args"]["session_id"], "session_child");
    assert_eq!(next[0]["args"]["message"], "From #12: rebase on main");
    assert!(fake.requests().is_empty(), "msg makes no GitHub calls");
}

#[test]
fn without_an_issue_the_prefix_is_this_session() {
    let fake = FakeGithub::start();
    let (_, v) = run(&fake, "session_test", NOW, &["msg", "session_child", "hi"]);
    assert_eq!(v["next"][0]["args"]["message"], "From session_test: hi");
}

#[test]
fn the_limit_is_500_characters() {
    let fake = FakeGithub::start();
    let at = "é".repeat(500);
    let (code, _) = run(&fake, "session_test", NOW, &["msg", "s", &at]);
    assert_eq!(code, 0, "500 characters pass, bytes do not count");
    let over = "x".repeat(501);
    let (code, v) = run(&fake, "session_test", NOW, &["msg", "s", &over]);
    assert_eq!(code, 2, "{v}");
    assert!(v["next"].as_array().unwrap().is_empty());
    let (code, _) = run(&fake, "session_test", NOW, &["msg", "s", "  "]);
    assert_eq!(code, 2);
}
