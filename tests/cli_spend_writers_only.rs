//! Spend records count only from writers: a forged `spend` comment by an outsider must not push
//! the daily budget, trip a per-task cap or skew `bosun progress`.
mod common;

use common::fake::FakeGithub;
use common::{bosun, one_object, run};
use serde_json::Value;

const NOW: &str = "2026-10-23T12:00:00Z";
const AT: &str = "2026-10-23T03:00:00Z";
const MERGED: &str = "bosun:state/merged";

fn spend(usd: f64, at: &str) -> String {
    format!(r#"{{"bosun":"spend","session":"s","usd":{usd},"at":"{at}"}}"#)
}

fn registry(fake: &FakeGithub) {
    fake.issue(1, "bosun registry", &["bosun:registry"], "");
    fake.permission("mallory", "read");
}

#[test]
fn budget_check_ignores_a_forged_registry_spend() {
    let fake = FakeGithub::start();
    registry(&fake);
    fake.comment_by(1, "mallory", &spend(9999.0, AT));
    let (code, v) = run(&fake, "session_me", NOW, &["budget", "check"]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["spent_today_usd"], 0.0, "{v}");
    assert_ne!(v["data"]["level"], "stop", "{v}");
    // A writer's spend still counts.
    fake.comment(1, &spend(150.0, AT));
    let (_, v) = run(&fake, "session_me", NOW, &["budget", "check"]);
    assert_eq!(v["data"]["spent_today_usd"], 150.0, "{v}");
    assert_eq!(v["data"]["level"], "stop", "{v}");
}

#[test]
fn progress_ignores_forged_registry_and_task_spend() {
    let fake = FakeGithub::start();
    registry(&fake);
    let body = r#"<!-- bosun:task {"item":"M4","task":"T10"} -->"#;
    fake.issue(10, "Task 10", &[MERGED], body);
    fake.close_issue(10, "2026-10-23T09:00:00Z");
    fake.comment(10, &spend(3.0, AT));
    fake.comment_by(10, "mallory", &spend(777.0, AT));
    fake.comment(1, &spend(3.0, AT));
    fake.comment_by(1, "mallory", &spend(500.0, AT));
    let (code, v) = run(&fake, "session_a", NOW, &["progress", "--full"]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["spend_usd"], 3.0, "{v}");
    assert_eq!(v["data"]["tasks"][0]["usd"], 3.0, "{v}");
}

#[test]
fn budget_record_does_not_block_a_task_over_a_forged_spend() {
    let fake = FakeGithub::start();
    fake.permission("mallory", "read");
    fake.issue(5, "task", &["bosun:state/building"], "");
    fake.comment_by(5, "mallory", &spend(9999.0, AT));
    let out = bosun()
        .env("BOSUN_GITHUB_API", fake.url())
        .env("BOSUN_NOW", NOW)
        .args(["budget", "record", "5", "--provider", "claude-sessions"])
        .args(["--from", "-"])
        .write_stdin(r#"{"session":{"usage":{"cost_usd":1.0}}}"#.to_string())
        .output()
        .unwrap();
    let v: Value = one_object(&out.stdout);
    assert_eq!(out.status.code(), Some(0), "{v}");
    assert_eq!(v["data"]["task_total_usd"], 1.0, "{v}");
    assert_eq!(v["data"]["over_task_cap"], false, "{v}");
    assert_eq!(v["data"]["blocked"], false, "{v}");
}

/// review-ci (mode b) posts its spend as github-actions[bot], whose permission lookup is not
/// `write`: that spend must still count, or the daily stop fails open.
#[test]
fn the_workflow_bots_spend_counts() {
    let fake = FakeGithub::start();
    registry(&fake);
    fake.permission("github-actions[bot]", "none");
    fake.comment_by(1, "github-actions[bot]", &spend(150.0, AT));
    let (code, v) = run(&fake, "session_me", NOW, &["budget", "check"]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["spent_today_usd"], 150.0, "{v}");
    assert_eq!(v["data"]["level"], "stop", "{v}");
}
