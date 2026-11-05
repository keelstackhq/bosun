//! The pack groups the files into chunks of patch bytes, one per reviewer subagent.

mod common;

use common::fake::FakeGithub;
use common::run;
use serde_json::json;

const T0: &str = "2026-10-22T10:00:00Z";

fn setup() -> FakeGithub {
    let fake = FakeGithub::start();
    fake.issue(7, "the task", &["bosun:state/review"], "Do it.");
    fake.pull_as_issue(42, "the pr");
    fake.pull(42, "the pr", "claude/x", "Closes #7");
    fake
}

#[test]
fn big_patches_make_one_chunk_each() {
    let fake = setup();
    let big = "x".repeat(30_000);
    for f in ["src/a.rs", "src/b.rs", "src/c.rs"] {
        fake.pull_file(42, f, "added", 5, 0, Some(&big));
    }
    let (code, v) = run(&fake, "s", T0, &["review-pack", "42"]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(
        v["data"]["chunks"],
        json!([["src/a.rs"], ["src/b.rs"], ["src/c.rs"]])
    );
    let text = v["data"]["instructions"].as_str().unwrap();
    assert!(text.contains("own reviewer subagent"));
}

#[test]
fn a_small_pr_is_one_chunk() {
    let fake = setup();
    fake.pull_file(42, "src/a.rs", "added", 5, 0, Some("@@ a"));
    fake.pull_file(42, "bin.png", "added", 0, 0, None);
    let (code, v) = run(&fake, "s", T0, &["review-pack", "42"]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["chunks"], json!([["src/a.rs", "bin.png"]]));
}
