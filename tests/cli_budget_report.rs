//! `bosun budget report`: spend per PR for one UTC month, read-only, deterministic files.
mod common;

use common::fake::FakeGithub;
use common::run;
use serde_json::{Value, json};

const NOW: &str = "2026-11-09T12:00:00Z";
const REVIEW: &str = "bosun:state/review";

fn spend(session: &str, usd: f64) -> String {
    format!(
        "{{\"bosun\":\"spend\",\"session\":\"{session}\",\"usd\":{usd},\"at\":\"2026-11-02T09:00:00Z\"}}"
    )
}

/// A task with spend (`usd` split over two sessions) and its PR `pr` (title `PR <pr>`).
fn task(fake: &FakeGithub, n: u64, pr: u64, usd: f64) {
    let body = format!(r#"<!-- bosun:task {{"item":"M4","task":"T{n}"}} -->"#);
    fake.issue(n, &format!("Task {n}"), &[REVIEW], &body);
    fake.comment(n, &spend("s1", usd / 2.0));
    fake.comment(n, &spend("s2", usd / 2.0));
    fake.comment(
        n,
        &format!(r#"{{"bosun":"task-pr","session":"s1","pr":{pr}}}"#),
    );
    fake.pull(pr, &format!("PR {pr}"), &format!("feat/{pr}"), "");
}

fn merged(fake: &FakeGithub, pr: u64, at: &str) {
    fake.pull_set(pr, "state", json!("closed"));
    fake.pull_set(pr, "merged", json!(true));
    fake.pull_set(pr, "merged_at", json!(at));
    fake.pull_set(pr, "closed_at", json!(at));
    fake.pull_set(pr, "updated_at", json!(at));
}

fn seeded() -> FakeGithub {
    let fake = FakeGithub::start();
    fake.issue(1, "bosun registry", &["bosun:registry"], "");
    task(&fake, 10, 20, 2.0); // merged in November, held
    task(&fake, 11, 21, 3.0); // open, updated in November
    task(&fake, 12, 22, 5.0); // merged in October: out
    task(&fake, 13, 23, 1.0); // merged in November, then reverted
    task(&fake, 15, 25, 0.5); // closed without a merge in November
    // No spend: out.
    fake.issue(
        14,
        "Task 14",
        &[REVIEW],
        r#"<!-- bosun:task {"item":"M4","task":"T14"} -->"#,
    );
    merged(&fake, 20, "2026-11-03T10:00:00Z");
    fake.pull_set(21, "updated_at", json!("2026-11-05T10:00:00Z"));
    merged(&fake, 22, "2026-10-30T10:00:00Z");
    merged(&fake, 23, "2026-11-04T10:00:00Z");
    fake.pull(40, "Revert", "bosun/revert-23", "");
    merged(&fake, 40, "2026-11-06T10:00:00Z");
    fake.pull_set(25, "state", json!("closed"));
    fake.pull_set(25, "closed_at", json!("2026-11-07T10:00:00Z"));
    fake
}

#[test]
fn month_rows_and_totals() {
    let fake = seeded();
    let (code, v) = run(&fake, "session_a", NOW, &["budget", "report"]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["command"], "budget report");
    let d = &v["data"];
    assert_eq!(d["month"], "2026-11");
    let rows = d["rows"].as_array().unwrap();
    let prs: Vec<u64> = rows.iter().map(|r| r["pr"].as_u64().unwrap()).collect();
    assert_eq!(prs, [20, 21, 23, 25]);
    let by = |pr: u64| rows.iter().find(|r| r["pr"] == pr).unwrap();
    assert_eq!(by(20)["outcome"], "held");
    assert_eq!(by(20)["usd"], 2.0);
    assert_eq!(by(20)["issue"], 10);
    assert_eq!(by(20)["title"], "PR 20");
    assert_eq!(by(20)["sessions"], json!(["s1", "s2"]));
    assert_eq!(by(20)["merged_at"], "2026-11-03T10:00:00Z");
    assert!(by(20)["tokens"].is_null());
    assert_eq!(by(21)["outcome"], "open");
    assert!(by(21)["merged_at"].is_null());
    assert_eq!(by(23)["outcome"], "reverted");
    assert_eq!(by(25)["outcome"], "closed");
    let t = &d["totals"];
    assert_eq!(
        (t["prs"].as_u64(), t["merged"].as_u64()),
        (Some(4), Some(2))
    );
    assert_eq!(
        (t["held"].as_u64(), t["reverted"].as_u64()),
        (Some(1), Some(1))
    );
    assert_eq!(t["usd"], 6.5);
    assert_eq!(t["cost_per_merged_usd"], 3.25);
    assert_eq!(t["cost_per_held_usd"], 6.5);
    assert_eq!(t["top_prs"][0]["pr"], 21);
    for r in fake.requests() {
        assert_eq!(r.method, "GET", "read-only, saw {} {}", r.method, r.path);
    }
}

#[test]
fn month_flag_picks_another_month_and_bad_month_is_usage() {
    let fake = seeded();
    let args = ["budget", "report", "--month", "2026-10"];
    let (code, v) = run(&fake, "session_a", NOW, &args);
    assert_eq!(code, 0, "{v}");
    let rows = v["data"]["rows"].as_array().unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["pr"], 22);
    let args = ["budget", "report", "--month", "2026-13"];
    let (code, v) = run(&fake, "session_a", NOW, &args);
    assert_eq!(code, 2, "{v}");
    assert!(!v["error"]["fix"].as_str().unwrap().is_empty());
}

#[test]
fn out_writes_md_and_json_with_the_same_bytes_each_run() {
    let fake = seeded();
    let dir = std::env::temp_dir().join(format!("bosun-budget-report-{}", std::process::id()));
    let out = dir.join("reports");
    let args = ["budget", "report", "--out", out.to_str().unwrap()];
    let (code, v) = run(&fake, "session_a", NOW, &args);
    assert_eq!(code, 0, "{v}");
    let (md, js) = (out.join("2026-11.md"), out.join("2026-11.json"));
    let (md1, js1) = (std::fs::read(&md).unwrap(), std::fs::read(&js).unwrap());
    let file: Value = serde_json::from_slice(&js1).unwrap();
    assert_eq!(file["rows"], v["data"]["rows"]);
    assert_eq!(file["totals"], v["data"]["totals"]);
    let text = String::from_utf8(md1.clone()).unwrap();
    assert!(text.starts_with("# Budget report 2026-11\n"), "{text}");
    assert!(
        text.contains(
            "| #20 | PR 20 | #10 | s1, s2 | - | 2.00 | - | held | 2026-11-03T10:00:00Z |"
        ),
        "{text}"
    );
    // A later run at another time of day writes the same bytes.
    let (code, _) = run(&fake, "session_a", "2026-11-20T01:00:00Z", &args);
    assert_eq!(code, 0);
    assert_eq!(std::fs::read(&md).unwrap(), md1);
    assert_eq!(std::fs::read(&js).unwrap(), js1);
    std::fs::remove_dir_all(&dir).ok();
}
