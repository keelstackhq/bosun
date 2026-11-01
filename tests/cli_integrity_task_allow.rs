//! `bosun test-integrity --pr <n>` reads the PR body and the linked task's `allow` from GitHub.
mod common;

use common::fake::FakeGithub;
use common::{bosun, one_object};
use serde_json::{Value, json};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn git(root: &Path, args: &[&str]) {
    let out = Command::new("git")
        .current_dir(root)
        .args(args)
        .output()
        .expect("git");
    assert!(out.status.success(), "git {args:?}: {out:?}");
}

/// A repo on branch `work` (base `main`) that deleted `tests/t.rs`.
fn repo_without_its_test() -> (tempfile::TempDir, PathBuf) {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().canonicalize().unwrap().join("repo");
    fs::create_dir_all(root.join("tests")).unwrap();
    git(&root, &["init", "-q", "-b", "main"]);
    git(&root, &["config", "user.name", "t"]);
    git(&root, &["config", "user.email", "t@example.com"]);
    fs::write(
        root.join("tests/t.rs"),
        "#[test]\nfn t() {\n    assert!(true);\n}\n",
    )
    .unwrap();
    git(&root, &["add", "."]);
    git(&root, &["commit", "-q", "-m", "base"]);
    git(&root, &["checkout", "-q", "-b", "work"]);
    fs::remove_file(root.join("tests/t.rs")).unwrap();
    git(&root, &["commit", "-qam", "drop"]);
    (tmp, root)
}

fn task(allow: &str) -> String {
    task_touching(r#"["tests/**"]"#, allow)
}

fn task_touching(touch: &str, allow: &str) -> String {
    format!(
        "<!-- bosun:plan M1/T1 -->\n<!-- bosun:task {{\"item\":\"M1\",\"task\":\"T1\",\"touch\":{touch},\"allow\":{allow}}} -->"
    )
}

/// PR 5 with `body` that removed `tests/t.rs`, as GitHub lists it.
fn pr(fake: &FakeGithub, body: &str) {
    fake.pull(5, "drop seed", "work", body);
    fake.pull_file(5, "tests/t.rs", "removed", 0, 4, None);
}

fn check(fake: &FakeGithub, root: &Path) -> (i32, Value) {
    let out = bosun()
        .env("BOSUN_GITHUB_API", fake.url())
        .env_remove("BOSUN_PR_BODY")
        .env_remove("BOSUN_INTEGRITY_ALLOW")
        .arg("-C")
        .arg(root)
        .args(["test-integrity", "--base", "main", "--pr", "5"])
        .output()
        .unwrap();
    (out.status.code().unwrap(), one_object(&out.stdout))
}

#[test]
fn the_linked_task_allows_a_deleted_test() {
    let (_tmp, root) = repo_without_its_test();
    let fake = FakeGithub::start();
    pr(&fake, "Drop the seed.\n\nCloses #7");
    fake.issue(7, "M1/T1: drop", &[], &task(r#"["delete-test","bogus"]"#));
    let (code, v) = check(&fake, &root);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["task"], 7);
    assert_eq!(v["data"]["allow"], json!(["delete-test"]));
    assert_eq!(v["data"]["findings"][0]["allowed"], true);
}

#[test]
fn a_task_that_allows_nothing_refuses() {
    let (_tmp, root) = repo_without_its_test();
    let fake = FakeGithub::start();
    pr(&fake, "Closes #7");
    fake.issue(7, "M1/T1: drop", &[], &task("[]"));
    let (code, v) = check(&fake, &root);
    assert_eq!(code, 1, "{v}");
    assert_eq!(v["data"]["allow"], json!([]));
    let fix = v["error"]["fix"].as_str().unwrap();
    assert!(fix.contains("allow:"), "{fix}");
}

#[test]
fn a_pr_without_a_task_allows_nothing() {
    let (_tmp, root) = repo_without_its_test();
    let fake = FakeGithub::start();
    pr(&fake, "no link");
    let (code, v) = check(&fake, &root);
    assert_eq!(code, 1, "{v}");
    assert_eq!(v["data"]["task"], Value::Null);
}

#[test]
fn a_snapshot_change_points_at_the_pr_body() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().canonicalize().unwrap().join("repo");
    fs::create_dir_all(root.join("tests/snapshots")).unwrap();
    git(&root, &["init", "-q", "-b", "main"]);
    git(&root, &["config", "user.name", "t"]);
    git(&root, &["config", "user.email", "t@example.com"]);
    fs::write(root.join("README.md"), "x\n").unwrap();
    git(&root, &["add", "."]);
    git(&root, &["commit", "-q", "-m", "base"]);
    git(&root, &["checkout", "-q", "-b", "work"]);
    fs::write(root.join("tests/snapshots/new.snap"), "one\n").unwrap();
    git(&root, &["add", "."]);
    git(&root, &["commit", "-q", "-m", "snap"]);
    let fake = FakeGithub::start();
    fake.pull(5, "snap", "work", "Closes #7");
    fake.issue(7, "M1/T1", &[], &task("[]"));
    let (code, v) = check(&fake, &root);
    assert_eq!(code, 1, "{v}");
    let fix = v["error"]["fix"].as_str().unwrap();
    assert!(fix.contains("snapshot-change: <reason>"), "{fix}");
    assert!(!fix.contains("allow"), "{fix}");
}

#[test]
fn a_decoy_task_first_does_not_lend_its_allow() {
    let (_tmp, root) = repo_without_its_test();
    let fake = FakeGithub::start();
    // #99 allows deleting tests but does not touch tests/t.rs; #7, the real task, allows nothing.
    pr(&fake, "Closes #99\nCloses #7");
    fake.issue(
        99,
        "M2/T1",
        &[],
        &task_touching(r#"["src/**"]"#, r#"["delete-test"]"#),
    );
    fake.issue(7, "M1/T1", &[], &task("[]"));
    let (code, v) = check(&fake, &root);
    assert_eq!(code, 1, "{v}");
    assert_eq!(v["data"]["task"], 7);
    assert_eq!(v["data"]["allow"], json!([]));
}

#[test]
fn no_task_whose_touch_covers_the_pr_allows_nothing() {
    let (_tmp, root) = repo_without_its_test();
    let fake = FakeGithub::start();
    pr(&fake, "Closes #7");
    fake.issue(
        7,
        "M1/T1",
        &[],
        &task_touching(r#"["src/**"]"#, r#"["delete-test"]"#),
    );
    let (code, v) = check(&fake, &root);
    assert_eq!(code, 1, "{v}");
    assert_eq!(v["data"]["task"], Value::Null);
    assert_eq!(v["data"]["allow"], json!([]));
    let why = v["data"]["allow_refused"].as_str().unwrap();
    assert!(why.contains("#7") && why.contains("tests/t.rs"), "{why}");
    // An empty touch covers nothing either.
    let fake = FakeGithub::start();
    pr(&fake, "Closes #7");
    fake.issue(7, "M1/T1", &[], &task_touching("[]", r#"["delete-test"]"#));
    let (code, v) = check(&fake, &root);
    assert_eq!((code, &v["data"]["task"]), (1, &Value::Null), "{v}");
}

#[test]
fn a_cut_file_list_allows_nothing() {
    let (_tmp, root) = repo_without_its_test();
    let fake = FakeGithub::start();
    pr(&fake, "Closes #7");
    fake.pull_set(5, "changed_files", json!(3001));
    fake.issue(7, "M1/T1", &[], &task(r#"["delete-test"]"#));
    let (code, v) = check(&fake, &root);
    assert_eq!(code, 1, "{v}");
    assert_eq!(v["data"]["task"], Value::Null);
    assert!(
        v["data"]["allow_refused"]
            .as_str()
            .unwrap()
            .contains("file list")
    );
}

#[test]
fn an_added_file_outside_the_touch_borrows_nothing() {
    let (_tmp, root) = repo_without_its_test();
    let fake = FakeGithub::start();
    pr(&fake, "Closes #7");
    // The PR also adds a file outside the task's touch: added files count too.
    fake.pull_file(5, "src/new.rs", "added", 3, 0, None);
    fake.issue(7, "M1/T1", &[], &task(r#"["delete-test"]"#));
    let (code, v) = check(&fake, &root);
    assert_eq!(code, 1, "{v}");
    assert_eq!(v["data"]["task"], Value::Null);
    let why = v["data"]["allow_refused"].as_str().unwrap();
    assert!(why.contains("src/new.rs"), "{why}");
    // A rename counts by both names.
    let fake = FakeGithub::start();
    pr(&fake, "Closes #7");
    fake.pull_file(5, "src/moved.rs", "renamed", 0, 0, None);
    fake.renamed_from(5, "tests/old.rs");
    fake.issue(7, "M1/T1", &[], &task(r#"["delete-test"]"#));
    let (code, v) = check(&fake, &root);
    assert_eq!((code, &v["data"]["task"]), (1, &Value::Null), "{v}");
}
