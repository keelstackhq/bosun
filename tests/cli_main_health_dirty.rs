mod common;

use common::fake::FakeGithub;
use common::run;
use serde_json::{Value, json};

const NOW: &str = "2026-10-27T10:00:00Z";

fn health(fake: &FakeGithub, result: &str) -> (i32, Value) {
    run(fake, "session_a", NOW, &["main-health", "--result", result])
}

fn conflict_records(fake: &FakeGithub, issue: u64) -> Vec<Value> {
    fake.comments_of(issue)
        .iter()
        .filter_map(|c| {
            c.body
                .lines()
                .find(|l| l.starts_with(r#"{"bosun":"conflict""#))
        })
        .map(|l| serde_json::from_str(l).unwrap())
        .collect()
}

fn task(fake: &FakeGithub, n: u64, state: &str) {
    fake.issue(n, "task", &[&format!("bosun:state/{state}")], "body");
}

fn has_state(fake: &FakeGithub, n: u64, state: &str) -> bool {
    fake.labels_of(n).contains(&format!("bosun:state/{state}"))
}

/// The `task-pr` record the auto-merge step posts on the task, by `user`.
fn task_pr(fake: &FakeGithub, issue: u64, user: &str, pr: u64) {
    fake.comment_by(
        issue,
        user,
        &format!(r#"{{"bosun":"task-pr","session":"session_a","pr":{pr}}}"#),
    );
}

fn pr(fake: &FakeGithub, n: u64, body: &str, mergeable_state: Value) {
    fake.pull(n, "change", "feat", body);
    // A PR is also an issue: its conversation takes comments.
    fake.issue(n, "change", &[], "");
    fake.pull_set(n, "mergeable_state", mergeable_state);
}

#[test]
fn a_queued_task_with_a_dirty_pr_gets_a_record_and_a_pr_comment_and_stays_queued() {
    let fake = FakeGithub::start();
    task(&fake, 5, "queued");
    task_pr(&fake, 5, "alice", 10);
    pr(&fake, 10, "Closes #5", json!("dirty"));

    let (code, v) = health(&fake, "success");
    assert_eq!(code, 0, "{v}");
    assert_eq!(
        v["data"]["conflicts"],
        json!([{"pr": 10, "issue": 5, "posted": true,
            "fix": "merge main into feat, resolve, push; auto-merge stays on"}])
    );
    let on_pr: Vec<_> = fake.comments_of(10).into_iter().map(|c| c.body).collect();
    assert_eq!(
        on_pr,
        vec![
            "This PR conflicts with main (head sha10, base base000). \
             Merge main into feat, resolve, push; auto-merge stays on."
        ]
    );
    assert_eq!(
        conflict_records(&fake, 5),
        vec![json!({"bosun": "conflict", "pr": 10, "head_sha": "sha10", "base_sha": "base0000"})]
    );
    assert!(has_state(&fake, 5, "queued"), "{:?}", fake.labels_of(5));
}

#[test]
fn records_that_do_not_count_do_nothing() {
    let fake = FakeGithub::start();
    for n in [5, 6, 7] {
        task(&fake, n, "queued");
    }
    // #5: only a body link, no record. #6: a record by a user without write access.
    // #7: a writer's record naming a PR that links another issue.
    pr(&fake, 10, "Closes #5", json!("dirty"));
    fake.permission("mallory", "read");
    task_pr(&fake, 6, "mallory", 11);
    pr(&fake, 11, "Closes #6", json!("dirty"));
    task_pr(&fake, 7, "alice", 12);
    pr(&fake, 12, "Closes #9", json!("dirty"));

    let (code, v) = health(&fake, "success");
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["conflicts"], json!([]));
    for (n, p) in [(5, 10), (6, 11), (7, 12)] {
        assert!(fake.comments_of(p).is_empty());
        assert!(conflict_records(&fake, n).is_empty());
        assert!(has_state(&fake, n, "queued"));
    }
}

#[test]
fn posts_once_per_head_sha() {
    let fake = FakeGithub::start();
    task(&fake, 5, "queued");
    task_pr(&fake, 5, "alice", 10);
    pr(&fake, 10, "Closes #5", json!("dirty"));

    health(&fake, "success");
    let (code, v) = health(&fake, "success");
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["conflicts"][0]["posted"], false);
    assert_eq!(conflict_records(&fake, 5).len(), 1);
    assert_eq!(fake.comments_of(10).len(), 1);
    assert!(has_state(&fake, 5, "queued"));

    // A new base sha (main moved) posts nothing; a new head sha is a new conflict.
    fake.pull_set(10, "base", json!({"sha": "base1111", "ref": "main"}));
    health(&fake, "success");
    assert_eq!(fake.comments_of(10).len(), 1);
    assert_eq!(conflict_records(&fake, 5).len(), 1);
    fake.pull_set(10, "head", json!({"sha": "sha10b", "ref": "feat"}));
    health(&fake, "success");
    assert_eq!(conflict_records(&fake, 5).len(), 2);
    assert_eq!(fake.comments_of(10).len(), 2);
}

#[test]
fn clean_unknown_and_null_states_do_nothing_and_unknown_is_listed() {
    let fake = FakeGithub::start();
    for (n, p, state) in [
        (5, 10, json!("clean")),
        (6, 11, json!("unknown")),
        (7, 12, Value::Null),
    ] {
        task(&fake, n, "queued");
        task_pr(&fake, n, "alice", p);
        pr(&fake, p, &format!("Closes #{n}"), state);
    }

    let (code, v) = health(&fake, "success");
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["conflicts"], json!([]));
    assert_eq!(v["data"]["skipped_unknown"], json!([11, 12]));
    for n in [5, 6, 7] {
        assert!(conflict_records(&fake, n).is_empty());
        assert!(has_state(&fake, n, "queued"));
    }
}

#[test]
fn a_failing_pull_read_is_reported_apart_and_keeps_exit_0() {
    let fake = FakeGithub::start();
    task(&fake, 5, "queued");
    task(&fake, 6, "queued");
    task_pr(&fake, 5, "alice", 10);
    task_pr(&fake, 6, "alice", 11);
    pr(&fake, 10, "Closes #5", json!("dirty"));
    pr(&fake, 11, "Closes #6", json!("dirty"));
    fake.fail_path("/pulls/10", 500);

    let (code, v) = health(&fake, "success");
    assert_eq!(code, 0, "{v}");
    assert_eq!(
        v["data"]["conflict_errors"].as_array().unwrap().len(),
        1,
        "{v}"
    );
    assert_eq!(v["data"]["conflict_errors"][0]["issue"], 5);
    assert_eq!(v["data"]["rejudge_errors"], json!([]));
    // The next task is still checked.
    assert_eq!(v["data"]["conflicts"][0]["issue"], 6);
    assert_eq!(conflict_records(&fake, 6).len(), 1);
    assert!(conflict_records(&fake, 5).is_empty());
}

#[test]
fn the_newest_task_pr_record_wins() {
    let fake = FakeGithub::start();
    task(&fake, 5, "queued");
    task_pr(&fake, 5, "alice", 10);
    task_pr(&fake, 5, "alice", 11);
    pr(&fake, 10, "Closes #5", json!("dirty"));
    pr(&fake, 11, "Closes #5", json!("clean"));

    let (code, v) = health(&fake, "success");
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["conflicts"], json!([]));
    assert!(fake.comments_of(10).is_empty());

    fake.pull_set(11, "mergeable_state", json!("dirty"));
    let (_, v) = health(&fake, "success");
    assert_eq!(v["data"]["conflicts"][0]["pr"], 11);
}

#[test]
fn a_record_naming_a_closed_or_merged_pr_is_skipped() {
    let fake = FakeGithub::start();
    for (n, p, state) in [(5, 10, "closed"), (6, 11, "merged")] {
        task(&fake, n, "queued");
        task_pr(&fake, n, "alice", p);
        pr(&fake, p, &format!("Closes #{n}"), json!("dirty"));
        fake.pull_set(p, "state", json!("closed"));
        if state == "merged" {
            fake.pull_set(p, "merged", json!(true));
        }
    }

    let (code, v) = health(&fake, "success");
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["conflicts"], json!([]));
    assert_eq!(v["data"]["skipped_unknown"], json!([]));
    assert!(conflict_records(&fake, 5).is_empty());
    assert!(conflict_records(&fake, 6).is_empty());
}

#[test]
fn a_failing_queued_listing_lands_in_conflict_errors_without_an_issue() {
    let fake = FakeGithub::start();
    task(&fake, 5, "queued");
    task_pr(&fake, 5, "alice", 10);
    pr(&fake, 10, "Closes #5", json!("dirty"));
    // The re-judge lists the open PRs first; the queued listing after it fails.
    fake.after_requests("/pulls", 0, |s| s.fail_paths.push(("/issues".into(), 500)));

    let (code, v) = health(&fake, "success");
    assert_eq!(code, 0, "{v}");
    let errors = v["data"]["conflict_errors"].as_array().unwrap();
    assert_eq!(errors.len(), 1, "{v}");
    assert!(errors[0].get("issue").is_none());
    assert!(errors[0]["error"].is_string());
    assert_eq!(v["data"]["rejudge_errors"], json!([]));
}

#[test]
fn a_failure_run_reads_no_pull_detail() {
    let fake = FakeGithub::start();
    task(&fake, 5, "queued");
    task_pr(&fake, 5, "alice", 10);
    pr(&fake, 10, "Closes #5", json!("dirty"));

    let (code, v) = health(&fake, "failure");
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["conflicts"], json!([]));
    assert!(conflict_records(&fake, 5).is_empty());
    assert!(
        !fake
            .requests()
            .iter()
            .any(|r| r.path.ends_with("/pulls/10"))
    );
}
