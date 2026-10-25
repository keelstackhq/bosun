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

#[test]
fn a_merged_pr_of_another_task_on_the_same_branch_is_ignored() {
    // The session merged task 6 on its branch, then claims task 7 on the same branch.
    let fake = FakeGithub::start();
    fake.issue(6, "old", &["bosun:state/merged"], BODY);
    fake.issue(7, "the task", &["bosun:state/ready"], BODY);
    fake.pull(20, "t", "claude/t7", "Closes #6");
    fake.merge_pull(20, 6);
    let dir = repo("claude/t7");
    let (code, v) = run(&fake, dir.path(), &["claim", "7"]);
    assert_eq!(code, 0, "{v}");
    let (code, v) = run(&fake, dir.path(), &["pr", "7"]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["created"], true);
    assert_eq!(fake.labels_of(7), vec!["bosun:state/review".to_string()]);
    assert!(lease_ends(&fake).is_empty());
}

fn queued_with_lease(fake: &FakeGithub) {
    fake.issue(7, "the task", &["bosun:state/queued"], BODY);
    fake.comment(
        7,
        &common::lease_note(SESSION, "2026-10-22T10:20:00Z", "src/a.rs"),
    );
}

#[test]
fn reap_leaves_a_queued_task_with_an_open_pr_alone() {
    // An old merged PR links #7 (the issue was reopened); the new PR 21 is open.
    let fake = FakeGithub::start();
    queued_with_lease(&fake);
    fake.pull(20, "t", "someone/old", "Fixes #7");
    fake.merge_pull(20, 99);
    fake.pull(21, "t", "claude/t7", "Closes #7");
    let dir = repo("claude/t7");
    let (code, v) = run(&fake, dir.path(), &["reap"]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["merged"], json!([]));
    assert_eq!(fake.labels_of(7), vec!["bosun:state/queued".to_string()]);
    assert!(lease_ends(&fake).is_empty());
}

#[test]
fn reap_ignores_a_pr_merged_into_a_branch_other_than_the_default() {
    let fake = FakeGithub::start();
    queued_with_lease(&fake);
    fake.pull(20, "t", "claude/t7", "Closes #7");
    fake.pull_set(20, "base", json!({"sha": "b", "ref": "feature-x"}));
    fake.merge_pull(20, 7);
    let dir = repo("claude/t7");
    let (_, v) = run(&fake, dir.path(), &["reap"]);
    assert_eq!(v["data"]["merged"], json!([]));
    assert_eq!(fake.labels_of(7), vec!["bosun:state/queued".to_string()]);
}

/// `GET /pulls` lists (every PR of the repo) made after the first `skip` requests.
fn pull_lists_after(fake: &FakeGithub, skip: usize) -> usize {
    fake.requests()
        .iter()
        .skip(skip)
        .filter(|r| r.method == "GET" && r.path.ends_with("/pulls"))
        .count()
}

/// `pr --auto-merge` records the task's PR, so reap reads that PR instead of listing them all.
#[test]
fn reap_reads_the_recorded_pr_without_listing_all_pulls() {
    let fake = FakeGithub::start();
    fake.issue(7, "the task", &["bosun:state/ready"], BODY);
    let dir = repo("claude/t7");
    assert_eq!(run(&fake, dir.path(), &["claim", "7"]).0, 0);
    let (code, v) = run(&fake, dir.path(), &["pr", "7"]);
    assert_eq!(code, 0, "{v}");
    let pr = v["data"]["pr"].as_u64().unwrap();
    green(&fake, v["data"]["head_sha"].as_str().unwrap());
    let (code, v) = run(&fake, dir.path(), &["pr", "7", "--auto-merge"]);
    assert_eq!(code, 0, "{v}");
    let record = format!(r#"{{"bosun":"task-pr","session":"{SESSION}","pr":{pr}}}"#);
    assert!(
        fake.comments_of(7).iter().any(|c| c.body.contains(&record)),
        "no {record}"
    );

    // Still open: reap leaves it, without a sweep.
    let before = fake.requests().len();
    let (_, v) = run(&fake, dir.path(), &["reap"]);
    assert_eq!(v["data"]["merged"], json!([]));
    assert_eq!(pull_lists_after(&fake, before), 0);

    fake.merge_pull(pr, 7);
    let before = fake.requests().len();
    let (code, v) = run(&fake, dir.path(), &["reap"]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["merged"], json!([7]));
    assert_eq!(fake.labels_of(7), vec!["bosun:state/merged".to_string()]);
    assert_eq!(pull_lists_after(&fake, before), 0);
    assert_nobody_merged(&fake);
}

/// A task without the record (written before it existed) still gets the sweep.
#[test]
fn reap_sweeps_for_a_task_without_a_recorded_pr() {
    let fake = FakeGithub::start();
    queued_with_lease(&fake);
    fake.pull(20, "t", "claude/t7", "Closes #7");
    fake.merge_pull(20, 7);
    let dir = repo("claude/t7");
    let (_, v) = run(&fake, dir.path(), &["reap"]);
    assert_eq!(v["data"]["merged"], json!([7]));
    assert_eq!(pull_lists_after(&fake, 0), 1);
}

/// A recorded PR that does not link the task is not trusted; the sweep decides.
#[test]
fn a_recorded_pr_that_does_not_link_the_task_falls_back_to_the_sweep() {
    let fake = FakeGithub::start();
    queued_with_lease(&fake);
    fake.comment(7, r#"{"bosun":"task-pr","session":"session_a","pr":30}"#);
    fake.pull(30, "other", "claude/x", "Closes #8");
    fake.merge_pull(30, 8);
    let dir = repo("claude/t7");
    let (_, v) = run(&fake, dir.path(), &["reap"]);
    assert_eq!(v["data"]["merged"], json!([]));
    assert_eq!(fake.labels_of(7), vec!["bosun:state/queued".to_string()]);
    assert_eq!(pull_lists_after(&fake, 0), 1);
}

/// A recorded PR closed unmerged, or merged into another branch, is no answer: the sweep decides.
#[test]
fn a_recorded_pr_closed_unmerged_or_off_the_default_branch_falls_back_to_the_sweep() {
    for off_default in [false, true] {
        let fake = FakeGithub::start();
        queued_with_lease(&fake);
        fake.comment(7, r#"{"bosun":"task-pr","session":"session_a","pr":30}"#);
        fake.pull(30, "first", "claude/t7", "Closes #7");
        if off_default {
            fake.pull_set(30, "base", json!({"sha": "b", "ref": "feature-x"}));
            fake.merge_pull(30, 7);
        } else {
            fake.pull_set(30, "state", json!("closed"));
        }
        // The replacement PR, opened by hand, merged into the default branch.
        fake.pull(31, "second", "claude/t7b", "Closes #7");
        fake.merge_pull(31, 7);
        let dir = repo("claude/t7");
        let (_, v) = run(&fake, dir.path(), &["reap"]);
        assert_eq!(v["data"]["merged"], json!([7]), "off_default={off_default}");
        assert_eq!(pull_lists_after(&fake, 0), 1);
    }
}
