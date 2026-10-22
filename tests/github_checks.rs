//! Statuses, check runs, contents, auto-merge and job logs against the fake.

mod common;

use bosun::github::{Github, LiveGithub};
use common::TOKEN_CANARY;
use common::fake::{FakeGithub, LogMode};

fn setup() -> (FakeGithub, LiveGithub) {
    let fake = FakeGithub::start();
    let gh = LiveGithub::with(&fake.url(), TOKEN_CANARY, "acme", "widget", 0);
    (fake, gh)
}

#[test]
fn create_status_posts_and_truncates_the_description() {
    let (fake, gh) = setup();
    gh.create_status(
        "sha1",
        "bosun/review",
        "success",
        "ok",
        Some("https://x.test/r"),
    )
    .unwrap();
    gh.create_status("sha1", "bosun/scope", "failure", &"é".repeat(200), None)
        .unwrap();
    let all = fake.statuses_of("sha1");
    assert_eq!(all.len(), 2);
    assert_eq!(all[0]["context"], "bosun/review");
    assert_eq!(all[0]["target_url"], "https://x.test/r");
    assert_eq!(all[1]["description"].as_str().unwrap().chars().count(), 140);
    assert!(all[1]["target_url"].is_null());
    let e = gh
        .create_status("sha1", "c", "green", "d", None)
        .unwrap_err();
    assert!(e.message.contains("green"), "{}", e.message);
    assert_eq!(fake.statuses_of("sha1").len(), 2);
}

#[test]
fn statuses_keep_the_newest_per_context() {
    let (fake, gh) = setup();
    fake.status("sha1", "ci", "pending", "running");
    fake.status("sha1", "other", "success", "fine");
    gh.create_status("sha1", "ci", "failure", "tests failed", None)
        .unwrap();
    let got = gh.statuses("sha1").unwrap();
    assert_eq!(got.len(), 2);
    let ci = got.iter().find(|s| s.context == "ci").unwrap();
    assert_eq!(
        (ci.state.as_str(), ci.description.as_str()),
        ("failure", "tests failed")
    );
    assert!(gh.statuses("nosuchsha").unwrap().is_empty());
}

#[test]
fn check_runs_read_every_page() {
    let (fake, gh) = setup();
    fake.check_run("sha1", "build", "completed", Some("success"));
    fake.check_run("sha1", "lint", "completed", Some("failure"));
    fake.check_run("sha1", "test", "in_progress", None);
    let runs = gh.check_runs("sha1").unwrap();
    assert_eq!(runs.len(), 3);
    assert_eq!(
        (runs[1].name.as_str(), runs[1].conclusion.as_deref()),
        ("lint", Some("failure"))
    );
    assert_eq!(
        (runs[2].status.as_str(), runs[2].conclusion.clone()),
        ("in_progress", None)
    );
    assert!(gh.check_runs("other").unwrap().is_empty());
}

#[test]
fn file_at_decodes_files_and_maps_missing_and_directories_to_none() {
    let (fake, gh) = setup();
    let text = "line one\nline two with ünïcode\n".repeat(5);
    fake.content("docs/a b.md", "main", &text);
    fake.content("docs/x.txt", "main", "x");
    assert_eq!(gh.file_at("docs/a b.md", "main").unwrap().unwrap(), text);
    assert_eq!(
        gh.file_at("docs/x.txt", "main").unwrap().as_deref(),
        Some("x")
    );
    assert_eq!(gh.file_at("docs/x.txt", "other").unwrap(), None);
    assert_eq!(gh.file_at("nope.md", "main").unwrap(), None);
    assert_eq!(gh.file_at("docs", "main").unwrap(), None); // a directory
    assert!(fake.requests().iter().all(|r| !r.path.contains(' ')));
}

#[test]
fn auto_merge_sends_the_mutation_with_the_node_id() {
    let (fake, gh) = setup();
    fake.pull(7, "t", "b", "");
    assert_eq!(fake.auto_merge_enabled(7), None);
    gh.enable_auto_merge("PR_node7", "SQUASH").unwrap();
    assert_eq!(fake.auto_merge_enabled(7).as_deref(), Some("SQUASH"));
    let req = fake.requests().pop().unwrap();
    assert_eq!(
        (req.method.as_str(), req.path.as_str()),
        ("POST", "/graphql")
    );
    let body = req.body.unwrap();
    assert!(body.contains("enablePullRequestAutoMerge"), "{body}");
    assert!(body.contains("PR_node7"), "{body}");
}

#[test]
fn graphql_errors_become_errors_with_the_first_message() {
    let (fake, gh) = setup();
    fake.pull(7, "t", "b", "");
    let e = gh.enable_auto_merge("PR_nope", "MERGE").unwrap_err();
    assert!(
        e.message.contains("Could not resolve to a node"),
        "{}",
        e.message
    );
    assert!(!e.fix.is_empty());
    fake.graphql_error("Pull request is in clean status");
    let e = gh.enable_auto_merge("PR_node7", "MERGE").unwrap_err();
    assert!(
        e.message.contains("Pull request is in clean status"),
        "{}",
        e.message
    );
    assert_eq!(fake.auto_merge_enabled(7), None);
    let e = gh.enable_auto_merge("PR_node7", "FAST").unwrap_err();
    assert!(e.message.contains("FAST"), "{}", e.message);
}

#[test]
fn failed_jobs_keep_only_failures_across_runs() {
    let (fake, gh) = setup();
    fake.run_with_job("sha1", "build", "success", "ok");
    fake.run_with_job("sha1", "test", "failure", "boom");
    fake.run_with_job("sha1", "lint", "cancelled", "");
    fake.run_with_job("sha1", "deploy", "timed_out", "");
    fake.run_with_job("sha1", "docs", "skipped", "");
    fake.run_with_job("sha2", "test", "failure", "elsewhere");
    let jobs = gh.failed_jobs("sha1").unwrap();
    let names: Vec<&str> = jobs.iter().map(|j| j.name.as_str()).collect();
    assert_eq!(names, ["test", "lint", "deploy"]);
    assert_eq!(jobs[0].conclusion, "failure");
    assert_ne!(jobs[0].run_id, jobs[1].run_id);
    assert!(jobs[0].html_url.contains("/runs/"), "{}", jobs[0].html_url);
    assert!(gh.failed_jobs("none").unwrap().is_empty());
}

#[test]
fn job_log_keeps_the_last_lines() {
    let (fake, gh) = setup();
    let log: String = (1..=10).map(|i| format!("line {i}\r\n")).collect();
    fake.run_with_job("sha1", "test", "failure", &log);
    let id = gh.failed_jobs("sha1").unwrap()[0].id;
    assert_eq!(gh.job_log(id, 3).unwrap(), "line 8\nline 9\nline 10");
    assert_eq!(gh.job_log(id, 50).unwrap().lines().count(), 10);
    assert_eq!(gh.job_log(id, 0).unwrap(), "");
}

#[test]
fn job_log_follows_one_redirect_on_the_same_host() {
    let (fake, gh) = setup();
    fake.run_with_job("sha1", "test", "failure", "a\nb\nc");
    fake.log_mode(LogMode::SameHost);
    let id = gh.failed_jobs("sha1").unwrap()[0].id;
    assert_eq!(gh.job_log(id, 2).unwrap(), "b\nc");
}

#[test]
fn job_log_on_another_host_is_an_error_naming_the_url() {
    let (fake, gh) = setup();
    fake.run_with_job("sha1", "test", "failure", "a");
    fake.log_mode(LogMode::Elsewhere(
        "https://logs.example.test/x?sig=1".into(),
    ));
    let id = gh.failed_jobs("sha1").unwrap()[0].id;
    let e = gh.job_log(id, 5).unwrap_err();
    assert!(
        e.message.contains("https://logs.example.test/x?sig=1"),
        "{}",
        e.message
    );
    // the token was only ever sent to the fake
    assert!(fake.requests().iter().all(|r| r.headers[0].is_some()));
}
