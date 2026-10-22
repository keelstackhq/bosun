mod common;

use std::path::Path;
use std::process::Command;

use common::fake::FakeGithub;
use common::{bosun, one_object};
use serde_json::Value;

/// A git repo on `branch` with one commit.
fn repo(branch: &str) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    for args in [
        vec!["init", "-q", "-b", branch],
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

fn bosun_in(fake: &FakeGithub, dir: &Path, args: &[&str]) -> (i32, Value) {
    let out = bosun()
        .env("BOSUN_GITHUB_API", fake.url())
        .arg("-C")
        .arg(dir)
        .args(args)
        .output()
        .unwrap();
    (out.status.code().unwrap(), one_object(&out.stdout))
}

fn task(fake: &FakeGithub, state: &str) {
    fake.issue(7, "the task", &[state], "Do it.");
}

fn merge_calls(fake: &FakeGithub) -> usize {
    fake.requests()
        .iter()
        .filter(|r| r.path.ends_with("/merge") || r.method == "PUT")
        .count()
}

#[test]
fn creates_the_pr_once_then_reuses_it() {
    let fake = FakeGithub::start();
    task(&fake, "bosun:state/building");
    let dir = repo("claude/t7");
    let (code, v) = bosun_in(&fake, dir.path(), &["pr", "7"]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["created"], true);
    let number = v["data"]["pr"].as_u64().unwrap();
    let (code, v2) = bosun_in(&fake, dir.path(), &["pr", "7", "--title", "ignored"]);
    assert_eq!(code, 0, "{v2}");
    assert_eq!(v2["data"]["created"], false);
    assert_eq!(v2["data"]["pr"], number);
    let creates = fake
        .requests()
        .iter()
        .filter(|r| r.method == "POST" && r.path.ends_with("/pulls"))
        .count();
    assert_eq!(creates, 1);
    let body = fake.pull_body(number);
    assert!(body.starts_with("Closes #7"));
    assert!(body.contains("<!-- bosun:task-pr 7 -->"));
    assert!(fake.labels_of(7).contains(&"bosun:state/review".into()));
}

#[test]
fn refuses_the_main_branch() {
    let fake = FakeGithub::start();
    task(&fake, "bosun:state/building");
    let dir = repo("main");
    let (code, v) = bosun_in(&fake, dir.path(), &["pr", "7"]);
    assert_eq!(code, 2, "{v}");
}

#[test]
fn auto_merge_is_refused_while_review_verdict_is_pending() {
    let fake = FakeGithub::start();
    task(&fake, "bosun:state/review");
    fake.pull(20, "t", "claude/t7", "Closes #7");
    fake.status("sha20", "ci-ok", "success", "");
    fake.status("sha20", "risk-check", "success", "");
    fake.status("sha20", "main-open", "success", "");
    fake.status("sha20", "review-verdict", "pending", "");
    let dir = repo("claude/t7");
    let (code, v) = bosun_in(&fake, dir.path(), &["pr", "7", "--auto-merge"]);
    assert_eq!(code, 1, "{v}");
    let msg = v["error"]["message"].as_str().unwrap();
    assert!(msg.contains("review-verdict: pending"), "{msg}");
    assert!(
        v["error"]["fix"]
            .as_str()
            .unwrap()
            .contains("bosun wait --pr 20")
    );
    assert_eq!(fake.auto_merge_enabled(20), None);
    assert!(fake.labels_of(7).contains(&"bosun:state/review".into()));
}

#[test]
fn auto_merge_is_enabled_when_all_four_are_green() {
    let fake = FakeGithub::start();
    task(&fake, "bosun:state/review");
    fake.pull(20, "t", "claude/t7", "Closes #7");
    fake.status("sha20", "risk-check", "success", "");
    fake.status("sha20", "main-open", "success", "");
    fake.status(
        "sha20",
        "review-verdict",
        "success",
        "approve 0.9, 0 blocker, 0 major",
    );
    fake.check_run("sha20", "ci-ok", "completed", Some("success"));
    let dir = repo("claude/t7");
    let (code, v) = bosun_in(&fake, dir.path(), &["pr", "7", "--auto-merge"]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["auto_merge"], true);
    assert_eq!(fake.auto_merge_enabled(20).as_deref(), Some("SQUASH"));
    assert!(fake.labels_of(7).contains(&"bosun:state/queued".into()));
    assert!(
        fake.requests()
            .iter()
            .any(|r| r.method == "POST" && r.path.ends_with("/graphql"))
    );
    assert_eq!(merge_calls(&fake), 0, "bosun must never merge");
}

#[test]
fn a_missing_check_is_listed() {
    let fake = FakeGithub::start();
    task(&fake, "bosun:state/review");
    fake.pull(20, "t", "claude/t7", "Closes #7");
    let dir = repo("claude/t7");
    let (code, v) = bosun_in(&fake, dir.path(), &["pr", "7", "--auto-merge"]);
    assert_eq!(code, 1);
    assert_eq!(v["data"]["not_green"].as_array().unwrap().len(), 4);
    assert_eq!(v["data"]["not_green"][0]["state"], "missing");
}

#[test]
fn auto_merge_respects_the_kill_switch() {
    let fake = FakeGithub::start();
    task(&fake, "bosun:state/review");
    fake.variable("BOSUN_HALT", "1");
    let dir = repo("claude/t7");
    let (code, _) = bosun_in(&fake, dir.path(), &["pr", "7", "--auto-merge"]);
    assert_eq!(code, 4);
}

fn graphql_calls(fake: &FakeGithub) -> usize {
    fake.requests()
        .iter()
        .filter(|r| r.path.ends_with("/graphql"))
        .count()
}

#[test]
fn auto_merge_is_refused_unless_the_task_is_in_review() {
    for state in ["needs-human", "blocked", "queued"] {
        let fake = FakeGithub::start();
        task(&fake, &format!("bosun:state/{state}"));
        fake.pull(20, "t", "claude/t7", "Closes #7");
        for c in ["ci-ok", "risk-check", "review-verdict", "main-open"] {
            fake.status("sha20", c, "success", "");
        }
        let dir = repo("claude/t7");
        let (code, v) = bosun_in(&fake, dir.path(), &["pr", "7", "--auto-merge"]);
        assert_eq!(code, 3, "{state}: {v}");
        assert!(!v["error"]["fix"].as_str().unwrap().is_empty());
        assert_eq!(graphql_calls(&fake), 0, "{state}");
        assert_eq!(fake.auto_merge_enabled(20), None);
    }
}

#[test]
fn plain_pr_respects_the_kill_switch() {
    let fake = FakeGithub::start();
    task(&fake, "bosun:state/building");
    fake.variable("BOSUN_HALT", "1");
    let dir = repo("claude/t7");
    let (code, _) = bosun_in(&fake, dir.path(), &["pr", "7"]);
    assert_eq!(code, 4);
    assert!(
        !fake
            .requests()
            .iter()
            .any(|r| r.method == "POST" && r.path.ends_with("/pulls"))
    );
}

#[test]
fn the_pr_base_is_the_repos_default_branch() {
    let fake = FakeGithub::start();
    fake.repo_set("default_branch", serde_json::json!("trunk"));
    task(&fake, "bosun:state/building");
    let dir = repo("claude/t7");
    let (code, v) = bosun_in(&fake, dir.path(), &["pr", "7"]);
    assert_eq!(code, 0, "{v}");
    let body = fake
        .requests()
        .into_iter()
        .find(|r| r.method == "POST" && r.path.ends_with("/pulls"))
        .and_then(|r| r.body)
        .unwrap();
    assert!(body.contains(r#""base":"trunk""#), "{body}");
}

#[test]
fn the_default_branch_is_refused_whatever_its_name() {
    let fake = FakeGithub::start();
    fake.repo_set("default_branch", serde_json::json!("trunk"));
    task(&fake, "bosun:state/building");
    let dir = repo("trunk");
    let (code, _) = bosun_in(&fake, dir.path(), &["pr", "7"]);
    assert_eq!(code, 2);
}
