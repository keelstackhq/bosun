//! Use cases of M5 (spec §12): the learning loop.
mod common;

use common::fake::FakeGithub;
use common::{bosun, one_object, run};

fn capture(fake: &FakeGithub, text: &str) -> serde_json::Value {
    let out = bosun()
        .env("BOSUN_GITHUB_API", fake.url())
        .args(["lesson", "capture", "--kind", "stop-rule", "--issue", "7"])
        .write_stdin(text.to_string())
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(0));
    one_object(&out.stdout)
}

/// UC-M5-1: a stop rule opens a lesson; the same signature again adds a hit, not a new issue.
#[test]
fn uc_m5_1() {
    let fake = FakeGithub::start();
    let first = capture(&fake, "stop rule: no progress\nsha 3f9c2ab81d04\n");
    let second = capture(&fake, "stop rule: no progress\nsha 77aa88bb99cc\n");
    assert_eq!(first["data"]["created"], true);
    assert_eq!(second["data"]["created"], false);
    assert_eq!(second["data"]["issue"], first["data"]["issue"]);
    let n = first["data"]["issue"].as_u64().unwrap();
    let seen: Vec<_> = fake
        .comments_of(n)
        .into_iter()
        .filter(|c| c.body.contains(r#""bosun":"lesson-seen""#))
        .collect();
    assert_eq!(seen.len(), 1);
}

/// UC-M5-2: a blocking lesson sets process-red, and waves pause.
#[test]
fn uc_m5_2() {
    let now = "2026-10-23T10:00:00Z";
    let fake = FakeGithub::start();
    fake.issue(10, "Epic", &["bosun:state/claimed"], "");
    fake.issue(
        11,
        "task",
        &["bosun:state/ready"],
        r#"<!-- bosun:task {"item":"M5","task":"T1","epic":10,"touch":["src/a.rs"]} -->"#,
    );
    fake.issue(
        20,
        "lesson",
        &["bosun:lesson", "bosun:lesson/blocking", "bosun:process-red"],
        "",
    );
    let (code, v) = run(&fake, "session_a", now, &["wave", "plan", "10"]);
    assert_eq!(code, 4, "{v}");
    let (code, v) = run(&fake, "session_a", now, &["status"]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["process_red"], 20);
}
