//! `bosun pr --pr <n>`: the draft-to-auto-merge flow for a PR that has no task (a governance PR).
mod common;

use std::path::Path;
use std::process::Command;

use common::fake::FakeGithub;
use common::stamps;
use common::{bosun, one_object};
use serde_json::{Value, json};

const ALL: [&str; 4] = ["ci-ok", "risk-check", "review-verdict", "main-open"];

fn repo() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    for args in [
        vec!["init", "-q", "-b", "bosun/migrate-steady"],
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

fn pr(fake: &FakeGithub, dir: &Path, args: &[&str]) -> (i32, Value) {
    let out = bosun()
        .env("BOSUN_GITHUB_API", fake.url())
        .arg("-C")
        .arg(dir)
        .arg("pr")
        .args(args)
        .output()
        .unwrap();
    let v = one_object(&out.stdout);
    if v["ok"] == false {
        assert!(!v["error"]["fix"].as_str().unwrap().trim().is_empty());
    }
    (out.status.code().unwrap(), v)
}

fn setup() -> (FakeGithub, tempfile::TempDir) {
    let fake = FakeGithub::start();
    fake.pull(
        30,
        "chore: bosun mode gatekeeper -> steady",
        "bosun/migrate-steady",
        "no task",
    );
    fake.pull_set(30, "draft", json!(true));
    fake.enforce_checks(&ALL);
    (fake, repo())
}

#[test]
fn without_ready_it_reports_the_pr_and_changes_nothing() {
    let (fake, dir) = setup();
    let (code, v) = pr(&fake, dir.path(), &["--pr", "30"]);
    assert_eq!(code, 0, "{v}");
    let d = &v["data"];
    assert_eq!(
        d["pr"],
        json!({"number": 30, "head_sha": "sha30", "draft": true})
    );
    assert_eq!(d["task"], Value::Null);
    assert_eq!(d["ready"], false);
    assert!(fake.pull_is_draft(30));
    assert!(fake.requests().iter().all(|r| r.method == "GET"));
}

#[test]
fn a_pr_that_closes_a_task_is_refused_with_the_task_command() {
    let (fake, dir) = setup();
    fake.pull(31, "feat: x", "claude/t7", "Closes #7");
    let (code, v) = pr(&fake, dir.path(), &["--pr", "31"]);
    assert_eq!(code, 3, "{v}");
    assert!(v["error"]["fix"].as_str().unwrap().contains("bosun pr 7"));
    fake.pull(32, "feat: y", "claude/t8", "<!-- bosun:task-pr 8 -->");
    let (code, v) = pr(&fake, dir.path(), &["--pr", "32"]);
    assert_eq!(code, 3, "{v}");
    assert!(v["error"]["fix"].as_str().unwrap().contains("bosun pr 8"));
}

#[test]
fn a_missing_closed_or_fork_pr_is_refused() {
    let (fake, dir) = setup();
    let (code, v) = pr(&fake, dir.path(), &["--pr", "99"]);
    assert_eq!(code, 1, "{v}");
    fake.pull_set(30, "state", json!("closed"));
    let (code, v) = pr(&fake, dir.path(), &["--pr", "30"]);
    assert_eq!(code, 3, "{v}");
    fake.pull_from_fork(40, "bosun/migrate-steady", "mallory/widget");
    let (code, v) = pr(&fake, dir.path(), &["--pr", "40"]);
    assert_eq!(code, 3, "{v}");
}

#[test]
fn a_task_or_a_pr_number_is_required_and_they_exclude_each_other() {
    let (fake, dir) = setup();
    let (code, v) = pr(&fake, dir.path(), &[]);
    assert_eq!(code, 2, "{v}");
    let (code, _) = pr(&fake, dir.path(), &["7", "--pr", "30"]);
    assert_eq!(code, 2);
    let (code, _) = pr(&fake, dir.path(), &["--pr", "30", "--bundle", "8"]);
    assert_eq!(code, 2);
}

#[test]
fn ready_with_stamps_marks_ready_and_turns_on_auto_merge() {
    let (fake, dir) = setup();
    stamps::ready(&fake, dir.path(), 30);
    let (code, v) = pr(&fake, dir.path(), &["--pr", "30", "--ready"]);
    assert_eq!(code, 0, "{v}");
    let d = &v["data"];
    assert_eq!(d["ready"], true);
    assert_eq!(d["pr"]["draft"], false);
    assert_eq!(d["task"], Value::Null);
    assert_eq!(d["enforced_checks"], "verified");
    assert!(d["auto_merge_route"].is_string(), "{d}");
    assert!(!fake.pull_is_draft(30));
    assert!(fake.auto_merge_enabled(30).is_some());
    // No task: no queue comment anywhere, and bosun never merges.
    assert!(fake.requests().iter().all(|r| !r.path.ends_with("/merge")));
    assert!(
        fake.requests()
            .iter()
            .all(|r| !(r.method == "POST" && r.path.contains("/comments")))
    );
}

#[test]
fn auto_merge_is_the_same_flag() {
    let (fake, dir) = setup();
    stamps::ready(&fake, dir.path(), 30);
    let (code, v) = pr(&fake, dir.path(), &["--pr", "30", "--auto-merge"]);
    assert_eq!(code, 0, "{v}");
    assert!(fake.auto_merge_enabled(30).is_some());
}

#[test]
fn ready_without_stamps_names_the_missing_gates() {
    let (fake, dir) = setup();
    fake.pull_head_sha(30, &stamps::head_sha(dir.path()));
    let (code, v) = pr(&fake, dir.path(), &["--pr", "30", "--ready"]);
    assert_eq!(code, 1, "{v}");
    assert!(
        v["data"]["missing_gates"]
            .as_array()
            .is_some_and(|g| !g.is_empty()),
        "{v}"
    );
    let fix = v["error"]["fix"].as_str().unwrap();
    assert!(fix.contains("bosun pr --pr 30 --ready"), "{fix}");
    assert!(fake.pull_is_draft(30));
    assert!(fake.auto_merge_enabled(30).is_none());
}

#[test]
fn ready_with_another_local_head_is_a_conflict() {
    let (fake, dir) = setup();
    stamps::stamp_all(dir.path());
    let (code, v) = pr(&fake, dir.path(), &["--pr", "30", "--ready"]);
    assert_eq!(code, 3, "{v}");
    assert!(
        v["error"]["fix"]
            .as_str()
            .unwrap()
            .contains("bosun pr --pr 30 --ready")
    );
}

#[test]
fn ready_needs_the_ruleset_to_require_the_gates() {
    let (fake, dir) = setup();
    fake.enforce_checks(&["ci-ok"]);
    stamps::ready(&fake, dir.path(), 30);
    let (code, v) = pr(&fake, dir.path(), &["--pr", "30", "--ready"]);
    assert_eq!(code, 5, "{v}");
    assert!(fake.pull_is_draft(30));
}

#[test]
fn a_session_without_graphql_gets_the_update_call() {
    let (fake, dir) = setup();
    fake.graphql_status(
        403,
        "403 GraphQL is not available from Claude Code sessions",
    );
    stamps::ready(&fake, dir.path(), 30);
    let (code, v) = pr(&fake, dir.path(), &["--pr", "30", "--ready"]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["ready"], "pending");
    assert_eq!(v["next"][0]["tool"], "mcp__github__update_pull_request");
    assert_eq!(v["next"][0]["args"]["pullNumber"], 30);
    assert_eq!(v["next"][0]["args"]["draft"], false);
}
