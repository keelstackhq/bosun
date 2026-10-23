//! UC-M4-1: at 80 % of the daily budget, spawn and claim refuse with exit 6, except mainfix.

mod common;

use common::fake::FakeGithub;
use common::run;
use serde_json::Value;

const NOW: &str = "2026-10-22T10:00:00Z";
const READY: &str = "bosun:state/ready";

fn spend(fake: &FakeGithub, usd: f64, at: &str) {
    fake.comment(
        1,
        &format!(r#"{{"bosun":"spend","session":"s","usd":{usd},"at":"{at}"}}"#),
    );
}

/// Registry #1, ready task #10 (feature) and ready mainfix task #11. Default daily budget 150.
fn setup() -> FakeGithub {
    let fake = FakeGithub::start();
    fake.issue(1, "bosun registry", &["bosun:registry"], "");
    let body = |kind: &str, t: &str| {
        format!(
            r#"<!-- bosun:task {{"item":"M4","task":"{t}","touch":["{t}/**"],"kind":"{kind}"}} -->"#
        )
    };
    fake.issue(10, "task", &[READY], &body("feature", "a"));
    fake.issue(11, "fix", &[READY], &body("mainfix", "b"));
    fake
}

fn claim(fake: &FakeGithub, n: &str) -> (i32, Value) {
    run(fake, "session_a", NOW, &["claim", n])
}

#[test]
fn uc_m4_1_below_80_percent_claims() {
    let fake = setup();
    spend(&fake, 119.99, "2026-10-22T03:00:00Z");
    let (code, v) = claim(&fake, "10");
    assert_eq!(code, 0, "{v}");
}

#[test]
fn uc_m4_1_exactly_80_percent_refuses_with_exit_6_and_no_lease() {
    let fake = setup();
    spend(&fake, 120.0, "2026-10-22T03:00:00Z");
    let (code, v) = claim(&fake, "10");
    assert_eq!(code, 6, "{v}");
    assert_eq!(v["data"]["check"], 2);
    assert_eq!(v["data"]["name"], "budget");
    assert_eq!(v["data"]["spent_today_usd"], 120.0);
    assert_eq!(v["data"]["daily_usd"], 150.0);
    assert_eq!(v["data"]["pct"], 80.0);
    assert!(v["error"]["message"].as_str().unwrap().contains("$150.00"));
    assert!(fake.comments_of(10).is_empty(), "a refusal writes no lease");
    assert!(fake.labels_of(10).contains(&READY.into()));
}

#[test]
fn uc_m4_1_yesterdays_spend_does_not_count() {
    let fake = setup();
    spend(&fake, 900.0, "2026-10-21T23:59:00Z");
    let (code, v) = claim(&fake, "10");
    assert_eq!(code, 0, "{v}");
}

#[test]
fn uc_m4_1_mainfix_is_claimed_at_90_percent() {
    let fake = setup();
    spend(&fake, 135.0, "2026-10-22T03:00:00Z");
    let (code, v) = claim(&fake, "11");
    assert_eq!(code, 0, "{v}");
    // The label alone also marks mainfix.
    fake.issue(
        12,
        "fix2",
        &[READY, "bosun:kind/mainfix"],
        r#"<!-- bosun:task {"touch":["c/**"]} -->"#,
    );
    let (code, v) = claim(&fake, "12");
    assert_eq!(code, 0, "{v}");
}

#[test]
fn uc_m4_1_budget_comes_from_limits_on_the_default_branch() {
    let fake = setup();
    spend(&fake, 50.0, "2026-10-22T03:00:00Z");
    fake.content(".bosun/limits.toml", "main", "[budget]\ndaily_usd = 60.0\n");
    let (code, v) = claim(&fake, "10");
    assert_eq!(code, 6, "{v}");
    assert_eq!(v["data"]["daily_usd"], 60.0);

    let fake = setup();
    spend(&fake, 200.0, "2026-10-22T03:00:00Z");
    fake.content(
        ".bosun/limits.toml",
        "main",
        "[budget]\ndaily_usd = 1000.0\n",
    );
    let (code, v) = claim(&fake, "10");
    assert_eq!(code, 0, "{v}");
}

#[test]
fn uc_m4_1_reclaiming_my_own_live_lease_still_succeeds() {
    let fake = setup();
    let (code, v) = claim(&fake, "10");
    assert_eq!(code, 0, "{v}");
    spend(&fake, 140.0, "2026-10-22T03:00:00Z");
    let (code, v) = claim(&fake, "10");
    assert_eq!(code, 0, "{v}");
    assert_eq!(fake.comments_of(10).len(), 1, "no second lease");
}

#[test]
fn uc_m4_1_spawn_refuses_at_80_percent_and_a_mainfix_epic_passes() {
    let fake = setup();
    fake.comment(
        1,
        r#"{"bosun":"join","session":"session_a","origin":"spawned","depth":1,"expires":"2026-10-22T10:20:00Z"}"#,
    );
    spend(&fake, 120.0, "2026-10-22T03:00:00Z");
    let (code, v) = run(&fake, "session_a", NOW, &["spawn", "10"]);
    assert_eq!(code, 6, "{v}");
    assert_eq!(v["data"]["name"], "budget");
    let (code, v) = run(&fake, "session_a", NOW, &["spawn", "11"]);
    assert_eq!(code, 0, "{v}");
}
