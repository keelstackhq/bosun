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

#[test]
fn a_secret_in_the_text_is_refused_without_echo() {
    let fake = FakeGithub::start();
    let secret = format!("ghp_{}", "a1B2".repeat(9));
    let text = format!("use {secret} to push");
    let (code, v) = run(&fake, "session_test", NOW, &["msg", "s", &text]);
    assert_eq!(code, 2, "{v}");
    assert!(v["error"]["fix"].as_str().unwrap().contains("redact"));
    let message = v["error"]["message"].as_str().unwrap();
    assert!(message.contains("GitHub token"), "{message}");
    assert!(!v.to_string().contains(&secret));
    assert!(v["next"].as_array().unwrap().is_empty());
}

#[test]
fn a_high_entropy_word_is_refused_and_ids_and_hashes_pass() {
    let fake = FakeGithub::start();
    let secret = format!("{}/{}", "wJalrXUtnFEMI/K7MDENG", "bPxRfiCYEXAMPLEKEY");
    let (code, v) = run(&fake, "session_test", NOW, &["msg", "s", &secret]);
    assert_eq!(code, 2, "{v}");
    assert!(
        v["error"]["message"]
            .as_str()
            .unwrap()
            .contains("high-entropy")
    );
    assert!(!v.to_string().contains(&secret));
    let integrity = format!(
        "sha512-{}==",
        &"q8Zr3KxP0vLm5NwT2bYc7HdJ4sFg9AeUZ1Xo6RiEt/+".repeat(3)[..86]
    );
    for ok in [
        format!("session_{}", "01SsFUkfeX2mg29KV4Lhoi5P"),
        format!("see {integrity}"),
    ] {
        let (code, v) = run(&fake, "session_test", NOW, &["msg", "s", &ok]);
        assert_eq!(code, 0, "{v}");
    }
}
