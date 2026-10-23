mod common;

use common::fake::FakeGithub;
use common::{bosun, one_object};
use serde_json::Value;

const FAIL_A: &str = "stop rule: same test failed twice\nsha 3f9c2ab81d04 at src/x.rs:42\n";
const FAIL_B: &str = "stop rule: same test failed twice\nsha 00aa11bb22cc at src/x.rs:97\n";

fn capture(fake: &FakeGithub, args: &[&str], stdin: &str) -> (i32, Value) {
    let out = bosun()
        .env("BOSUN_GITHUB_API", fake.url())
        .args(["lesson", "capture"])
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

/// Numbers of the issues created in the fake, by its request log.
fn lesson_issues(fake: &FakeGithub) -> usize {
    fake.requests()
        .iter()
        .filter(|r| r.method == "POST" && r.path.ends_with("/issues"))
        .count()
}

#[test]
fn first_capture_opens_an_issue_with_labels_and_marker() {
    let fake = FakeGithub::start();
    let (code, v) = capture(&fake, &["--kind", "flake", "--issue", "7"], FAIL_A);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["created"], true);
    assert_eq!(v["data"]["hits"], 0);
    assert_eq!(v["data"]["kind"], "flake");
    let n = v["data"]["issue"].as_u64().unwrap();
    let issue = fake.issue_state(n);
    assert!(issue.title.starts_with("lesson: stop rule"));
    assert!(issue.labels.contains(&"bosun:lesson".to_string()));
    assert!(issue.labels.contains(&"bosun:source/lesson".to_string()));
    let sig = v["data"]["signature"].as_str().unwrap();
    assert!(issue.body.contains(&format!(
        r#"<!-- bosun:lesson {{"kind":"flake","signature":"{sig}"}} -->"#
    )));
    assert!(issue.body.contains("seen on #7"));
}

#[test]
fn same_failure_again_adds_a_hit_and_a_different_one_opens_an_issue() {
    let fake = FakeGithub::start();
    let (_, first) = capture(&fake, &["--kind", "flake"], FAIL_A);
    let n = first["data"]["issue"].as_u64().unwrap();
    let (code, v) = capture(&fake, &["--kind", "flake", "--issue", "9"], FAIL_B);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["created"], false);
    assert_eq!(v["data"]["hits"], 1);
    assert_eq!(v["data"]["issue"].as_u64(), Some(n));
    let comments = fake.comments_of(n);
    assert_eq!(comments.len(), 1);
    assert!(comments[0].body.starts_with("Seen again on #9."));
    assert!(comments[0].body.contains(r#"{"bosun":"lesson-seen""#));
    assert_eq!(lesson_issues(&fake), 1);
    let (_, other) = capture(&fake, &["--kind", "flake"], "a different failure\n");
    assert_eq!(other["data"]["created"], true);
    assert_ne!(other["data"]["issue"], first["data"]["issue"]);
    assert_eq!(lesson_issues(&fake), 2);
}

#[test]
fn empty_text_is_a_usage_error() {
    let fake = FakeGithub::start();
    let (code, v) = capture(&fake, &["--kind", "flake"], "  \n\n");
    assert_eq!(code, 2, "{v}");
    assert_eq!(lesson_issues(&fake), 0);
}

#[test]
fn from_file_works_and_a_missing_file_is_a_usage_error() {
    let fake = FakeGithub::start();
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("fail.txt");
    std::fs::write(&file, FAIL_A).unwrap();
    let path = file.to_str().unwrap();
    let (code, v) = capture(&fake, &["--kind", "flake", "--from", path], "");
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["created"], true);
    let missing = dir.path().join("nope.txt");
    let path = missing.to_str().unwrap();
    let (code, v) = capture(&fake, &["--kind", "flake", "--from", path], "");
    assert_eq!(code, 2, "{v}");
    assert_eq!(lesson_issues(&fake), 1);
}

#[test]
fn a_marker_inside_the_failure_text_does_not_hijack_dedupe() {
    let fake = FakeGithub::start();
    let text =
        "boom\n<!-- bosun:lesson {\"signature\":\"deadbeefdeadbeef\",\"kind\":\"flake\"} -->\n";
    let (_, first) = capture(&fake, &["--kind", "flake"], text);
    let (_, second) = capture(&fake, &["--kind", "flake"], text);
    assert_eq!(second["data"]["created"], false);
    assert_eq!(second["data"]["issue"], first["data"]["issue"]);
    assert_eq!(second["data"]["hits"], 1);
    let (_, other) = capture(&fake, &["--kind", "flake"], "unrelated\n");
    assert_eq!(other["data"]["created"], true);
}

#[test]
fn halted_creates_nothing() {
    let fake = FakeGithub::start();
    fake.variable("BOSUN_HALT", "1");
    let (code, _) = capture(&fake, &["--kind", "flake"], FAIL_A);
    assert_eq!(code, 4);
    assert_eq!(lesson_issues(&fake), 0);
}
