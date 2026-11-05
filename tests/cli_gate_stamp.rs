//! A passing `bosun gate` outside CI leaves a stamp for the tree it ran on; `bosun pr --ready`
//! reads them. test-integrity is built in, so these tests need no toolchain.

mod common;

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use common::{bosun, one_object};
use serde_json::Value;

const TWO_ASSERTS: &str = "#[test]\nfn t() {\n    assert!(true);\n    assert_eq!(1, 1);\n}\n";

fn git(root: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .current_dir(root)
        .args(args)
        .output()
        .expect("git");
    assert!(out.status.success(), "git {args:?}: {out:?}");
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

fn put(root: &Path, rel: &str, text: &str) {
    let p = root.join(rel);
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    fs::write(p, text).unwrap();
}

/// A repo on branch `work` (base `main`).
fn repo() -> (tempfile::TempDir, PathBuf) {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().canonicalize().unwrap().join("repo");
    fs::create_dir_all(&root).unwrap();
    git(&root, &["init", "-q", "-b", "main"]);
    git(&root, &["config", "user.name", "t"]);
    git(&root, &["config", "user.email", "t@example.com"]);
    put(&root, "tests/t.rs", TWO_ASSERTS);
    put(&root, "src/lib.rs", "pub fn f() {}\n");
    git(&root, &["add", "."]);
    git(&root, &["commit", "-q", "-m", "base"]);
    git(&root, &["checkout", "-q", "-b", "work"]);
    (tmp, root)
}

fn gate(root: &Path, env: &[(&str, &str)]) -> (i32, Value) {
    let mut cmd = bosun();
    cmd.env_remove("GITHUB_BASE_REF")
        .env_remove("BOSUN_PR_BODY")
        .env_remove("BOSUN_INTEGRITY_ALLOW");
    for (k, v) in env {
        cmd.env(k, v);
    }
    let out = cmd
        .arg("-C")
        .arg(root)
        .args(["gate", "test-integrity", "--base", "main"])
        .output()
        .unwrap();
    (out.status.code().unwrap(), one_object(&out.stdout))
}

fn stamp_path(root: &Path, tree: &str) -> PathBuf {
    let common = git(
        root,
        &["rev-parse", "--path-format=absolute", "--git-common-dir"],
    );
    Path::new(&common)
        .join("bosun/gates")
        .join(tree)
        .join("test-integrity")
}

#[test]
fn a_passing_gate_stamps_the_tree_at_head() {
    let (_tmp, root) = repo();
    put(&root, "src/lib.rs", "pub fn f() { let _ = 1; }\n");
    git(&root, &["add", "."]);
    git(&root, &["commit", "-q", "-m", "change"]);
    let (code, v) = gate(&root, &[]);
    assert_eq!(code, 0, "{v}");
    let tree = git(&root, &["rev-parse", "HEAD^{tree}"]);
    let stamp = stamp_path(&root, &tree);
    assert!(stamp.is_file(), "{stamp:?}");
    assert_eq!(v["data"]["stamp"], stamp.to_str().unwrap());
}

#[test]
fn uncommitted_tracked_work_stamps_its_own_tree_not_head() {
    let (_tmp, root) = repo();
    put(&root, "src/lib.rs", "pub fn f() { let _ = 2; }\n");
    let (code, v) = gate(&root, &[]);
    assert_eq!(code, 0, "{v}");
    let head_tree = git(&root, &["rev-parse", "HEAD^{tree}"]);
    assert!(!stamp_path(&root, &head_tree).exists());
    let stamp = v["data"]["stamp"].as_str().unwrap();
    assert!(Path::new(stamp).is_file(), "{stamp}");
    assert!(!stamp.contains(&head_tree), "{stamp}");
}

#[test]
fn a_failing_gate_leaves_no_stamp() {
    let (_tmp, root) = repo();
    put(
        &root,
        "tests/t.rs",
        "#[test]\nfn t() {\n    assert!(true);\n}\n",
    );
    git(&root, &["add", "."]);
    git(&root, &["commit", "-q", "-m", "drop an assertion"]);
    let (code, v) = gate(&root, &[]);
    assert_eq!(code, 1, "{v}");
    let tree = git(&root, &["rev-parse", "HEAD^{tree}"]);
    assert!(!stamp_path(&root, &tree).exists());
}

#[test]
fn ci_runs_do_not_stamp() {
    let (_tmp, root) = repo();
    let (code, v) = gate(&root, &[("GITHUB_ACTIONS", "true")]);
    assert_eq!(code, 0, "{v}");
    assert!(v["data"].get("stamp").is_none_or(Value::is_null), "{v}");
    let tree = git(&root, &["rev-parse", "HEAD^{tree}"]);
    assert!(!stamp_path(&root, &tree).exists());
}
