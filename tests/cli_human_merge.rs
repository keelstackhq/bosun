//! F10: a human merge (no auto-merge) moves building, review and fixing tasks to merged.

mod common;

use std::path::Path;
use std::process::Command;

use common::fake::FakeGithub;
use common::{bosun, lease_note, one_object, run as run_as};
use serde_json::{Value, json};

const SESSION: &str = "session_a";
const NOW: &str = "2026-10-22T10:00:00Z";
const LIVE: &str = "2026-10-22T12:00:00Z";
const EXPIRED: &str = "2026-10-22T09:00:00Z";
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

fn bosun_in(fake: &FakeGithub, dir: &Path, args: &[&str]) -> (i32, Value) {
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

fn ends(fake: &FakeGithub, issue: u64, reason: &str) -> usize {
    fake.comments_of(issue)
        .iter()
        .filter(|c| c.body.contains(r#""bosun":"lease-end""#))
        .filter(|c| c.body.contains(&format!(r#""reason":"{reason}""#)))
        .count()
}

fn merged_label() -> Vec<String> {
    vec!["bosun:state/merged".to_string()]
}

const RESULT: [&str; 6] = ["result", "9", "--outcome", "merged", "--pr", "11"];

fn untouched(fake: &FakeGithub, label: &str) {
    assert_eq!(fake.labels_of(7), vec![label.to_string()]);
    assert_eq!(ends(fake, 7, "merged"), 0);
}

fn off_default(fake: &FakeGithub, pr: u64) {
    fake.pull_set(pr, "base", json!({"sha": "b0", "ref": "feature/x"}));
}

#[test]
fn result_ignores_a_pr_merged_into_another_branch() {
    let fake = FakeGithub::start();
    fake.issue(9, "Epic", &["bosun:state/claimed"], "");
    fake.issue(7, "task", &["bosun:state/review"], BODY);
    fake.comment(7, &lease_note(SESSION, LIVE, "src/a.rs"));
    fake.pull(11, "t", "claude/t7", "Closes #7");
    off_default(&fake, 11);
    fake.merge_pull(11, 7);
    let (code, v) = run_as(&fake, SESSION, NOW, &RESULT);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["tasks_merged"], json!([]));
    untouched(&fake, "bosun:state/review");
}

#[test]
fn result_ignores_a_task_whose_issue_is_still_open() {
    let fake = FakeGithub::start();
    fake.issue(9, "Epic", &["bosun:state/claimed"], "");
    fake.issue(7, "task", &["bosun:state/review"], BODY);
    fake.comment(7, &lease_note(SESSION, LIVE, "src/a.rs"));
    fake.pull(11, "t", "claude/t7", "Closes #7");
    fake.pull_merged(11, "abc");
    let (code, v) = run_as(&fake, SESSION, NOW, &RESULT);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["tasks_merged"], json!([]));
    untouched(&fake, "bosun:state/review");
}

#[test]
fn result_with_a_missing_pr_is_a_report_only() {
    let fake = FakeGithub::start();
    fake.issue(9, "Epic", &["bosun:state/claimed"], "");
    let (code, v) = run_as(&fake, SESSION, NOW, &RESULT);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["tasks_merged"], json!([]));
}

#[test]
fn result_does_not_move_claimed_or_blocked_tasks() {
    for st in ["claimed", "blocked"] {
        let fake = FakeGithub::start();
        fake.issue(9, "Epic", &["bosun:state/claimed"], "");
        let label = format!("bosun:state/{st}");
        fake.issue(7, "task", &[&label], BODY);
        fake.pull(11, "t", "claude/t7", "Closes #7");
        fake.merge_pull(11, 7);
        let (code, v) = run_as(&fake, SESSION, NOW, &RESULT);
        assert_eq!(code, 0, "{v}");
        assert_eq!(v["data"]["tasks_merged"], json!([]), "{st}");
        untouched(&fake, &label);
    }
}

#[test]
fn reap_leaves_an_open_building_task_alone_without_listing_pulls() {
    let fake = FakeGithub::start();
    fake.issue(7, "t", &["bosun:state/building"], BODY);
    fake.comment(7, &lease_note(SESSION, LIVE, "src/a.rs"));
    // An old merged PR links the task; the issue is open, so the work is not done.
    fake.pull(20, "old", "claude/old", "Closes #7");
    fake.pull_merged(20, "abc");
    let dir = repo("main");
    let (code, v) = bosun_in(&fake, dir.path(), &["reap"]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["merged"], json!([]));
    untouched(&fake, "bosun:state/building");
    let pulls = fake
        .requests()
        .iter()
        .filter(|r| r.method == "GET" && r.path.contains("/pulls"))
        .count();
    assert_eq!(pulls, 0);
}

#[test]
fn pr_ignores_a_pr_merged_into_another_base() {
    let fake = FakeGithub::start();
    fake.issue(7, "the task", &["bosun:state/review"], BODY);
    fake.comment(7, &lease_note(SESSION, LIVE, "src/a.rs"));
    fake.pull(20, "t", "claude/t7", "Closes #7");
    off_default(&fake, 20);
    fake.merge_pull(20, 7);
    let dir = repo("claude/t7");
    let _ = bosun_in(&fake, dir.path(), &["pr", "7"]);
    untouched(&fake, "bosun:state/review");
}

#[test]
fn pr_after_a_human_merge_moves_review_to_merged_and_ends_the_lease() {
    let fake = FakeGithub::start();
    fake.issue(7, "the task", &["bosun:state/review"], BODY);
    fake.comment(7, &lease_note(SESSION, LIVE, "src/a.rs"));
    fake.pull(20, "t", "claude/t7", "Closes #7");
    fake.merge_pull(20, 7);
    let dir = repo("claude/t7");
    let (code, v) = bosun_in(&fake, dir.path(), &["pr", "7"]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["task_state"], "merged");
    assert_eq!(v["data"]["merged"], true);
    assert_eq!(fake.labels_of(7), merged_label());
    assert_eq!(ends(&fake, 7, "merged"), 1);
}

#[test]
fn reap_moves_building_review_and_fixing_tasks_whose_pr_merged_even_when_closed() {
    let fake = FakeGithub::start();
    for (n, st) in [(7, "building"), (8, "review"), (9, "fixing")] {
        fake.issue(n, "t", &[&format!("bosun:state/{st}")], BODY);
        fake.comment(n, &lease_note(SESSION, LIVE, "src/a.rs"));
        fake.pull(
            20 + n,
            "t",
            &format!("claude/t{n}"),
            &format!("Closes #{n}"),
        );
        fake.merge_pull(20 + n, n);
    }
    let dir = repo("main");
    let (code, v) = bosun_in(&fake, dir.path(), &["reap"]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["merged"], json!([7, 8, 9]));
    for n in [7, 8, 9] {
        assert_eq!(fake.labels_of(n), merged_label(), "#{n}");
        assert_eq!(ends(&fake, n, "merged"), 1, "#{n}");
    }
}

#[test]
fn reap_ends_an_expired_lease_as_merged_not_reaped_and_does_not_make_it_ready() {
    let fake = FakeGithub::start();
    // Lease expired, PR merged by hand (GitHub closed the issue).
    fake.issue(7, "t", &["bosun:state/building"], BODY);
    fake.comment(7, &lease_note(SESSION, EXPIRED, "src/a.rs"));
    fake.pull(20, "t", "claude/t7", "Closes #7");
    fake.merge_pull(20, 7);
    let dir = repo("main");
    let (code, v) = bosun_in(&fake, dir.path(), &["reap"]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["merged"], json!([7]));
    assert_eq!(v["data"]["reaped"], json!([]));
    assert_eq!(fake.labels_of(7), merged_label());
    assert_eq!(ends(&fake, 7, "merged"), 1);
    assert_eq!(ends(&fake, 7, "reaped"), 0);
}

#[test]
fn reap_leaves_a_review_task_with_an_open_pr_alone() {
    let fake = FakeGithub::start();
    fake.issue(7, "t", &["bosun:state/review"], BODY);
    fake.comment(7, &lease_note(SESSION, LIVE, "src/a.rs"));
    fake.pull(20, "t", "claude/t7", "Closes #7");
    let dir = repo("main");
    let (code, v) = bosun_in(&fake, dir.path(), &["reap"]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["merged"], json!([]));
    assert_eq!(fake.labels_of(7), vec!["bosun:state/review".to_string()]);
    assert_eq!(ends(&fake, 7, "merged"), 0);
}

#[test]
fn result_merged_with_a_merged_pr_moves_the_linked_task_and_ends_this_sessions_lease() {
    let fake = FakeGithub::start();
    fake.issue(9, "Epic", &["bosun:state/claimed"], "");
    fake.issue(7, "task", &["bosun:state/review"], BODY);
    fake.comment(7, &lease_note(SESSION, LIVE, "src/a.rs"));
    fake.comment(7, &lease_note("session_other", LIVE, "src/b.rs"));
    fake.pull(11, "t", "claude/t7", "Closes #7");
    fake.merge_pull(11, 7);
    let args = ["result", "9", "--outcome", "merged", "--pr", "11"];
    let (code, v) = run_as(&fake, SESSION, NOW, &args);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["tasks_merged"], json!([7]));
    assert_eq!(fake.labels_of(7), merged_label());
    let mine = fake
        .comments_of(7)
        .iter()
        .filter(|c| c.body.contains(r#""bosun":"lease-end""#) && c.body.contains(SESSION))
        .count();
    assert_eq!((mine, ends(&fake, 7, "merged")), (1, 1));
}

#[test]
fn result_merged_with_an_unmerged_pr_changes_nothing() {
    let fake = FakeGithub::start();
    fake.issue(9, "Epic", &["bosun:state/claimed"], "");
    fake.issue(7, "task", &["bosun:state/review"], BODY);
    fake.comment(7, &lease_note(SESSION, LIVE, "src/a.rs"));
    fake.pull(11, "t", "claude/t7", "Closes #7");
    let args = ["result", "9", "--outcome", "merged", "--pr", "11"];
    let (code, v) = run_as(&fake, SESSION, NOW, &args);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["tasks_merged"], json!([]));
    assert_eq!(fake.labels_of(7), vec!["bosun:state/review".to_string()]);
    assert_eq!(ends(&fake, 7, "merged"), 0);
}
