mod common;

use common::fake::FakeGithub;
use common::run;

const T0: &str = "2026-10-22T10:00:00Z";

#[test]
fn returns_the_tail_of_the_failed_job() {
    let fake = FakeGithub::start();
    fake.pull(9, "t", "claude/x", "");
    fake.run_with_job("sha9", "gate-test", "failure", "l1\nl2\nl3\nboom");
    fake.run_with_job("sha9", "gate-fmt", "success", "fine");
    let (code, v) = run(&fake, "s", T0, &["ci-log", "9", "--tail", "2"]);
    assert_eq!(code, 0, "{v}");
    let jobs = v["data"]["jobs"].as_array().unwrap();
    assert_eq!(jobs.len(), 1);
    assert_eq!(jobs[0]["name"], "gate-test");
    assert_eq!(jobs[0]["conclusion"], "failure");
    assert_eq!(jobs[0]["log"], "l3\nboom");
    assert!(jobs[0]["url"].as_str().unwrap().contains("/job/"));
    assert_eq!(v["data"]["head_sha"], "sha9");
}

#[test]
fn no_failed_jobs_is_an_empty_list() {
    let fake = FakeGithub::start();
    fake.pull(9, "t", "claude/x", "");
    fake.run_with_job("sha9", "gate-fmt", "success", "fine");
    let (code, v) = run(&fake, "s", T0, &["ci-log", "9"]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["jobs"], serde_json::json!([]));
}

#[test]
fn unknown_pr_is_exit_1() {
    let fake = FakeGithub::start();
    let (code, _) = run(&fake, "s", T0, &["ci-log", "99"]);
    assert_eq!(code, 1);
}
