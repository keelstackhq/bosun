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
    let (code, v) = run(
        &fake,
        "s",
        T0,
        &["ci-log", "9", "--head", "0", "--tail", "2"],
    );
    assert_eq!(code, 0, "{v}");
    let jobs = v["data"]["jobs"].as_array().unwrap();
    assert_eq!(jobs.len(), 1);
    assert_eq!(jobs[0]["name"], "gate-test");
    assert_eq!(jobs[0]["conclusion"], "failure");
    assert_eq!(jobs[0]["log"], "… 2 lines cut …\nl3\nboom");
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

#[test]
fn long_log_keeps_head_tail_and_middle_errors() {
    let fake = FakeGithub::start();
    fake.pull(9, "t", "claude/x", "");
    let mut lines: Vec<String> = (1..=300).map(|i| format!("line {i}")).collect();
    lines[149] = "error[E0308]: mismatched types".into();
    fake.run_with_job("sha9", "gate-test", "failure", &lines.join("\n"));
    let (code, v) = run(&fake, "s", T0, &["ci-log", "9"]);
    assert_eq!(code, 0, "{v}");
    let log = v["data"]["jobs"][0]["log"].as_str().unwrap();
    assert!(log.starts_with("line 1\n"), "{log}");
    assert!(log.contains("line 20\n"), "{log}");
    assert!(log.contains("error[E0308]: mismatched types"), "{log}");
    assert!(log.ends_with("line 300"), "{log}");
    assert!(log.contains("line 221"), "{log}");
    assert!(!log.contains("line 100\n"), "{log}");
    assert!(log.contains("… 129 lines cut …"), "{log}");
    assert!(log.contains("… 70 lines cut …"), "{log}");
}
