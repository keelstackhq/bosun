//! `bosun pr` closes the epic of a task whose PR it finds merged (spec section 4).

mod common;

use std::process::Command;

use common::fake::FakeGithub;
use common::{bosun, one_object};
use serde_json::{Value, json};

const NOW: &str = "2026-11-05T10:00:00Z";

fn setup(fake: &FakeGithub, other_task: Option<&str>) {
    setup_epic(fake, other_task, "ready");
}

fn setup_epic(fake: &FakeGithub, other_task: Option<&str>, epic_state: &str) {
    fake.issue(
        9,
        "E1",
        &["bosun:source/plan", &format!("bosun:state/{epic_state}")],
        r#"E: epic <!-- bosun:epic {"item":"E1"} -->"#,
    );
    let task = |n: u64, state: &str| {
        let body = format!(r#"<!-- bosun:task {{"item":"E1","task":"T{n}","epic":9}} -->"#);
        let labels = ["bosun:source/plan", &format!("bosun:state/{state}")];
        fake.issue(n, "task", &labels, &body);
    };
    task(10, "review");
    if let Some(state) = other_task {
        task(11, state);
    }
    fake.pull(20, "T10", "claude/t10", "Closes #10");
    fake.pull_merged(20, "abc");
}

fn pr(fake: &FakeGithub) -> Value {
    pr_as(fake, "session_root")
}

fn spawn_lease(session: &str, expires: &str) -> String {
    format!(
        r#"{{"bosun":"lease","session":"{session}","origin":"spawned","expires":"{expires}","touch":[]}}"#
    )
}

fn pr_as(fake: &FakeGithub, session: &str) -> Value {
    let dir = tempfile::tempdir().unwrap();
    for args in [
        vec!["init", "-q", "-b", "claude/t10"],
        vec![
            "-c",
            "user.name=t",
            "-c",
            "user.email=t@t",
            "commit",
            "-q",
            "--allow-empty",
            "-m",
            "x",
        ],
    ] {
        let st = Command::new("git")
            .arg("-C")
            .arg(dir.path())
            .args(&args)
            .status()
            .unwrap();
        assert!(st.success());
    }
    let out = bosun()
        .env("BOSUN_GITHUB_API", fake.url())
        .env("BOSUN_NOW", NOW)
        .env("BOSUN_SESSION", session)
        .arg("-C")
        .arg(dir.path())
        .args(["pr", "10"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(0), "{out:?}");
    one_object(&out.stdout)
}

#[test]
fn the_last_merged_task_closes_the_epic() {
    let fake = FakeGithub::start();
    setup(&fake, None);
    let v = pr(&fake);
    assert_eq!(v["data"]["merged"], json!(true), "{v}");
    assert_eq!(v["data"]["epic_closed"], json!(true), "{v}");
    let e = fake.issue_state(9);
    assert_eq!(e.state, "closed");
    assert_eq!(e.state_reason.as_deref(), Some("completed"));
}

#[test]
fn another_open_task_keeps_the_epic_open() {
    let fake = FakeGithub::start();
    setup(&fake, Some("ready"));
    let v = pr(&fake);
    assert_eq!(v["data"]["merged"], json!(true), "{v}");
    assert_eq!(v["data"]["epic_closed"], json!(false), "{v}");
    assert_eq!(fake.issue_state(9).state, "open");
}

#[test]
fn a_childs_own_lease_leaves_the_close_to_its_result() {
    let fake = FakeGithub::start();
    setup(&fake, None);
    fake.comment(9, &spawn_lease("session_child", "2026-11-05T11:00:00Z"));
    let v = pr_as(&fake, "session_child");
    assert_eq!(v["data"]["merged"], json!(true), "{v}");
    assert_eq!(v["data"]["epic_closed"], json!(false), "{v}");
    assert_eq!(fake.issue_state(9).state, "open");
}

#[test]
fn an_expired_child_lease_without_a_result_keeps_the_epic_open() {
    let fake = FakeGithub::start();
    setup_epic(&fake, None, "claimed");
    fake.comment(9, &spawn_lease("session_child", "2026-11-05T09:00:00Z"));
    let v = pr(&fake);
    assert_eq!(v["data"]["epic_closed"], json!(false), "{v}");
    assert_eq!(fake.issue_state(9).state, "open");
}

#[test]
fn an_expired_child_lease_on_a_ready_epic_does_not_block() {
    let fake = FakeGithub::start();
    setup(&fake, None);
    fake.comment(9, &spawn_lease("session_child", "2026-11-05T09:00:00Z"));
    let v = pr(&fake);
    assert_eq!(v["data"]["epic_closed"], json!(true), "{v}");
}

#[test]
fn a_roots_own_local_lease_does_not_block() {
    let fake = FakeGithub::start();
    setup(&fake, None);
    fake.comment(
        9,
        &common::lease_note("session_root", "2026-11-05T11:00:00Z", "x/**"),
    );
    let v = pr(&fake);
    assert_eq!(v["data"]["epic_closed"], json!(true), "{v}");
    assert_eq!(fake.issue_state(9).state, "closed");
}
