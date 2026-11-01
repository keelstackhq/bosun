//! `bosun wait` keeps the caller alive (F7, F18), reaps what expired (F10) and shows only the
//! newest run of each check (F8).

mod common;

use common::fake::FakeGithub;
use common::{bosun, lease_note, one_object};
use serde_json::{Value, json};

const T0: &str = "2026-10-22T10:00:00Z";

fn wait(fake: &FakeGithub, session: &str, args: &[&str]) -> (i32, Value) {
    wait_env(fake, session, args, "600000")
}

fn wait_env(fake: &FakeGithub, session: &str, args: &[&str], beat_ms: &str) -> (i32, Value) {
    let out = bosun()
        .env("BOSUN_WAIT_BEAT_MS", beat_ms)
        .env("BOSUN_GITHUB_API", fake.url())
        .env("BOSUN_WAIT_BASE_MS", "20")
        .env("BOSUN_SESSION", session)
        .env("BOSUN_NOW", T0)
        .arg("wait")
        .args(args)
        .output()
        .unwrap();
    (out.status.code().unwrap(), one_object(&out.stdout))
}

fn join(session: &str, expires: &str) -> String {
    format!(
        r#"{{"bosun":"join","session":"{session}","origin":"spawned","depth":2,"expires":"{expires}"}}"#
    )
}

#[test]
fn wait_renews_the_callers_leases_and_registry_record() {
    let fake = FakeGithub::start();
    fake.issue(1, "bosun registry", &["bosun:registry"], "");
    let reg = fake.comment(1, &join("session_c", "2026-10-22T10:05:00Z"));
    fake.issue(10, "Epic", &["bosun:state/claimed"], "");
    let mine = fake.comment(
        10,
        &lease_note("session_c", "2026-10-22T10:05:00Z", "src/a.rs"),
    );
    let theirs = fake.comment(
        10,
        &lease_note("session_z", "2026-10-22T10:05:00Z", "src/b.rs"),
    );
    fake.pull(5, "Work", "claude/work", "");

    let (code, v) = wait(&fake, "session_c", &["--pr", "5", "--timeout", "0"]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["heartbeat"]["leases"], json!([10]));
    assert_eq!(v["data"]["heartbeat"]["registry"], true);
    let body = |id: u64| {
        let s = fake.state.lock().unwrap();
        s.comments.iter().find(|c| c.id == id).unwrap().body.clone()
    };
    assert!(
        body(mine).contains("2026-10-22T10:30:00Z"),
        "{}",
        body(mine)
    );
    assert!(body(reg).contains("2026-10-22T10:30:00Z"), "{}", body(reg));
    assert!(
        body(theirs).contains("2026-10-22T10:05:00Z"),
        "not another's"
    );
}

#[test]
fn wait_reaps_expired_leases_and_members() {
    let fake = FakeGithub::start();
    fake.issue(1, "bosun registry", &["bosun:registry"], "");
    fake.comment(1, &join("session_gone", "2026-10-22T09:00:00Z"));
    fake.issue(
        11,
        "Task",
        &["bosun:state/claimed"],
        r#"<!-- bosun:task {"item":"M1","task":"T1","touch":["src/a.rs"]} -->"#,
    );
    fake.comment(
        11,
        &lease_note("session_gone", "2026-10-22T09:00:00Z", "src/a.rs"),
    );
    fake.issue(7, "Watched", &["bosun:state/review"], "");

    let (code, v) = wait(&fake, "session_c", &["--issue", "7", "--timeout", "0"]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(
        v["data"]["heartbeat"]["reaped"],
        json!([{"issue": 11, "session": "session_gone"}])
    );
    assert_eq!(v["data"]["heartbeat"]["left"], json!(["session_gone"]));
    assert!(
        fake.labels_of(11)
            .contains(&"bosun:state/ready".to_string())
    );
}

#[test]
fn only_the_newest_run_of_a_check_counts() {
    let fake = FakeGithub::start();
    fake.pull(5, "Work", "claude/work", "");
    // GitHub does not list runs oldest first: the re-run (higher id) comes first here.
    fake.check_run("sha5", "ci", "completed", Some("success"));
    fake.check_run("sha5", "ci", "completed", Some("failure"));
    {
        let mut s = fake.state.lock().unwrap();
        let runs = s.check_runs.get_mut("sha5").unwrap();
        runs[0]["id"] = json!(9);
        runs[1]["id"] = json!(3);
    }
    let (code, v) = wait(&fake, "session_c", &["--pr", "5", "--timeout", "0"]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["after"]["checks"]["ci"], "success");
}

#[test]
fn a_long_wait_beats_again() {
    let fake = FakeGithub::start();
    fake.issue(1, "bosun registry", &["bosun:registry"], "");
    fake.comment(1, &join("session_c", "2026-10-22T10:05:00Z"));
    fake.pull(5, "Work", "claude/work", "");
    let (code, v) = wait_env(&fake, "session_c", &["--pr", "5", "--timeout", "1"], "0");
    assert_eq!(code, 0, "{v}");
    let beats = v["data"]["heartbeat"]["beats"].as_u64().unwrap();
    assert!(beats >= 2, "beats again while polling: {v}");
    assert_eq!(v["data"]["heartbeat"]["error"], Value::Null);
}

#[test]
fn a_halt_stops_the_heartbeat_but_not_the_wait() {
    let fake = FakeGithub::start();
    fake.issue(1, "bosun registry", &["bosun:registry"], "");
    let reg = fake.comment(1, &join("session_c", "2026-10-22T10:05:00Z"));
    fake.pull(5, "Work", "claude/work", "");
    // The switch opens after the wait's own start check.
    fake.after_requests("/actions/variables/BOSUN_HALT", 1, |s| {
        s.variables.insert("BOSUN_HALT".into(), "1".into());
    });
    let (code, v) = wait(&fake, "session_c", &["--pr", "5", "--timeout", "0"]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["heartbeat"]["beats"], 0);
    assert!(
        v["data"]["heartbeat"]["error"]
            .as_str()
            .unwrap()
            .contains("halted")
    );
    let s = fake.state.lock().unwrap();
    let body = &s.comments.iter().find(|c| c.id == reg).unwrap().body;
    assert!(body.contains("2026-10-22T10:05:00Z"), "not renewed: {body}");
}
