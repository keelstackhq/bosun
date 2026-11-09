//! `bosun budget record --pr` and the PR's own spend records in `budget report` (spec §9).
mod common;

use common::fake::FakeGithub;
use common::{bosun, one_object, run};
use serde_json::{Value, json};

const NOW: &str = "2026-11-09T12:00:00Z";
const REVIEW: &str = "bosun:state/review";
const SESSION: &str = r#"{"session_id":"session_w","session_context":{"model":"claude-sonnet-5-5"},"usage":{"cost_usd":3.0}}"#;

fn record(fake: &FakeGithub, stdin: &str, args: &[&str]) -> (i32, Value) {
    let out = bosun()
        .env("BOSUN_GITHUB_API", fake.url())
        .env("BOSUN_SESSION", "session_o")
        .env("BOSUN_NOW", NOW)
        .args(["budget", "record"])
        .args(args)
        .write_stdin(stdin.to_string())
        .output()
        .unwrap();
    let v = one_object(&out.stdout);
    if v["ok"] == false {
        assert!(!v["error"]["fix"].as_str().unwrap().trim().is_empty());
    }
    (out.status.code().unwrap(), v)
}

fn spends(fake: &FakeGithub, n: u64) -> Vec<Value> {
    fake.comments_of(n)
        .iter()
        .filter(|c| c.body.contains(r#""bosun":"spend""#))
        .map(|c| serde_json::from_str(&c.body).unwrap())
        .collect()
}

fn seeded() -> FakeGithub {
    let fake = FakeGithub::start();
    fake.issue(1, "bosun registry", &["bosun:registry"], "");
    // A PR is an issue too: its comments live there.
    for n in [20, 21] {
        fake.issue(n, &format!("PR {n}"), &[], "");
        fake.pull(n, &format!("PR {n}"), &format!("feat/{n}"), "");
    }
    fake
}

#[test]
fn pr_splits_the_session_cost_evenly_and_writes_session_and_model() {
    let fake = seeded();
    let (code, v) = record(&fake, SESSION, &["--pr", "20", "--pr", "21", "--from", "-"]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["usd"], 3.0);
    for pr in [20, 21] {
        let s = spends(&fake, pr);
        assert_eq!(s.len(), 1, "PR {pr}");
        assert_eq!(s[0]["usd"], 1.5);
        assert_eq!(s[0]["session"], "session_w");
        assert_eq!(s[0]["model"], "claude-sonnet-5-5");
    }
}

#[test]
fn pr_refuses_a_second_record_of_the_same_session_and_writes_nothing() {
    let fake = seeded();
    let args = ["--pr", "20", "--pr", "21", "--from", "-"];
    assert_eq!(record(&fake, SESSION, &args).0, 0);
    let (code, v) = record(&fake, SESSION, &args);
    assert_eq!(code, 3, "{v}");
    assert_eq!((spends(&fake, 20).len(), spends(&fake, 21).len()), (1, 1));
}

#[test]
fn usd_overrides_and_needs_no_stdin() {
    let fake = seeded();
    let (code, v) = record(&fake, "", &["--pr", "20", "--usd", "2.5"]);
    assert_eq!(code, 0, "{v}");
    let s = spends(&fake, 20);
    assert_eq!(s[0]["usd"], json!(2.5));
    assert_eq!(s[0]["session"], json!("session_o"));
}

#[test]
fn report_adds_pr_spend_to_task_spend_and_counts_a_session_once() {
    let fake = seeded();
    let s = |sess: &str, usd: f64| {
        format!(r#"{{"bosun":"spend","session":"{sess}","usd":{usd},"at":"2026-11-02T09:00:00Z"}}"#)
    };
    fake.issue(
        10,
        "Task 10",
        &[REVIEW],
        r#"<!-- bosun:task {"item":"M4","task":"T10"} -->"#,
    );
    fake.comment(10, &s("s1", 2.0));
    fake.comment(10, r#"{"bosun":"task-pr","session":"s1","pr":20}"#);
    fake.comment(20, &s("s1", 2.0)); // same session as the task: not counted again
    fake.comment(20, &s("s2", 1.0)); // new session: added
    fake.comment(21, &s("s3", 4.0)); // PR without a task
    for pr in [20, 21] {
        fake.pull_set(pr, "created_at", json!("2026-11-05T10:00:00Z"));
    }
    let (code, v) = run(&fake, "session_a", NOW, &["budget", "report"]);
    assert_eq!(code, 0, "{v}");
    let rows = v["data"]["rows"].as_array().unwrap();
    let by = |pr: u64| rows.iter().find(|r| r["pr"] == pr).unwrap();
    assert_eq!(by(20)["usd"], 3.0);
    assert_eq!(by(20)["sessions"], json!(["s1", "s2"]));
    assert_eq!(by(21)["usd"], 4.0);
    assert_eq!(by(21)["sessions"], json!(["s3"]));
    assert_eq!(v["data"]["totals"]["prs_with_spend"], 2);
}

#[test]
fn session_flag_names_the_child_and_a_second_child_is_no_duplicate() {
    let fake = seeded();
    assert_eq!(
        record(&fake, "", &["--pr", "20", "--usd", "1", "--session", "c1"]).0,
        0
    );
    assert_eq!(
        record(&fake, "", &["--pr", "20", "--usd", "2", "--session", "c2"]).0,
        0
    );
    let s = spends(&fake, 20);
    assert_eq!(
        (s[0]["session"].clone(), s[1]["session"].clone()),
        (json!("c1"), json!("c2"))
    );
}

#[test]
fn a_retry_writes_only_the_prs_that_lack_the_record() {
    let fake = seeded();
    assert_eq!(
        record(&fake, "", &["--pr", "20", "--usd", "1", "--session", "c1"]).0,
        0
    );
    let (code, v) = record(
        &fake,
        "",
        &["--pr", "20", "--pr", "21", "--usd", "2", "--session", "c1"],
    );
    assert_eq!(code, 0, "{v}");
    assert_eq!((spends(&fake, 20).len(), spends(&fake, 21).len()), (1, 1));
    assert_eq!(v["data"]["already_recorded"], json!([20]));
}

#[test]
fn bad_input_is_refused_with_nothing_written() {
    let fake = seeded();
    for (args, code) in [
        (vec!["--usd", "1"], 2),
        (vec!["5", "--pr", "20"], 2),
        (vec!["--pr", "20", "--usd=-1"], 1),
        (vec!["--pr", "99", "--usd", "1"], 1),
    ] {
        assert_eq!(record(&fake, "", &args).0, code, "{args:?}");
    }
    assert!(record(&fake, "", &[]).0 == 2);
    assert!(spends(&fake, 20).is_empty());
    assert_eq!(
        record(&fake, "", &["--pr", "20", "--pr", "20", "--usd", "2"]).0,
        0
    );
    assert_eq!(spends(&fake, 20)[0]["usd"], 2.0);
}
