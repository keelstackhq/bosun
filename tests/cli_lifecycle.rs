//! UC-M1-1: a task moves through its states by bosun commands and auto-merge only.

mod common;

use std::path::Path;
use std::process::Command;

use common::fake::FakeGithub;
use common::{bosun, one_object};
use serde_json::{Value, json};

const SESSION: &str = "session_a";
const NOW: &str = "2026-10-22T10:00:00Z";
const BODY: &str = r#"Do it.

<!-- bosun:task {"item":"M1","task":"T1","touch":["src/a.rs"],"kind":"feature"} -->"#;

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

fn run(fake: &FakeGithub, dir: &Path, args: &[&str]) -> (i32, Value) {
    let out = bosun()
        .env("BOSUN_GITHUB_API", fake.url())
        .env("BOSUN_SESSION", SESSION)
        .env("BOSUN_NOW", NOW)
        .arg("-C")
        .arg(dir)
        .args(args)
        .output()
        .unwrap();
    (out.status.code().unwrap(), one_object(&out.stdout))
}

/// The state labels added to issue 7, in request order.
fn added_states(fake: &FakeGithub) -> Vec<String> {
    fake.requests()
        .iter()
        .filter(|r| r.method == "POST" && r.path.ends_with("/issues/7/labels"))
        .filter_map(|r| r.body.clone())
        .filter_map(|b| b.split("bosun:state/").nth(1).map(String::from))
        .map(|s| s.split('"').next().unwrap().to_string())
        .collect()
}

fn lease_ends(fake: &FakeGithub) -> Vec<String> {
    fake.comments_of(7)
        .into_iter()
        .filter(|c| c.body.contains(r#""bosun":"lease-end""#))
        .map(|c| c.body)
        .collect()
}

fn assert_nobody_merged(fake: &FakeGithub) {
    let merges: Vec<_> = fake
        .requests()
        .into_iter()
        .filter(|r| r.method == "PUT" || r.path.ends_with("/merge"))
        .collect();
    assert!(merges.is_empty(), "a request merged: {merges:?}");
}

fn green(fake: &FakeGithub, sha: &str) {
    for ctx in ["ci-ok", "risk-check", "review-verdict", "main-open"] {
        fake.status(sha, ctx, "success", "");
    }
}

#[test]
fn uc_m1_1_a_task_goes_ready_to_merged_by_bosun_commands_only() {
    // planned -> ready comes from plan import (milestone M2, another session); start at ready.
    let fake = FakeGithub::start();
    fake.issue(7, "the task", &["bosun:state/ready"], BODY);
    let dir = repo("claude/t7");

    let (code, v) = run(&fake, dir.path(), &["claim", "7"]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(fake.labels_of(7), vec!["bosun:state/claimed".to_string()]);

    let (code, v) = run(&fake, dir.path(), &["pr", "7"]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["task_state"], "review");
    assert_eq!(added_states(&fake), ["claimed", "building", "review"]);
    assert_eq!(fake.labels_of(7), vec!["bosun:state/review".to_string()]);
    let pr = v["data"]["pr"].as_u64().unwrap();

    green(&fake, v["data"]["head_sha"].as_str().unwrap());
    let (code, v) = run(&fake, dir.path(), &["pr", "7", "--auto-merge"]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["task_state"], "queued");
    assert_eq!(fake.labels_of(7), vec!["bosun:state/queued".to_string()]);

    // GitHub merges the PR and closes the issue; no bosun command does.
    fake.merge_pull(pr, 7);
    let (code, v) = run(&fake, dir.path(), &["reap"]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["merged"], json!([7]));
    assert_eq!(fake.labels_of(7), vec!["bosun:state/merged".to_string()]);
    let ends = lease_ends(&fake);
    assert_eq!(ends.len(), 1, "{ends:?}");
    assert!(ends[0].contains(r#""reason":"merged""#) && ends[0].contains(SESSION));
    assert_nobody_merged(&fake);

    // A second reap changes nothing.
    let (_, v) = run(&fake, dir.path(), &["reap"]);
    assert_eq!(v["data"]["merged"], json!([]));
}

#[test]
fn pr_after_the_merge_moves_queued_to_merged_without_a_new_pr() {
    let fake = FakeGithub::start();
    fake.issue(7, "the task", &["bosun:state/queued"], BODY);
    fake.comment(
        7,
        &common::lease_note(SESSION, "2026-10-22T10:20:00Z", "src/a.rs"),
    );
    fake.pull(20, "t", "claude/t7", "Closes #7");
    fake.merge_pull(20, 7);
    let dir = repo("claude/t7");
    let (code, v) = run(&fake, dir.path(), &["pr", "7"]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["task_state"], "merged");
    assert_eq!(v["data"]["merged"], true);
    assert_eq!(v["data"]["pr"], 20);
    assert_eq!(fake.labels_of(7), vec!["bosun:state/merged".to_string()]);
    assert_eq!(lease_ends(&fake).len(), 1);
    let creates = fake
        .requests()
        .iter()
        .filter(|r| r.method == "POST" && r.path.ends_with("/pulls"))
        .count();
    assert_eq!(creates, 0);
    assert_nobody_merged(&fake);
}

#[test]
fn a_direct_label_edit_is_refused_by_the_hook() {
    let fake = FakeGithub::start();
    let dir = repo("claude/t7");
    let input = json!({"session_id": "s", "tool_name": "Bash",
        "tool_input": {"command": "gh issue edit 7 --add-label bosun:state/merged"},
        "cwd": dir.path().to_str().unwrap()});
    let out = bosun()
        .current_dir(dir.path())
        .env("BOSUN_GITHUB_API", fake.url())
        .args(["hook", "pre-tool-use"])
        .write_stdin(input.to_string())
        .output()
        .unwrap();
    let v: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["hookSpecificOutput"]["permissionDecision"], "deny");
}
