//! D-016: a verdict for a PR with several review chunks must list every chunk it reviewed.

mod common;

use common::fake::FakeGithub;
use common::run;
use serde_json::Value;

const T0: &str = "2026-10-22T10:00:00Z";

fn setup(files: usize) -> FakeGithub {
    let fake = FakeGithub::start();
    fake.issue(7, "the task", &["bosun:state/review"], "Do it.");
    fake.pull_as_issue(42, "the pr");
    fake.pull(42, "the pr", "claude/x", "Closes #7");
    let big = "x".repeat(30_000);
    for k in 0..files {
        fake.pull_file(42, &format!("src/f{k}.rs"), "added", 5, 0, Some(&big));
    }
    fake
}

fn verdict(chunks: &str) -> String {
    format!(
        r#"{{"pr":42,"head_sha":"sha42","verdict":"approve","confidence":0.95,"chunks_reviewed":{chunks}}}"#
    )
}

fn call(fake: &FakeGithub, sub: &str, json: &str) -> (i32, Value) {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("v.json");
    std::fs::write(&file, json).unwrap();
    run(
        fake,
        "s",
        T0,
        &["verdict", sub, "--from", file.to_str().unwrap()],
    )
}

fn nothing_written(fake: &FakeGithub) {
    assert!(fake.statuses_of("sha42").is_empty());
    assert!(fake.comments_of(42).is_empty());
}

#[test]
fn a_missing_chunk_refuses_post_and_comment_and_writes_nothing() {
    let fake = setup(3);
    for sub in ["post", "comment"] {
        let (code, v) = call(&fake, sub, &verdict("[1,3]"));
        assert_eq!(code, 1, "{sub}: {v}");
        assert_eq!(
            v["error"]["message"],
            "verdict lists chunks [1, 3] of 3; missing [2]"
        );
        assert_eq!(
            v["error"]["fix"],
            "review every chunk of `bosun review-pack <pr>` and list each number in `chunks_reviewed`"
        );
        nothing_written(&fake);
    }
    // Absent entirely: every chunk is missing.
    let (code, v) = call(
        &fake,
        "post",
        r#"{"pr":42,"head_sha":"sha42","verdict":"approve","confidence":0.95}"#,
    );
    assert_eq!(code, 1, "{v}");
    assert!(
        v["error"]["message"]
            .as_str()
            .unwrap()
            .contains("missing [1, 2, 3]")
    );
}

#[test]
fn every_chunk_listed_posts() {
    let fake = setup(3);
    let (code, v) = call(&fake, "post", &verdict("[3,1,2]"));
    assert_eq!(code, 0, "{v}");
    assert_eq!(fake.statuses_of("sha42").len(), 1);
}

#[test]
fn one_chunk_needs_no_list() {
    let fake = setup(1);
    let (code, v) = call(
        &fake,
        "post",
        r#"{"pr":42,"head_sha":"sha42","verdict":"approve","confidence":0.95}"#,
    );
    assert_eq!(code, 0, "{v}");
}
