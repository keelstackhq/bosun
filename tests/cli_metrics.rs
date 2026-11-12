//! `bosun metrics`: red-CI facts, the brake, cost, lead time, owner touches, blocks per rule.
mod common;

use common::fake::FakeGithub;
use common::{bosun, one_object, run};
use serde_json::{Value, json};

const NOW: &str = "2026-11-12T12:00:00Z";
const ACTIONS: &str = "github-actions";

fn merged(fake: &FakeGithub, n: u64, at: &str) {
    fake.pull_merged(n, &format!("m{n}"));
    for k in ["merged_at", "closed_at", "updated_at"] {
        fake.pull_set(n, k, json!(at));
    }
}

fn pr(fake: &FakeGithub, n: u64, body: &str, heads: &[&str]) {
    fake.pull(n, &format!("PR {n}"), &format!("feat/{n}"), body);
    fake.pull_commit_shas(n, heads);
}

fn metrics(fake: &FakeGithub, args: &[&str]) -> Value {
    let mut a = vec!["metrics"];
    a.extend(args);
    let (code, v) = run(fake, "session_a", NOW, &a);
    assert_eq!(code, 0, "{v}");
    v["data"].clone()
}

/// Two merged PRs whose merge commits failed `ci-ok`.
fn two_red_mains(fake: &FakeGithub) {
    for n in [10, 11] {
        pr(fake, n, "", &[]);
        merged(fake, n, "2026-11-10T10:00:00Z");
        fake.check_run_by(&format!("m{n}"), "ci-ok", "failure", ACTIONS);
    }
}

#[test]
fn two_red_main_merges_trip_the_brake_and_no_main_red_issue_is_read() {
    let fake = FakeGithub::start();
    fake.issue(1, "main is red", &["main-red"], "");
    fake.close_issue(1, "2026-11-11T00:00:00Z");
    two_red_mains(&fake);
    let d = metrics(&fake, &[]);
    assert_eq!(d["brake"]["on"], true, "{d}");
    assert_eq!(d["brake"]["window"]["main_red"], 2);
    assert_eq!(d["brake"]["window"]["days"], 7);
    let why = d["brake"]["reasons"][0].as_str().unwrap();
    assert!(why.contains("main went red 2"), "{why}");
    for r in fake.requests() {
        assert!(!r.path.contains("main-red"), "{}", r.path);
        assert_eq!(r.method, "GET");
    }
}

#[test]
fn a_green_rerun_still_counts_and_other_sources_do_not() {
    let fake = FakeGithub::start();
    pr(&fake, 20, "", &["h20"]);
    fake.check_run_by("h20", "ci-ok", "failure", ACTIONS);
    fake.check_run_by("h20", "ci-ok", "success", ACTIONS);
    pr(&fake, 21, "", &["h21"]);
    fake.check_run_by("h21", "ci-ok", "failure", "some-other-app");
    pr(&fake, 22, "", &["h22"]);
    fake.status("h22", "ci-ok", "failure", "red");
    let d = metrics(&fake, &[]);
    assert_eq!(
        d["red_ci"],
        json!({"failing_heads": 1, "pushed_heads": 1, "rate_pct": 100.0})
    );
}

fn revert_pr(fake: &FakeGithub, n: u64, user: &str) {
    let body = format!("Reverts #1.\n\n<!-- bosun:revert {n} -->");
    pr(fake, n, &body, &[]);
    fake.pull_set(n, "user", json!({"login": user}));
    merged(fake, n, "2026-11-11T10:00:00Z");
}

#[test]
fn only_a_writers_revert_marker_counts() {
    let fake = FakeGithub::start();
    fake.permission("reader", "read");
    revert_pr(&fake, 30, "reader");
    assert_eq!(metrics(&fake, &[])["brake"]["window"]["reverts"], 0);
    revert_pr(&fake, 31, "octocat");
    assert_eq!(metrics(&fake, &[])["brake"]["window"]["reverts"], 1);
    revert_pr(&fake, 32, "octocat");
    let d = metrics(&fake, &[]);
    assert_eq!(d["brake"]["window"]["reverts"], 2);
    assert_eq!(d["brake"]["on"], true);
}

fn run_in(fake: &FakeGithub, mode: Option<&str>) -> Value {
    let dir = tempfile::tempdir().unwrap();
    if let Some(m) = mode {
        std::fs::create_dir_all(dir.path().join(".bosun")).unwrap();
        let toml = format!("[mode]\nname = \"{m}\"\n");
        std::fs::write(dir.path().join(".bosun/bosun.toml"), toml).unwrap();
    }
    let out = bosun()
        .current_dir(dir.path())
        .env("BOSUN_GITHUB_API", fake.url())
        .env("BOSUN_NOW", NOW)
        .arg("metrics")
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(0));
    one_object(&out.stdout)["data"].clone()
}

#[test]
fn the_knob_follows_the_mode_and_a_red_rate_trips_steady_only() {
    let fake = FakeGithub::start();
    pr(&fake, 40, "", &["h40"]);
    fake.check_run_by("h40", "ci-ok", "failure", ACTIONS);
    pr(&fake, 41, "", &["h41"]);
    fake.check_run_by("h41", "ci-ok", "success", ACTIONS);
    let g = run_in(&fake, None);
    assert_eq!(g["mode"], "gatekeeper");
    assert_eq!(g["brake"]["knob"], "off");
    assert_eq!(g["brake"]["on"], false);
    assert_eq!(g["red_ci"]["rate_pct"], 50.0);
    let s = run_in(&fake, Some("steady"));
    assert_eq!(s["brake"]["knob"], "on");
    assert_eq!(s["brake"]["on"], true);
    let why = s["brake"]["reasons"][0].as_str().unwrap();
    assert!(why.contains("red-CI rate 1/2"), "{why}");
}

#[test]
fn a_gatekeeper_still_trips_on_main_red() {
    let fake = FakeGithub::start();
    two_red_mains(&fake);
    let g = run_in(&fake, None);
    assert_eq!(
        (g["brake"]["knob"].as_str(), g["brake"]["on"].as_bool()),
        (Some("off"), Some(true))
    );
}

#[test]
fn propose_is_not_built_and_zero_days_is_usage() {
    let fake = FakeGithub::start();
    let (code, v) = run(&fake, "s", NOW, &["metrics", "--propose"]);
    assert_eq!(code, 1);
    assert!(v["error"]["fix"].as_str().unwrap().contains("plan PR 8"));
    let (code, v) = run(&fake, "s", NOW, &["metrics", "--days", "0"]);
    assert_eq!(code, 2, "{v}");
}

fn task(fake: &FakeGithub, n: u64, epic: u64, claimed: &str, usd: f64) {
    let body = format!(r#"<!-- bosun:task {{"item":"M1","task":"T{n}","epic":{epic}}} -->"#);
    fake.issue(n, &format!("Task {n}"), &[], &body);
    let lease = r#"{"bosun":"lease","session":"s1","origin":"local","expires":"2026-11-20T00:00:00Z","touch":["a"]}"#;
    fake.comment_at(n, "octocat", lease, claimed);
    let spend = format!(r#"{{"bosun":"spend","session":"s1","usd":{usd},"at":"{claimed}"}}"#);
    fake.comment(n, &spend);
}

fn risk(task: u64, rules: &[&str]) -> String {
    let bullets: String = rules.iter().map(|r| format!("- **{r}**: why\n")).collect();
    format!(
        "Closes #{task}\n<!-- bosun:risk -->\n## Risk: high\n\n{bullets}\nOwner: review the head SHA x and approve.\n<!-- /bosun:risk -->"
    )
}

#[test]
fn cost_lead_time_touches_and_blocks() {
    let fake = FakeGithub::start();
    fake.content(
        "governance/owners.toml",
        "main",
        "[[owner]]\ngithub = \"boss\"\n",
    );
    task(&fake, 100, 50, "2026-11-01T00:00:00Z", 2.0);
    task(&fake, 101, 50, "2026-11-02T00:00:00Z", 1.0);
    task(&fake, 102, 51, "2026-11-04T00:00:00Z", 3.0);
    // 20: one head, passed. 21: two heads, caught. 22: no risk note.
    pr(&fake, 20, &risk(100, &["R-gov"]), &["h20"]);
    pr(&fake, 21, &risk(101, &["R-gov"]), &["h21a", "h21b"]);
    pr(&fake, 22, "Closes #102", &["h22"]);
    for h in ["h20", "h21a", "h21b", "h22"] {
        fake.check_run_by(h, "ci-ok", "success", ACTIONS);
    }
    merged(&fake, 20, "2026-11-03T00:00:00Z");
    merged(&fake, 21, "2026-11-05T00:00:00Z");
    merged(&fake, 22, "2026-11-04T12:00:00Z");
    pr(&fake, 23, &risk(0, &["R-gate"]), &["h23"]);
    fake.check_run_by("h23", "ci-ok", "failure", ACTIONS);
    fake.pull_set(23, "state", json!("closed"));
    fake.pull_set(23, "closed_at", json!("2026-11-06T00:00:00Z"));
    pr(&fake, 24, &risk(0, &["R-gate"]), &[]); // open: skipped
    for n in [20, 21] {
        fake.pull_as_issue(n, "pr");
    }
    let answer = r#"{"bosun":"answer","session":"s","at":"2026-11-04T00:00:00Z"}"#;
    fake.comment_at(20, "boss", "/bosun approve 20 x", "2026-11-04T00:00:00Z");
    fake.comment_at(100, "boss", answer, "2026-11-04T00:00:00Z");
    fake.comment_at(20, "boss", "thanks", "2026-11-04T01:00:00Z");
    fake.comment_at(21, "octocat", "/bosun approve 21 x", "2026-11-04T01:00:00Z");
    fake.comment_at(21, "boss", "/bosun approve 21 x", "2026-09-01T00:00:00Z");
    let d = metrics(&fake, &[]);
    assert_eq!(d["merged"], 3);
    assert_eq!(d["cost_per_merged_usd"], 2.0);
    assert_eq!(d["epics"], 2);
    assert_eq!(d["lead_time_p50_hours"], 54.0);
    assert_eq!(d["owner_touches"], 2);
    assert_eq!(d["owner_touches_per_day"], 0.07);
    assert_eq!(
        d["blocks"],
        json!([{"rule": "R-gate", "blocks": 1, "caught": 1, "passed": 0},
               {"rule": "R-gov", "blocks": 2, "caught": 1, "passed": 1}])
    );
    assert_eq!(d["days"], 30);
    assert_eq!(d["since"], "2026-10-13T12:00:00Z");
    assert_eq!(
        metrics(&fake, &["--days", "10"])["owner_touches_per_day"],
        0.2
    );
}
