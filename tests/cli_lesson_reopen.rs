//! Reopen on a hit, `lesson add --upstream` and `progress --lessons` (spec §6, UC-M5-5).
mod common;

use common::fake::FakeGithub;
use common::{bosun, one_object, run};
use serde_json::Value;

const FAIL: &str = "stop rule: same test failed twice\nsha 3f9c2ab81d04 at src/x.rs:42\n";
const FAIL_AGAIN: &str = "stop rule: same test failed twice\nsha 00aa11bb22cc at src/x.rs:97\n";

fn capture(fake: &FakeGithub, text: &str, issue: Option<&str>) -> Value {
    let mut args = vec!["lesson", "capture", "--kind", "stop-rule"];
    if let Some(i) = issue {
        args.extend(["--issue", i]);
    }
    let out = bosun()
        .env("BOSUN_GITHUB_API", fake.url())
        .args(args)
        .write_stdin(text.to_string())
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(0), "{:?}", out);
    one_object(&out.stdout)["data"].clone()
}

fn closed_lesson(reason: &str) -> (FakeGithub, u64) {
    let fake = FakeGithub::start();
    let n = capture(&fake, FAIL, None)["issue"].as_u64().unwrap();
    fake.close_issue_as(n, "2026-10-20T09:00:00Z", reason);
    (fake, n)
}

#[test]
fn a_hit_on_a_completed_lesson_reopens_it_as_blocking() {
    let (fake, n) = closed_lesson("completed");
    let d = capture(&fake, FAIL_AGAIN, Some("7"));
    assert_eq!(d["issue"].as_u64(), Some(n));
    assert_eq!(d["created"], false);
    assert_eq!(d["reopened"], true);
    assert_eq!(d["hits"], 1);
    let i = fake.issue_state(n);
    assert_eq!(i.state, "open");
    for l in ["bosun:lesson/blocking", "bosun:process-red"] {
        assert!(i.labels.contains(&l.to_string()), "{l} in {:?}", i.labels);
    }
    let c = fake.comments_of(n);
    assert_eq!(c.len(), 1);
    assert!(c[0].body.starts_with(
        "The mechanism did not hold: seen again on #7. Reopened as blocking (spec §6)."
    ));
    assert!(c[0].body.contains(r#"{"bosun":"lesson-seen""#));
}

#[test]
fn an_open_match_is_preferred_over_a_closed_one() {
    let (fake, closed) = closed_lesson("completed");
    // A second lesson with the same marker, open.
    let body = fake.issue_state(closed).body;
    fake.issue(50, "lesson: dup", &["bosun:lesson"], &body);
    let d = capture(&fake, FAIL_AGAIN, None);
    assert_eq!(d["issue"], 50);
    assert_eq!(d["reopened"], false);
    assert_eq!(fake.issue_state(closed).state, "closed");
}

#[test]
fn not_planned_and_advice_lessons_are_ignored() {
    let (fake, n) = closed_lesson("not_planned");
    let d = capture(&fake, FAIL_AGAIN, None);
    assert_eq!(d["created"], true);
    assert_eq!(d["reopened"], false);
    assert_ne!(d["issue"].as_u64(), Some(n));
    assert_eq!(fake.issue_state(n).state, "closed");

    let (fake, n) = closed_lesson("completed");
    let body = fake.issue_state(n).body;
    fake.issue(
        60,
        "lesson: advice",
        &["bosun:lesson", "bosun:lesson/advice"],
        &body,
    );
    fake.close_issue_as(60, "2026-10-20T09:00:00Z", "completed");
    fake.close_issue_as(n, "2026-10-20T09:00:00Z", "not_planned");
    let d = capture(&fake, FAIL_AGAIN, None);
    assert_eq!(d["created"], true);
    assert_eq!(d["reopened"], false);
    assert_eq!(fake.issue_state(60).state, "closed");
}

#[test]
fn upstream_files_the_lesson_on_keelstackhq_bosun() {
    let fake = FakeGithub::start();
    let tmp = tempfile::tempdir().unwrap();
    let out = bosun()
        .arg("-C")
        .arg(tmp.path())
        .env("BOSUN_GITHUB_API", fake.url())
        .args([
            "lesson",
            "add",
            "--upstream",
            "bosun wave plan hides epics",
            "--why",
            "lost an hour",
        ])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(0), "{:?}", out);
    let v = one_object(&out.stdout);
    assert_eq!(v["data"]["upstream"], true);
    let n = v["data"]["issue"].as_u64().unwrap();
    let i = fake.issue_state(n);
    assert_eq!(i.title, "lesson: bosun wave plan hides epics");
    for l in ["bosun:lesson", "bosun:source/lesson"] {
        assert!(i.labels.contains(&l.to_string()));
    }
    assert!(i.body.contains("bosun wave plan hides epics"));
    assert!(i.body.contains("lost an hour"));
    assert!(i.body.contains("Reported from acme/widget by session_test"));
    assert!(
        fake.requests()
            .iter()
            .any(|r| r.method == "POST" && r.path == "/repos/keelstackhq/bosun/issues")
    );
    assert!(!tmp.path().join("docs/lessons.toml").exists());
}

#[test]
fn progress_lessons_counts_open_blocking_mechanized_and_hits() {
    let fake = FakeGithub::start();
    let seen =
        |sig: &str| format!(r#"{{"bosun":"lesson-seen","session":"s","signature":"{sig}"}}"#);
    let hit = r#"{"bosun":"lesson-hit","session":"s","id":"L-001"}"#;
    fake.issue(
        1,
        "lesson: a",
        &["bosun:lesson", "bosun:lesson/blocking"],
        "",
    );
    fake.issue(2, "lesson: b", &["bosun:lesson"], "");
    fake.issue(3, "lesson: c", &["bosun:lesson"], "");
    fake.close_issue_as(3, "2026-10-01T00:00:00Z", "completed");
    fake.issue(4, "lesson: d", &["bosun:lesson", "bosun:lesson/advice"], "");
    fake.close_issue_as(4, "2026-10-01T00:00:00Z", "completed");
    fake.issue(5, "lesson: e", &["bosun:lesson"], "");
    fake.close_issue_as(5, "2026-10-01T00:00:00Z", "not_planned");
    for (issue, body, at) in [
        (1, seen("aa"), "2026-10-22T10:00:00Z"),
        (2, hit.to_string(), "2026-10-20T10:00:00Z"),
        (3, seen("bb"), "2026-10-10T10:00:00Z"),
        (3, seen("bb"), "2026-09-01T10:00:00Z"),
    ] {
        fake.comment(issue, &body);
        fake.last_comment(|c| c.created_at = at.into());
    }
    fake.comment(2, "just a note");
    let (code, v) = run(
        &fake,
        "s",
        "2026-10-23T12:00:00Z",
        &["progress", "--lessons"],
    );
    assert_eq!(code, 0, "{v}");
    let l = &v["data"]["lessons"];
    assert_eq!(l["open"], 2);
    assert_eq!(l["blocking"], 1);
    assert_eq!(l["mechanized"], 1);
    assert_eq!(l["hits_last_7_days"], 2);
    let weeks = l["hits_by_week"].as_array().unwrap();
    let counts: Vec<u64> = weeks.iter().map(|w| w["hits"].as_u64().unwrap()).collect();
    assert_eq!(counts, [0, 0, 1, 2]);
    assert_eq!(weeks[3]["week_start"], "2026-10-16T12:00:00Z");
}
