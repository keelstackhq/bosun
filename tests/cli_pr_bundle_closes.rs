//! `bosun pr <lead> --bundle <issue>...` writes a `Closes` line for every bundled issue.

mod common;

use std::process::Command;

use common::fake::FakeGithub;
use common::{bosun, one_object};
use serde_json::Value;

const BRANCH: &str = "claude/t11";
const BUILDING: &str = "bosun:state/building";
const READY: &str = "bosun:state/ready";
const LEASE: &str = r#"{"bosun":"lease","session":"session_test","origin":"spawned","expires":"2026-10-22T10:30:00Z","touch":["src/**","hoist.toml"]}"#;

fn repo() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    for args in [
        vec!["init", "-q", "-b", BRANCH],
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
        assert!(
            Command::new("git")
                .arg("-C")
                .arg(dir.path())
                .args(&args)
                .status()
                .unwrap()
                .success()
        );
    }
    dir
}

fn run_pr(fake: &FakeGithub, extra: &[&str]) -> (i32, Value) {
    let dir = repo();
    let mut args = vec!["pr", "11"];
    args.extend_from_slice(extra);
    let out = bosun()
        .env("BOSUN_GITHUB_API", fake.url())
        .arg("-C")
        .arg(dir.path())
        .args(&args)
        .output()
        .unwrap();
    (out.status.code().unwrap(), one_object(&out.stdout))
}

fn meta(epic: u64, extra: &str) -> String {
    format!(r#"Do it. <!-- bosun:task {{"item":"M1","task":"T","epic":{epic}{extra}}} -->"#)
}

/// Epic #10 (single), lead #11 (building, leased), ready bundle tasks #12 and #13, a gated #14,
/// a task #15 of another single epic #20, and a split epic #30 with task #31.
fn world() -> FakeGithub {
    world_with_lead("")
}

fn world_with_lead(lead_extra: &str) -> FakeGithub {
    let fake = FakeGithub::start();
    fake.issue(
        10,
        "epic",
        &[],
        r#"<!-- bosun:epic {"item":"M1","shape":"single"} -->"#,
    );
    fake.issue(11, "lead", &[BUILDING], &meta(10, lead_extra));
    fake.comment(11, LEASE);
    fake.compare_file("main", BRANCH, "src/a.rs", "@@ -1 +1 @@");
    for n in [12, 13] {
        fake.issue(n, "t", &[READY], &meta(10, ""));
    }
    fake.issue(14, "g", &[READY], &meta(10, r#","gated":true"#));
    fake.issue(
        20,
        "epic2",
        &[],
        r#"<!-- bosun:epic {"item":"M2","shape":"single"} -->"#,
    );
    fake.issue(15, "o", &[READY], &meta(20, ""));
    fake.issue(
        30,
        "epic3",
        &[],
        r#"<!-- bosun:epic {"item":"M3","shape":"split"} -->"#,
    );
    fake.issue(31, "s", &[READY], &meta(30, ""));
    fake
}

#[test]
fn a_new_pr_closes_the_lead_and_every_bundled_issue() {
    let fake = world();
    let (code, v) = run_pr(&fake, &["--bundle", "12", "--bundle", "13"]);
    assert_eq!(code, 0, "{v}");
    let n = v["data"]["pr"].as_u64().unwrap();
    let body = fake.pull_body(n);
    assert!(
        body.starts_with("Closes #11\nCloses #12\nCloses #13\n"),
        "{body}"
    );
    assert!(body.contains("<!-- bosun:task-pr 11 -->"), "{body}");
}

#[test]
fn a_gated_other_epic_or_split_bundle_issue_is_refused_before_any_change() {
    for bad in ["14", "15", "31", "11", "99"] {
        let fake = world();
        let (code, v) = run_pr(&fake, &["--bundle", "12", "--bundle", bad]);
        assert_eq!(code, 1, "{bad}: {v}");
        assert_eq!(
            v["error"]["fix"], "pass only the `bundle` issues `bosun wave plan` listed",
            "{bad}"
        );
        assert_eq!(fake.labels_of(11), vec![BUILDING], "{bad}");
        assert!(
            !fake
                .requests()
                .iter()
                .any(|r| r.method == "POST" && r.path.ends_with("/pulls")),
            "{bad}: a PR was created"
        );
    }
}

#[test]
fn an_existing_pr_missing_a_closes_is_a_conflict() {
    let fake = world();
    fake.pull(3, "t", BRANCH, "Closes #11\nCloses #12");
    let (code, v) = run_pr(&fake, &["--bundle", "12", "--bundle", "13"]);
    assert_eq!(code, 3, "{v}");
    assert_eq!(
        v["error"]["fix"],
        "add `Closes #N` for each bundled issue to the PR body, then rerun"
    );
    assert_eq!(fake.labels_of(11), vec![BUILDING]);
    assert_eq!(fake.pull_body(3), "Closes #11\nCloses #12");
}

#[test]
fn a_new_bundle_pr_changing_a_gated_file_is_refused() {
    let fake = world();
    fake.compare_file("main", BRANCH, "hoist.toml", "@@ -1 +1 @@");
    let (code, v) = run_pr(&fake, &["--bundle", "12"]);
    assert_eq!(code, 3, "{v}");
    assert!(
        v["error"]["message"].as_str().unwrap().contains("D-016"),
        "{v}"
    );
}

#[test]
fn a_bundled_issue_or_lead_whose_touch_is_gated_is_refused() {
    let touch = r#","touch":["hoist.toml"]"#;
    let fake = world();
    fake.issue(16, "t", &[READY], &meta(10, touch));
    let (code, v) = run_pr(&fake, &["--bundle", "16"]);
    assert_eq!(code, 1, "bundled: {v}");

    let fake = world_with_lead(touch);
    let (code, v) = run_pr(&fake, &["--bundle", "12"]);
    assert_eq!(code, 1, "lead: {v}");
}

#[test]
fn a_closed_lead_is_refused() {
    let fake = world();
    fake.close_issue(11, "2026-10-22T09:00:00Z");
    let (code, v) = run_pr(&fake, &["--bundle", "12"]);
    assert_eq!(code, 1, "{v}");
}

#[test]
fn another_repos_closes_does_not_make_a_gated_pr_a_bundle() {
    let fake = world();
    fake.pull(3, "t", BRANCH, "Closes #11\nFixes keelstackhq/splice#40");
    fake.pull_file(3, "hoist.toml", "modified", 1, 0, Some("@@ -1 +1 @@\n+x"));
    let (_, v) = run_pr(&fake, &[]);
    let msg = v["error"]["message"].as_str().unwrap_or("");
    assert!(!msg.contains("D-016"), "{v}");
}
