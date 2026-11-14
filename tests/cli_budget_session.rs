//! `budget record`: get_session shapes, session totals as deltas, the session ledger, `--at` (C7).

mod common;

use common::fake::FakeGithub;
use common::{bosun, one_object};
use serde_json::{Value, json};

const T0: &str = "2026-10-22T10:00:00Z";

fn budget(fake: &FakeGithub, stdin: &str, args: &[&str]) -> (i32, Value) {
    let out = bosun()
        .env("BOSUN_GITHUB_API", fake.url())
        .env("BOSUN_NOW", T0)
        .args(["budget", "record"])
        .args(args)
        .write_stdin(stdin.to_string())
        .output()
        .unwrap();
    (out.status.code().unwrap(), one_object(&out.stdout))
}

fn registry_spends(fake: &FakeGithub) -> Vec<String> {
    let n = {
        let s = fake.state.lock().unwrap();
        s.issues
            .iter()
            .find(|i| i.labels.contains(&"bosun:registry".to_string()))
            .map(|i| i.number)
    };
    n.map(|n| {
        fake.comments_of(n)
            .into_iter()
            .map(|c| c.body)
            .filter(|b| b.contains(r#""bosun":"spend""#))
            .collect()
    })
    .unwrap_or_default()
}

#[test]
fn the_ccr_wrapper_and_external_metadata_shapes_are_read() {
    let fake = FakeGithub::start();
    fake.issue(5, "task", &["bosun:state/building"], "");
    let body = r#"{"ccr":{"id":"session_x","external_metadata":{"usage":{"cost_usd":2.5}}}}"#;
    let (code, v) = budget(&fake, body, &["5", "--from", "-"]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["usd"], 2.5);
}

#[test]
fn a_session_total_records_only_the_delta() {
    let fake = FakeGithub::start();
    fake.issue(5, "task", &["bosun:state/building"], "");
    let at = |usd: f64| format!(r#"{{"session_id":"session_x","usage":{{"cost_usd":{usd}}}}}"#);
    let (_, v) = budget(&fake, &at(2.0), &["5", "--from", "-"]);
    assert_eq!(v["data"]["usd"], 2.0, "{v}");
    let (code, v) = budget(&fake, &at(3.5), &["5", "--from", "-"]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["usd"], 1.5, "{v}");
    assert_eq!(v["data"]["task_total_usd"], 3.5, "{v}");
    let (_, v) = budget(&fake, &at(3.5), &["5", "--from", "-"]);
    assert_eq!(v["data"]["no_new_spend"], true, "{v}");
    assert_eq!(registry_spends(&fake).len(), 2);
}

#[test]
fn no_issue_writes_the_session_ledger_only_with_the_given_day() {
    let fake = FakeGithub::start();
    let body = r#"{"session_id":"session_orch","usage":{"cost_usd":4.0}}"#;
    let (code, v) = budget(&fake, body, &["--from", "-", "--at", "2026-10-20"]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["ledger"], true, "{v}");
    let r = registry_spends(&fake);
    assert_eq!(r.len(), 1);
    assert!(r[0].contains("2026-10-20T12:00:00Z") && !r[0].contains(r#""issue""#));
    let (code, v) = budget(&fake, body, &["--from", "-", "--at", "soon"]);
    assert_eq!(code, 2, "{v}");
}

#[test]
fn the_ledger_counts_for_the_daily_budget() {
    let fake = FakeGithub::start();
    let body = json!({"session_id": "session_orch", "usage": {"cost_usd": 4.0}}).to_string();
    budget(&fake, &body, &["--from", "-"]);
    let out = bosun()
        .env("BOSUN_GITHUB_API", fake.url())
        .env("BOSUN_NOW", T0)
        .args(["budget", "check"])
        .output()
        .unwrap();
    let v = one_object(&out.stdout);
    assert_eq!(v["data"]["spent_today_usd"], 4.0, "{v}");
}

#[test]
fn at_is_bounded_and_a_back_dated_record_still_counts_today() {
    let fake = FakeGithub::start();
    let body = r#"{"session_id":"session_orch","usage":{"cost_usd":4.0}}"#;
    let (code, v) = budget(&fake, body, &["--from", "-", "--at", "2026-10-10"]);
    assert_eq!(code, 2, "{v}");
    let (code, v) = budget(&fake, body, &["--from", "-", "--at", "2026-10-21"]);
    assert_eq!(code, 0, "{v}");
    // The comment was created "now" (the fake's clock), so `check` counts it for today.
    let out = bosun()
        .env("BOSUN_GITHUB_API", fake.url())
        .env("BOSUN_NOW", T0)
        .args(["budget", "check"])
        .output()
        .unwrap();
    let v = one_object(&out.stdout);
    assert_eq!(v["data"]["spent_today_usd"], 4.0, "{v}");
}

#[test]
fn an_id_less_result_records_the_full_amount_without_subtracting() {
    let fake = FakeGithub::start();
    fake.issue(5, "task", &["bosun:state/building"], "");
    let with_id = r#"{"session_id":"session_x","usage":{"cost_usd":2.0}}"#;
    budget(&fake, with_id, &["5", "--from", "-"]);
    let no_id = r#"{"usage":{"cost_usd":3.0}}"#;
    let (code, v) = budget(
        &fake,
        no_id,
        &["5", "--from", "-", "--session", "session_x"],
    );
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["usd"], 3.0, "{v}");
}

#[test]
fn a_session_flag_that_contradicts_the_result_is_refused() {
    let fake = FakeGithub::start();
    let body = r#"{"session_id":"session_x","usage":{"cost_usd":2.0}}"#;
    let (code, v) = budget(&fake, body, &["--from", "-", "--session", "session_y"]);
    assert_eq!(code, 2, "{v}");
    assert!(registry_spends(&fake).is_empty());
}
