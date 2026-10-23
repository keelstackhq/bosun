mod common;

use common::fake::FakeGithub;
use common::{lease_note, run};

const NOW: &str = "2026-10-22T11:00:00Z";
const EXPIRED: &str = "2026-10-22T10:50:00Z";
const LIVE: &str = "2026-10-22T11:20:00Z";
const NEW_EXPIRY: &str = "2026-10-22T11:30:00Z";

fn join_note(session: &str, expires: &str) -> String {
    format!(
        "{{\"bosun\":\"join\",\"session\":\"{session}\",\"origin\":\"local\",\"depth\":1,\"expires\":\"{expires}\"}}"
    )
}

fn registry(fake: &FakeGithub) -> u64 {
    fake.issue(1, "bosun registry", &["bosun:registry"], "");
    1
}

#[test]
fn uc_m1_4_resume_continues_the_open_pr_after_a_restart() {
    let fake = FakeGithub::start();
    let reg = registry(&fake);
    fake.comment(reg, &join_note("session_a", EXPIRED));
    fake.issue(10, "the task", &["bosun:state/review"], "");
    let lease = fake.comment(10, &lease_note("session_a", EXPIRED, "src/core/**"));
    fake.pull(20, "feat: the task", "claude/the-task", "Closes #10");

    let (code, v) = run(&fake, "session_a", NOW, &["resume"]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["command"], "resume");
    assert_eq!(v["data"]["session"], "session_a");
    assert_eq!(v["data"]["joined"], false);
    let t = &v["data"]["tasks"][0];
    assert_eq!(t["issue"], 10);
    assert_eq!(t["state"], "review");
    assert_eq!(t["pr"], 20);
    assert_eq!(t["branch"], "claude/the-task");
    assert_eq!(t["head_sha"], "sha20");
    assert!(t["action"].as_str().unwrap().contains("bosun wait --pr 20"));
    assert_eq!(v["data"]["lost"].as_array().unwrap().len(), 0);

    let comments = fake.comments_of(10);
    assert_eq!(comments.len(), 1, "edited in place");
    assert_eq!(comments[0].id, lease);
    assert!(
        comments[0].body.contains(NEW_EXPIRY),
        "{}",
        comments[0].body
    );
    let join = &fake.comments_of(reg)[0];
    assert!(join.body.contains(NEW_EXPIRY));
}

#[test]
fn a_claimed_task_without_a_pr_says_to_continue_building() {
    let fake = FakeGithub::start();
    registry(&fake);
    fake.issue(10, "the task", &["bosun:state/claimed"], "");
    fake.comment(10, &lease_note("session_a", LIVE, "src/core/**"));
    let (code, v) = run(&fake, "session_a", NOW, &["resume"]);
    assert_eq!(code, 0, "{v}");
    let t = &v["data"]["tasks"][0];
    assert!(t["pr"].is_null() && t["branch"].is_null() && t["head_sha"].is_null());
    assert!(t["action"].as_str().unwrap().contains("bosun pr 10"));
}

#[test]
fn a_building_task_with_a_pr_says_to_switch_to_the_branch() {
    let fake = FakeGithub::start();
    registry(&fake);
    fake.issue(10, "the task", &["bosun:state/fixing"], "");
    fake.comment(10, &lease_note("session_a", LIVE, "src/core/**"));
    fake.pull(20, "feat: the task", "claude/the-task", "Closes #10");
    let (_, v) = run(&fake, "session_a", NOW, &["resume"]);
    let action = v["data"]["tasks"][0]["action"].as_str().unwrap();
    assert!(action.contains("git switch claude/the-task") && action.contains("bosun pr 10"));
}

#[test]
fn a_lease_of_another_session_is_not_reported() {
    let fake = FakeGithub::start();
    registry(&fake);
    fake.issue(10, "theirs", &["bosun:state/building"], "");
    let body = lease_note("session_b", EXPIRED, "src/core/**");
    fake.comment(10, &body);
    let (code, v) = run(&fake, "session_a", NOW, &["resume"]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["tasks"].as_array().unwrap().len(), 0);
    assert_eq!(v["data"]["lost"].as_array().unwrap().len(), 0);
    assert_eq!(fake.comments_of(10)[0].body, body);
}

#[test]
fn an_expired_lease_that_another_lease_now_overlaps_is_lost() {
    let fake = FakeGithub::start();
    registry(&fake);
    fake.issue(10, "mine", &["bosun:state/building"], "");
    let mine = lease_note("session_a", EXPIRED, "src/core/**");
    fake.comment(10, &mine);
    fake.issue(11, "theirs", &["bosun:state/building"], "");
    fake.comment(11, &lease_note("session_b", LIVE, "src/core/lease.rs"));
    let (code, v) = run(&fake, "session_a", NOW, &["resume"]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["tasks"].as_array().unwrap().len(), 0);
    let lost = &v["data"]["lost"][0];
    assert_eq!(lost["issue"], 10);
    assert_eq!(lost["conflicts"][0]["issue"], 11);
    assert_eq!(lost["conflicts"][0]["session"], "session_b");
    assert_eq!(fake.comments_of(10)[0].body, mine, "not renewed");
}

#[test]
fn a_session_without_a_join_record_is_joined() {
    let fake = FakeGithub::start();
    let (code, v) = run(&fake, "session_a", NOW, &["resume"]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["joined"], true);
    assert_eq!(v["data"]["tasks"].as_array().unwrap().len(), 0);
    let s = fake.state.lock().unwrap();
    let reg = s
        .issues
        .iter()
        .find(|i| i.labels.iter().any(|l| l == "bosun:registry"));
    assert!(reg.is_some());
}

#[test]
fn resume_is_halted_by_the_kill_switch_and_writes_nothing() {
    let fake = FakeGithub::start();
    fake.variable("BOSUN_HALT", "1");
    fake.issue(10, "mine", &["bosun:state/building"], "");
    let body = lease_note("session_a", EXPIRED, "src/core/**");
    fake.comment(10, &body);
    let (code, v) = run(&fake, "session_a", NOW, &["resume"]);
    assert_eq!(code, 4, "{v}");
    assert_eq!(v["error"]["kind"], "halted");
    assert_eq!(fake.comments_of(10)[0].body, body);
    assert!(fake.requests().iter().all(|r| r.method == "GET"));
}

#[test]
fn an_expired_lease_with_a_bad_glob_is_lost_not_renewed() {
    let fake = FakeGithub::start();
    registry(&fake);
    fake.issue(10, "mine", &["bosun:state/building"], "");
    let body = lease_note("session_a", EXPIRED, "src/[core");
    fake.comment(10, &body);
    let (code, v) = run(&fake, "session_a", NOW, &["resume"]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["tasks"].as_array().unwrap().len(), 0);
    assert_eq!(v["data"]["lost"][0]["issue"], 10);
    assert!(!v["data"]["lost"][0]["fix"].as_str().unwrap().is_empty());
    assert_eq!(fake.comments_of(10)[0].body, body);
}
