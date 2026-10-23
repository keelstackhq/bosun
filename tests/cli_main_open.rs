mod common;

use common::fake::FakeGithub;
use common::{bosun, run};
use serde_json::json;

const NOW: &str = "2026-10-23T10:00:00Z";

fn red(fake: &FakeGithub) {
    fake.issue(3, "main is red", &["bosun:main-red"], "ci-ok failed");
}

fn main_open(fake: &FakeGithub, pr: &str) -> (i32, serde_json::Value) {
    run(fake, "session_a", NOW, &["gate", "main-open", "--pr", pr])
}

fn status_of(fake: &FakeGithub, sha: &str) -> serde_json::Value {
    let all = fake.statuses_of(sha);
    let last = all.last().expect("a status was posted").clone();
    assert_eq!(last["context"], "main-open");
    last
}

#[test]
fn uc_m3_2_feature_pr_fails_and_mainfix_pr_passes_while_main_is_red() {
    let fake = FakeGithub::start();
    red(&fake);
    fake.pull(10, "feature", "feat", "No link.");
    fake.pull(11, "fix main", "fix", "No link.");
    fake.pull_set(11, "labels", json!([{"name": "bosun:kind/mainfix"}]));
    fake.issue(
        7,
        "revert",
        &[],
        r#"<!-- bosun:task {"item":"M3","task":"T9","kind":"mainfix"} -->"#,
    );
    fake.pull(12, "revert", "rev", "Closes #7");

    let (code, v) = main_open(&fake, "10");
    assert_eq!(code, 4, "{v}");
    assert_eq!(v["data"]["pass"], false);
    assert_eq!(v["data"]["main_red"], 3);
    assert!(v["error"]["fix"].as_str().unwrap().contains("#3"));
    let s = status_of(&fake, "sha10");
    assert_eq!(s["state"], "failure");
    assert!(s["description"].as_str().unwrap().contains("#3"));

    let (code, v) = main_open(&fake, "11");
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["mainfix"], true);
    assert_eq!(status_of(&fake, "sha11")["state"], "success");

    let (code, v) = main_open(&fake, "12");
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["mainfix"], true);
    assert_eq!(status_of(&fake, "sha12")["state"], "success");
}

#[test]
fn feature_pr_passes_while_main_is_open() {
    let fake = FakeGithub::start();
    fake.pull(10, "feature", "feat", "No link.");
    let (code, v) = main_open(&fake, "10");
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["main_red"], serde_json::Value::Null);
    let s = status_of(&fake, "sha10");
    assert_eq!(s["state"], "success");
    assert_eq!(s["description"], "main is open");
}

#[test]
fn missing_pr_number_is_usage() {
    let fake = FakeGithub::start();
    let (code, v) = run(&fake, "session_a", NOW, &["gate", "main-open"]);
    assert_eq!(code, 2, "{v}");
    assert!(v["error"]["fix"].as_str().unwrap().contains("--pr"));
}

#[test]
fn pr_comes_from_the_environment() {
    let fake = FakeGithub::start();
    fake.pull(10, "feature", "feat", "");
    let out = bosun()
        .env("BOSUN_GITHUB_API", fake.url())
        .env("BOSUN_PR", "10")
        .args(["gate", "main-open"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(0));
    assert_eq!(status_of(&fake, "sha10")["state"], "success");
}

#[test]
fn unknown_pr_is_exit_1() {
    let fake = FakeGithub::start();
    let (code, v) = main_open(&fake, "99");
    assert_eq!(code, 1, "{v}");
}
