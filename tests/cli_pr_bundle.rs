//! `bosun pr` refuses a PR that closes several tasks and changes a gated file (D-016 backstop).

mod common;

use std::process::Command;

use common::fake::FakeGithub;
use common::{bosun, one_object};
use serde_json::{Value, json};

const BRANCH: &str = "claude/t7";
const BUILDING: &str = "bosun:state/building";
const LEASE: &str = r#"{"bosun":"lease","session":"session_test","origin":"spawned","expires":"2026-10-22T10:30:00Z","touch":["src/a.rs","crates/x/hoist.toml"]}"#;

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

fn run_pr(fake: &FakeGithub, extra: &[&str]) -> (i32, Value) {
    let dir = repo();
    let mut args = vec!["pr", "7"];
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

/// Task #7 in building, with or without this session's lease, no PR yet.
fn task(lease: bool) -> FakeGithub {
    let fake = FakeGithub::start();
    fake.issue(7, "the task", &[BUILDING], "Do it.");
    if lease {
        fake.comment(7, LEASE);
    }
    fake
}

/// Task #7 with an open PR #3 closing `body`'s issues and listing `files`.
fn world(body: &str, files: &[&str], changed: u64) -> FakeGithub {
    let fake = task(true);
    fake.pull(3, "t", BRANCH, body);
    fake.pull_set(3, "changed_files", json!(changed));
    for name in files {
        fake.pull_file(3, name, "modified", 1, 0, Some("@@ -1 +1 @@"));
    }
    fake
}

const TWO: &str = "Closes #7\nCloses #8";

#[test]
fn a_two_task_pr_changing_a_gated_file_is_refused() {
    let fake = world(TWO, &["src/a.rs", "crates/x/hoist.toml"], 2);
    let (code, v) = run_pr(&fake, &[]);
    assert_eq!(code, 3, "{v}");
    let m = v["error"]["message"].as_str().unwrap();
    assert!(
        m.contains("closes 2 tasks") && m.contains("crates/x/hoist.toml"),
        "{v}"
    );
    assert!(m.contains("D-016"), "{v}");
    assert_eq!(
        v["error"]["fix"],
        "move the gated changes to their own task and PR, then run `bosun pr` again"
    );
    assert_eq!(fake.labels_of(7), vec![BUILDING]);
}

#[test]
fn an_incomplete_file_list_fails_closed() {
    let fake = world(TWO, &["src/a.rs"], 3);
    let (code, v) = run_pr(&fake, &[]);
    assert_eq!(code, 3, "{v}");
    assert!(
        v["error"]["message"]
            .as_str()
            .unwrap()
            .contains("incomplete"),
        "{v}"
    );
}

#[test]
fn a_two_task_pr_without_gated_files_passes_the_check() {
    let fake = world(TWO, &["src/a.rs"], 1);
    let (code, v) = run_pr(&fake, &[]);
    assert!(!v.to_string().contains("closes 2 tasks"), "{v}");
    assert_eq!(code, 0, "{v}");
}

#[test]
fn a_one_task_pr_may_change_a_gated_file() {
    let fake = world("Closes #7", &["crates/x/hoist.toml"], 1);
    let (_, v) = run_pr(&fake, &[]);
    assert!(!v.to_string().contains("closes 1 tasks"), "{v}");
    assert!(!v.to_string().contains("D-016"), "{v}");
}
