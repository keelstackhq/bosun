mod common;

use common::fake::{FakeGithub, add_comment};
use common::{bosun, one_object};
use serde_json::{Value, json};

/// `bosun wait ...` with a 20 ms poll base; returns (exit code, envelope).
fn wait(fake: &FakeGithub, args: &[&str]) -> (i32, Value) {
    let out = bosun()
        .env("BOSUN_GITHUB_API", fake.url())
        .env("BOSUN_WAIT_BASE_MS", "20")
        .arg("wait")
        .args(args)
        .output()
        .unwrap();
    (out.status.code().unwrap(), one_object(&out.stdout))
}

#[test]
fn a_pr_that_merges_after_two_polls_returns_changed_with_three_snapshots() {
    let fake = FakeGithub::start();
    fake.pull(5, "Work", "claude/work", "");
    fake.status("sha5", "ci-ok", "pending", "");
    fake.check_run("sha5", "rust", "in_progress", None);
    fake.after_requests("/pulls/5", 2, |s| {
        s.pulls[0]["state"] = json!("closed");
        s.pulls[0]["merged"] = json!(true);
    });
    let (code, v) = wait(&fake, &["--pr", "5", "--timeout", "30"]);
    assert_eq!(code, 0, "{v}");
    let d = &v["data"];
    assert_eq!(d["changed"], true);
    assert_eq!(d["polls"], 3);
    assert_eq!(d["before"]["state"], "open");
    assert_eq!(d["before"]["merged"], false);
    assert_eq!(d["before"]["head_sha"], "sha5");
    assert_eq!(d["before"]["statuses"]["ci-ok"], "pending");
    assert_eq!(d["before"]["checks"]["rust"], Value::Null);
    assert_eq!(d["after"]["state"], "closed");
    assert_eq!(d["after"]["merged"], true);
}

#[test]
fn a_check_conclusion_counts_as_a_change() {
    let fake = FakeGithub::start();
    fake.pull(5, "Work", "claude/work", "");
    fake.check_run("sha5", "rust", "in_progress", None);
    fake.after_requests("/commits/sha5/check-runs", 1, |s| {
        s.check_runs.get_mut("sha5").unwrap()[0]["conclusion"] = json!("success");
    });
    let (code, v) = wait(&fake, &["--pr", "5", "--timeout", "30"]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["changed"], true);
    assert_eq!(v["data"]["polls"], 2);
    assert_eq!(v["data"]["after"]["checks"]["rust"], "success");
}

#[test]
fn a_state_label_and_a_comment_change_an_issue() {
    let fake = FakeGithub::start();
    fake.issue(7, "Task", &["bosun:state/review"], "");
    fake.after_requests("/issues/7", 1, |s| {
        s.issues[0].labels = vec!["bosun:state/merged".into()];
    });
    let (_, v) = wait(&fake, &["--issue", "7", "--timeout", "30"]);
    assert_eq!(v["data"]["changed"], true);
    assert_eq!(v["data"]["before"]["state"], "review");
    assert_eq!(v["data"]["after"]["state"], "merged");
    assert_eq!(v["data"]["after"]["open"], true);

    let fake = FakeGithub::start();
    fake.issue(7, "Task", &["bosun:state/review"], "");
    fake.after_requests("/issues/7/comments", 1, |s| {
        add_comment(s, 7, "hello");
    });
    let (_, v) = wait(&fake, &["--issue", "7", "--timeout", "30"]);
    assert_eq!(v["data"]["changed"], true);
    assert_eq!(v["data"]["before"]["comments"], 0);
    assert_eq!(v["data"]["after"]["comments"], 1);
}

#[test]
fn a_timeout_is_exit_0_with_changed_false() {
    let fake = FakeGithub::start();
    fake.pull(5, "Work", "claude/work", "");
    let (code, v) = wait(&fake, &["--pr", "5", "--timeout", "1"]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["changed"], false);
    assert!(v["data"]["polls"].as_u64().unwrap() >= 3);
    assert_eq!(v["data"]["before"], v["data"]["after"]);
}

#[test]
fn a_child_result_ends_the_wait() {
    let fake = FakeGithub::start();
    fake.issue(10, "Epic", &["bosun:state/claimed"], "");
    // A result of another session does not count.
    fake.comment(
        10,
        r#"{"bosun":"result","session":"session_other","outcome":"blocked"}"#,
    );
    fake.after_requests("/issues/10/comments", 1, |s| {
        add_comment(
            s,
            10,
            r#"{"bosun":"result","session":"session_child","outcome":"merged","pr":42}"#,
        );
    });
    let (code, v) = wait(&fake, &["--child", "session_child", "--timeout", "30"]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["changed"], true);
    assert_eq!(v["data"]["before"]["result"], Value::Null);
    let after = &v["data"]["after"]["result"];
    assert_eq!(
        (after["issue"].as_u64(), after["pr"].as_u64()),
        (Some(10), Some(42))
    );
    assert_eq!(after["outcome"], "merged");
}

#[test]
fn wait_never_steers_or_schedules() {
    let fake = FakeGithub::start();
    fake.issue(7, "Task", &["bosun:state/review"], "");
    let (_, v) = wait(&fake, &["--issue", "7", "--timeout", "0"]);
    assert_eq!(v["next"], json!([]));
    assert_eq!(v["data"]["polls"], 1);
    assert!(fake.requests().iter().all(|r| r.method == "GET"));
}

#[test]
fn the_target_is_required_and_unique() {
    let fake = FakeGithub::start();
    let (code, v) = wait(&fake, &["--timeout", "1"]);
    assert_eq!(code, 2, "{v}");
    let (code, _) = wait(&fake, &["--pr", "1", "--issue", "2"]);
    assert_eq!(code, 2);
}
