//! Contents, failed jobs and job logs against the fake.

mod common;

use bosun::github::{Github, LiveGithub};
use common::TOKEN_CANARY;
use common::fake::{FakeGithub, LogHost, LogMode};

fn setup() -> (FakeGithub, LiveGithub) {
    let fake = FakeGithub::start();
    let gh = LiveGithub::with(&fake.url(), TOKEN_CANARY, "acme", "widget", 0);
    (fake, gh)
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
fn job_log_follows_the_redirect_without_github_headers() {
    let (fake, gh) = setup();
    let host = LogHost::start("a\nb\nc\n");
    fake.run_with_job("sha1", "test", "failure", "unused");
    fake.log_mode(LogMode::Redirect(format!("{}/log?sig=1", host.url)));
    let id = gh.failed_jobs("sha1").unwrap()[0].id;
    assert_eq!(gh.job_log(id, 2).unwrap(), "b\nc");
    let seen = host.headers_seen();
    assert_eq!(seen.len(), 1);
    for forbidden in ["authorization", "x-github-api-version"] {
        assert!(
            !seen[0].iter().any(|h| h == forbidden),
            "{forbidden}: {seen:?}"
        );
    }
    assert!(seen[0].iter().any(|h| h == "user-agent"));
    // the API host saw the token, once, on the first request only
    assert!(fake.requests().iter().all(|r| r.headers[0].is_some()));
}

#[test]
fn job_log_redirect_to_a_non_http_scheme_is_refused() {
    let (fake, gh) = setup();
    fake.run_with_job("sha1", "test", "failure", "a");
    fake.log_mode(LogMode::Redirect("ftp://logs.example.test/x".into()));
    let id = gh.failed_jobs("sha1").unwrap()[0].id;
    let e = gh.job_log(id, 5).unwrap_err();
    assert!(e.message.contains("not an https URL"), "{}", e.message);
}

#[test]
fn job_log_missing_log_and_failing_signed_url_are_errors() {
    let (fake, gh) = setup();
    assert!(gh.job_log(42, 5).is_err());
    // the signed URL answers 401 (the fake wants auth, which is never sent there)
    fake.run_with_job("sha1", "test", "failure", "a");
    fake.log_mode(LogMode::Redirect(format!("{}/again", fake.url())));
    let id = gh.failed_jobs("sha1").unwrap()[0].id;
    assert!(gh.job_log(id, 5).is_err());
}
