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
    fake.enforce_checks(&ALL);
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

fn assert_not_enabled(fake: &FakeGithub) {
    assert_eq!(fake.auto_merge_enabled(20), None);
    assert_eq!(merge_calls(fake), 0);
    let labels = fake.labels_of(7);
    assert!(labels.contains(&"bosun:state/review".into()), "{labels:?}");
}

#[test]
fn a_required_check_the_ruleset_does_not_enforce_needs_the_owner() {
    let (fake, dir) = setup(&ALL);
    fake.enforce_checks(&["ci-ok", "risk-check", "main-open"]);
    let (code, v) = run(&fake, dir.path());
    assert_eq!(code, 5, "{v}");
    assert_eq!(v["data"]["unenforced"], json!(["review-verdict"]), "{v}");
    let msg = v["error"]["message"].as_str().unwrap();
    assert!(msg.contains("review-verdict"), "{msg}");
    assert!(v["error"]["fix"].as_str().unwrap().contains("hoist apply"));
    assert_not_enabled(&fake);
}

#[test]
fn a_branch_without_rules_never_gets_auto_merge() {
    let (fake, dir) = setup(&ALL);
    fake.enforce_checks(&[]);
    let (code, v) = run(&fake, dir.path());
    assert_eq!(code, 5, "{v}");
    assert_eq!(v["data"]["unenforced"].as_array().unwrap().len(), 4, "{v}");
    assert_not_enabled(&fake);
}

#[test]
fn classic_protection_counts_as_enforcement() {
    let (fake, dir) = setup(&ALL);
    fake.enforce_checks(&["ci-ok"]);
    fake.enforce_classic(&["risk-check", "review-verdict", "main-open"]);
    let (code, v) = run(&fake, dir.path());
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["enforced_checks"], "verified");
    assert_eq!(fake.auto_merge_enabled(20).as_deref(), Some("SQUASH"));
}

#[test]
fn unreadable_rules_are_unknown_and_do_not_block() {
    let (fake, dir) = setup(&ALL);
    fake.enforce_checks(&[]);
    fake.fail_path("/rules/branches/main", 403);
    fake.fail_path("/protection/required_status_checks", 403);
    let (code, v) = run(&fake, dir.path());
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["enforced_checks"], "unknown");
    assert_eq!(fake.auto_merge_enabled(20).as_deref(), Some("SQUASH"));
}

#[test]
fn partly_readable_rules_are_unknown_and_do_not_block() {
    let (fake, dir) = setup(&ALL);
    fake.enforce_checks(&["ci-ok"]);
    fake.fail_path("/protection/required_status_checks", 403);
    let (code, v) = run(&fake, dir.path());
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["enforced_checks"], "unknown");
    assert_eq!(fake.auto_merge_enabled(20).as_deref(), Some("SQUASH"));
}

#[test]
fn an_unstable_answer_names_the_failing_checks() {
    let (fake, dir) = setup(&ALL);
    fake.graphql_error("Pull request is in unstable status");
    let (code, v) = run(&fake, dir.path());
    assert_eq!(code, 1, "{v}");
    let fix = v["error"]["fix"].as_str().unwrap();
    assert!(fix.contains("bosun wait --pr 20"), "{fix}");
    assert!(fix.contains("failing"), "{fix}");
    assert_not_enabled(&fake);
}
