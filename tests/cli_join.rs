mod common;

use common::fake::FakeGithub;
use common::{TOKEN_CANARY, bosun, one_object};
use serde_json::Value;

/// Run `bosun <args>` as `session` at `now` against the fake; returns (exit code, envelope).
fn run(fake: &FakeGithub, session: &str, now: &str, args: &[&str]) -> (i32, Value) {
    let out = bosun()
        .env("BOSUN_GITHUB_API", fake.url())
        .env("BOSUN_SESSION", session)
        .env("BOSUN_NOW", now)
        .args(args)
        .output()
        .unwrap();
    let all = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(!all.contains(TOKEN_CANARY), "token leaked");
    let v = one_object(&out.stdout);
    if v["ok"] == false {
        assert!(!v["error"]["fix"].as_str().unwrap().trim().is_empty());
    }
    (out.status.code().unwrap(), v)
}

const T0: &str = "2026-10-22T10:00:00Z";
const T1: &str = "2026-10-22T10:10:00Z";

fn registry(fake: &FakeGithub) -> u64 {
    let s = fake.state.lock().unwrap();
    let regs: Vec<_> = s
        .issues
        .iter()
        .filter(|i| i.labels.iter().any(|l| l == "bosun:registry"))
        .collect();
    assert_eq!(regs.len(), 1, "exactly one registry issue");
    assert_eq!(regs[0].title, "bosun registry");
    regs[0].number
}

#[test]
fn join_creates_the_registry_once_and_edits_in_place() {
    let fake = FakeGithub::start();
    let (code, v) = run(&fake, "session_a", T0, &["join", "--area", "lifecycle"]);
    assert_eq!(code, 0, "{v}");
    let reg = registry(&fake);
    assert_eq!(v["data"]["registry_issue"], reg);
    assert_eq!(v["data"]["session"], "session_a");
    assert_eq!(v["data"]["origin"], "local");
    assert_eq!(v["data"]["area"], "lifecycle");
    assert_eq!(v["data"]["depth"], 1);
    assert_eq!(v["data"]["expires"], "2026-10-22T10:30:00Z");

    let (code, v) = run(&fake, "session_a", T1, &["join"]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["expires"], "2026-10-22T10:40:00Z");
    assert_eq!(v["data"]["area"], "lifecycle", "kept on rejoin");
    registry(&fake);
    let comments = fake.comments_of(reg);
    assert_eq!(
        comments.len(),
        1,
        "a live member never gets a second comment"
    );
    assert!(comments[0].body.contains("2026-10-22T10:40:00Z"));
    assert!(fake.requests().iter().any(|r| {
        r.method == "PATCH"
            && r.path
                .ends_with(&format!("/issues/comments/{}", comments[0].id))
    }));

    let (_, v) = run(
        &fake,
        "session_b",
        T1,
        &[
            "join",
            "--origin",
            "spawned",
            "--parent",
            "session_a",
            "--depth",
            "2",
        ],
    );
    assert_eq!(v["data"]["origin"], "spawned");
    assert_eq!(fake.comments_of(reg).len(), 2);
    assert!(
        fake.comments_of(reg)[1]
            .body
            .contains(r#""parent":"session_a""#)
    );
}

#[test]
fn join_and_heartbeat_are_halted_by_the_kill_switch() {
    let fake = FakeGithub::start();
    fake.variable("BOSUN_HALT", "1");
    for cmd in ["join", "heartbeat"] {
        let (code, v) = run(&fake, "session_a", T0, &[cmd]);
        assert_eq!(code, 4, "{cmd}");
        assert_eq!(v["error"]["kind"], "halted");
    }
}

#[test]
fn bad_origin_is_an_error_with_a_fix() {
    let fake = FakeGithub::start();
    let (code, _) = run(&fake, "session_a", T0, &["join", "--origin", "mars"]);
    assert_eq!(code, 1);
}

fn lease(session: &str, expires: &str, touch: &str) -> String {
    format!(
        "Claimed.\n\n{{\"bosun\":\"lease\",\"session\":\"{session}\",\"origin\":\"local\",\"expires\":\"{expires}\",\"touch\":[\"{touch}\"]}}"
    )
}

#[test]
fn heartbeat_edits_leases_and_the_join_record_in_place() {
    let fake = FakeGithub::start();
    run(&fake, "session_a", T0, &["join"]);
    let reg = registry(&fake);
    fake.issue(5, "mine", &["bosun:state/claimed"], "");
    fake.issue(6, "expired", &["bosun:state/building"], "");
    fake.issue(7, "theirs", &["bosun:state/claimed"], "");
    let mine = fake.comment(5, &lease("session_a", "2026-10-22T10:20:00Z", "src/a.rs"));
    fake.comment(6, &lease("session_a", "2026-10-22T09:00:00Z", "src/b.rs"));
    let theirs = fake.comment(7, &lease("session_z", "2026-10-22T10:20:00Z", "src/c.rs"));
    let before = fake.state.lock().unwrap().comments.len();
    let posts = fake
        .requests()
        .iter()
        .filter(|r| r.method == "POST")
        .count();

    let (code, v) = run(&fake, "session_a", T1, &["heartbeat"]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["leases"], serde_json::json!([5]));
    assert_eq!(v["data"]["registry"], true);
    let s = fake.state.lock().unwrap();
    assert_eq!(s.comments.len(), before, "no new comments");
    let body = |id: u64| s.comments.iter().find(|c| c.id == id).unwrap().body.clone();
    assert!(body(mine).contains("2026-10-22T10:40:00Z"));
    assert!(body(mine).starts_with("Claimed."), "human text kept");
    assert!(body(theirs).contains("2026-10-22T10:20:00Z"));
    assert!(body(s.comments.iter().find(|c| c.issue == reg).unwrap().id).contains("10:40:00Z"));
    drop(s);
    let posts_after = fake
        .requests()
        .iter()
        .filter(|r| r.method == "POST")
        .count();
    assert_eq!(posts, posts_after, "heartbeat only PATCHes");
}

#[test]
fn heartbeat_without_a_registry_reports_false() {
    let fake = FakeGithub::start();
    let (code, v) = run(&fake, "session_a", T0, &["heartbeat"]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["registry"], false);
    assert_eq!(v["data"]["leases"], serde_json::json!([]));
}
