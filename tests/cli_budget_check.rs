//! `bosun budget check` (M4 T2, UC-M4-2: at 100 %, the janitor stops the live sessions).
mod common;

use common::fake::FakeGithub;
use common::run;
use serde_json::{Value, json};

const T0: &str = "2026-10-22T10:00:00Z";
const LIVE: &str = "2026-10-22T10:20:00Z";
const GONE: &str = "2026-10-22T09:00:00Z";

fn join_note(session: &str, expires: &str) -> String {
    format!(
        r#"{{"bosun":"join","session":"{session}","origin":"spawned","depth":2,"expires":"{expires}"}}"#
    )
}

fn spend(usd: f64, at: &str) -> String {
    format!(r#"{{"bosun":"spend","session":"s","usd":{usd},"at":"{at}"}}"#)
}

/// Registry with the caller, three live members and one expired member, and `usd` spent today.
fn setup(usd: f64) -> FakeGithub {
    let fake = FakeGithub::start();
    fake.issue(1, "bosun registry", &["bosun:registry"], "");
    for s in ["session_me", "session_a", "session_b", "session_c"] {
        fake.comment(1, &join_note(s, LIVE));
    }
    fake.comment(1, &join_note("session_old", GONE));
    fake.comment(1, &spend(usd, "2026-10-22T03:00:00Z"));
    fake
}

fn check(fake: &FakeGithub) -> Value {
    let (code, v) = run(fake, "session_me", T0, &["budget", "check"]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["command"], "budget check");
    v
}

#[test]
fn uc_m4_2_at_100_percent_interrupts_every_other_live_session() {
    let fake = setup(150.0);
    let v = check(&fake);
    assert_eq!(v["data"]["level"], "stop");
    assert_eq!(v["data"]["pct"], 100.0);
    assert_eq!(v["data"]["spent_today_usd"], 150.0);
    assert_eq!(v["data"]["daily_usd"], 150.0);
    assert_eq!(
        v["data"]["stopped"],
        json!(["session_a", "session_b", "session_c"])
    );
    let next = v["next"].as_array().unwrap();
    assert_eq!(next.len(), 3, "{v}");
    for (n, id) in next.iter().zip(["session_a", "session_b", "session_c"]) {
        assert_eq!(n["tool"], "interrupt_session");
        assert_eq!(n["args"], json!({"session_id": id}));
    }
}

#[test]
fn uc_m4_2_wave_agents_fold_into_their_session_and_local_ids_are_skipped() {
    let fake = FakeGithub::start();
    fake.issue(1, "bosun registry", &["bosun:registry"], "");
    for s in [
        "session_a",
        "session_a/M4-T1",
        "session_b/M4-T2",
        "local-host",
        "session_me/M4-T3",
    ] {
        fake.comment(1, &join_note(s, LIVE));
    }
    fake.comment(1, &spend(150.0, "2026-10-22T03:00:00Z"));
    let v = check(&fake);
    assert_eq!(v["data"]["stopped"], json!(["session_a", "session_b"]));
    let next = v["next"].as_array().unwrap();
    assert_eq!(next.len(), 2, "{v}");
    for (n, id) in next.iter().zip(["session_a", "session_b"]) {
        assert_eq!(n["tool"], "interrupt_session");
        assert_eq!(n["args"], json!({"session_id": id}));
    }
}

#[test]
fn uc_m4_2_at_85_percent_is_guard_and_stops_nobody() {
    let v = check(&setup(127.5));
    assert_eq!(v["data"]["level"], "guard");
    assert_eq!(v["data"]["stopped"], json!([]));
    assert_eq!(v["next"], json!([]));
}

#[test]
fn uc_m4_2_below_80_percent_is_normal() {
    let v = check(&setup(60.0));
    assert_eq!(v["data"]["level"], "normal");
    assert_eq!(v["data"]["pct"], 40.0);
    assert_eq!(v["next"], json!([]));
}

#[test]
fn yesterdays_spend_is_ignored() {
    let fake = setup(10.0);
    fake.comment(1, &spend(900.0, "2026-10-21T23:00:00Z"));
    let v = check(&fake);
    assert_eq!(v["data"]["spent_today_usd"], 10.0);
    assert_eq!(v["data"]["level"], "normal");
}

#[test]
fn no_registry_means_zero_spend() {
    let v = check(&FakeGithub::start());
    assert_eq!(v["data"]["spent_today_usd"], 0.0);
    assert_eq!(v["data"]["level"], "normal");
}

#[test]
fn limits_on_the_default_branch_decide_the_daily_budget() {
    let fake = setup(60.0);
    fake.content(".bosun/limits.toml", "main", "[budget]\ndaily_usd = 50.0\n");
    let v = check(&fake);
    assert_eq!(v["data"]["daily_usd"], 50.0);
    assert_eq!(v["data"]["level"], "stop");
    assert_eq!(v["next"].as_array().unwrap().len(), 3);
}

#[test]
fn it_still_works_when_halted_and_writes_nothing() {
    let fake = setup(150.0);
    fake.issue(2, "halt", &["bosun:halt"], "");
    let v = check(&fake);
    assert_eq!(v["data"]["level"], "stop");
    let writes: Vec<_> = fake
        .requests()
        .into_iter()
        .filter(|r| r.method != "GET")
        .collect();
    assert!(writes.is_empty(), "{writes:?}");
}
