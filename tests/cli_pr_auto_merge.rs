//! `bosun pr --auto-merge` turns auto-merge on at once; the ruleset gates the merge. A PR that is
//! already clean needs the owner, and bosun never merges it.

mod common;

use std::path::Path;
use std::process::Command;

use common::fake::FakeGithub;
use common::{bosun, one_object};
use serde_json::{Value, json};

const CCR_MSG: &str = "403 GraphQL is not available from Claude Code sessions";

fn repo() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    for args in [
        vec!["init", "-q", "-b", "claude/t7"],
        vec![
            "-c",
            "user.name=t",
            "-c",
            "user.email=t@t",
            "commit",
            "-q",
            "--allow-empty",
            "-m",
            "x",
        ],
    ] {
        let st = Command::new("git")
            .arg("-C")
            .arg(dir.path())
            .args(&args)
            .status()
            .unwrap();
        assert!(st.success());
    }
    dir
}

fn run(fake: &FakeGithub, dir: &Path) -> (i32, Value) {
    let out = bosun()
        .env("BOSUN_GITHUB_API", fake.url())
        .arg("-C")
        .arg(dir)
        .args(["pr", "7", "--auto-merge"])
        .output()
        .unwrap();
    (out.status.code().unwrap(), one_object(&out.stdout))
}

fn setup(green: &[&str]) -> (FakeGithub, tempfile::TempDir) {
    let fake = FakeGithub::start();
    fake.issue(7, "the task", &["bosun:state/review"], "Do it.");
    fake.pull(20, "t", "claude/t7", "Closes #7");
    for c in green {
        fake.status("sha20", c, "success", "");
    }
    (fake, repo())
}

const ALL: [&str; 4] = ["ci-ok", "risk-check", "review-verdict", "main-open"];

fn merge_calls(fake: &FakeGithub) -> usize {
    fake.requests()
        .iter()
        .filter(|r| r.path.ends_with("/merge"))
        .count()
}

#[test]
fn pending_checks_do_not_stop_auto_merge() {
    let (fake, dir) = setup(&["risk-check"]);
    fake.status("sha20", "ci-ok", "pending", "");
    let (code, v) = run(&fake, dir.path());
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["task_state"], "queued");
    assert_eq!(fake.auto_merge_enabled(20).as_deref(), Some("SQUASH"));
    let pending = v["data"]["pending_checks"].as_array().unwrap();
    assert!(
        pending.contains(&json!({"context": "ci-ok", "state": "pending"})),
        "{v}"
    );
    assert!(
        pending.contains(&json!({"context": "main-open", "state": "missing"})),
        "{v}"
    );
    assert!(fake.labels_of(7).contains(&"bosun:state/queued".into()));
    assert_eq!(merge_calls(&fake), 0);
}

#[test]
fn all_green_but_not_yet_clean_is_enabled() {
    let (fake, dir) = setup(&ALL);
    let (code, v) = run(&fake, dir.path());
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["pending_checks"], json!([]));
    assert_eq!(fake.auto_merge_enabled(20).as_deref(), Some("SQUASH"));
    assert!(fake.labels_of(7).contains(&"bosun:state/queued".into()));
}

fn assert_clean_refusal(fake: &FakeGithub, code: i32, v: &Value) {
    assert_eq!(code, 5, "{v}");
    assert_eq!(v["data"]["already_clean"], true, "{v}");
    assert_eq!(v["data"]["pr"], 20);
    assert_eq!(v["data"]["head_sha"], "sha20");
    let msg = v["error"]["message"].as_str().unwrap();
    assert!(msg.contains("PR #20 is already mergeable"), "{msg}");
    assert!(!v["error"]["fix"].as_str().unwrap().is_empty());
    assert_eq!(fake.auto_merge_enabled(20), None);
    assert_eq!(merge_calls(fake), 0);
    let labels = fake.labels_of(7);
    assert!(labels.contains(&"bosun:state/review".into()), "{labels:?}");
    assert!(!labels.contains(&"bosun:state/queued".into()));
}

#[test]
fn a_graphql_clean_answer_needs_the_owner_and_merges_nothing() {
    let (fake, dir) = setup(&ALL);
    fake.already_clean();
    let (code, v) = run(&fake, dir.path());
    assert_clean_refusal(&fake, code, &v);
}

#[test]
fn a_ccr_clean_answer_needs_the_owner_and_merges_nothing() {
    let (fake, dir) = setup(&ALL);
    fake.graphql_status(403, CCR_MSG);
    fake.already_clean();
    let (code, v) = run(&fake, dir.path());
    assert_clean_refusal(&fake, code, &v);
    assert!(
        fake.requests()
            .iter()
            .any(|r| r.method == "PUT" && r.path.ends_with("/pulls/20/ccr/auto_merge"))
    );
}
