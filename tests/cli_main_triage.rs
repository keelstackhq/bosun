mod common;

use std::path::Path;
use std::process::Command;

use common::{bosun, one_object};
use serde_json::Value;

fn git(dir: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(["-c", "user.name=t", "-c", "user.email=t@t"])
        .args(args)
        .output()
        .unwrap();
    assert!(out.status.success(), "git {args:?}: {out:?}");
    String::from_utf8(out.stdout).unwrap().trim().to_string()
}

/// An initial commit (returned sha) and 8 squash-like commits #101..#108.
/// PR #105 adds the file `broken`; later commits keep it.
fn history() -> (tempfile::TempDir, String) {
    let dir = tempfile::tempdir().unwrap();
    git(dir.path(), &["init", "-q", "-b", "main"]);
    git(
        dir.path(),
        &["commit", "-q", "--allow-empty", "-m", "initial"],
    );
    let good = git(dir.path(), &["rev-parse", "HEAD"]);
    for k in 1..=8 {
        std::fs::write(dir.path().join(format!("f{k}")), "x").unwrap();
        if k == 5 {
            std::fs::write(dir.path().join("broken"), "x").unwrap();
        }
        git(dir.path(), &["add", "-A"]);
        git(
            dir.path(),
            &["commit", "-q", "-m", &format!("feat: change {k} (#10{k})")],
        );
    }
    (dir, good)
}

fn triage(dir: &Path, good: &str, run: &str) -> (i32, Value, String) {
    let out = bosun()
        .arg("-C")
        .arg(dir)
        .args(["main-triage", "--good", good, "--run", run])
        .output()
        .unwrap();
    (
        out.status.code().unwrap(),
        one_object(&out.stdout),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

fn worktrees(dir: &Path) -> usize {
    git(dir, &["worktree", "list"]).lines().count()
}

#[test]
fn uc_m3_3_names_the_first_bad_pr_of_eight_merges() {
    let (dir, good) = history();
    let (code, v, _) = triage(dir.path(), &good, "test ! -f broken");
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["command"], "main-triage");
    assert_eq!(v["data"]["candidates"], 8);
    assert_eq!(v["data"]["first_bad"]["pr"], 105);
    assert_eq!(v["data"]["first_bad"]["subject"], "feat: change 5 (#105)");
    assert_eq!(
        v["data"]["first_bad"]["sha"],
        git(dir.path(), &["rev-parse", "HEAD~3"])
    );
    let runs = v["data"]["runs"].as_u64().unwrap();
    assert!((1..8).contains(&runs), "bisection, not linear: {runs}");
    assert_eq!(worktrees(dir.path()), 1);
}

#[test]
fn all_good_names_nothing() {
    let (dir, good) = history();
    let (code, v, _) = triage(dir.path(), &good, "true");
    assert_eq!(code, 0, "{v}");
    assert!(v["data"]["first_bad"].is_null());
    assert_eq!(v["data"]["runs"], 1);
    assert!(v["data"]["note"].as_str().unwrap().contains("passes"));
    assert_eq!(worktrees(dir.path()), 1);
}

#[test]
fn child_output_goes_to_stderr() {
    let (dir, good) = history();
    let (_, _, err) = triage(dir.path(), &good, "echo probe-output; test ! -f broken");
    assert!(err.contains("probe-output"));
}

#[test]
fn no_candidates_is_a_usage_error() {
    let (dir, _) = history();
    let (code, v, _) = triage(dir.path(), "HEAD", "true");
    assert_eq!(code, 2, "{v}");
    assert!(!v["error"]["fix"].as_str().unwrap().is_empty());
}

#[test]
fn unknown_rev_is_an_error_with_a_fix() {
    let (dir, _) = history();
    let (code, v, _) = triage(dir.path(), "nope", "true");
    assert_eq!(code, 1, "{v}");
    assert!(!v["error"]["fix"].as_str().unwrap().is_empty());
}

#[test]
fn run_failing_at_good_too_needs_a_human() {
    let (dir, good) = history();
    let (code, v, _) = triage(dir.path(), &good, "false");
    assert_eq!(code, 5, "{v}");
    assert!(
        v["error"]["message"]
            .as_str()
            .unwrap()
            .contains("fails at --good too")
    );
    assert!(!v["error"]["fix"].as_str().unwrap().is_empty());
    assert_eq!(worktrees(dir.path()), 1);
}

#[test]
fn bad_rev_older_than_head_limits_the_range() {
    let (dir, good) = history();
    let bad = git(dir.path(), &["rev-parse", "HEAD~2"]);
    let out = bosun()
        .arg("-C")
        .arg(dir.path())
        .args([
            "main-triage",
            "--good",
            &good,
            "--bad",
            "HEAD~2",
            "--run",
            "test ! -f broken",
        ])
        .output()
        .unwrap();
    let v = one_object(&out.stdout);
    assert_eq!(out.status.code(), Some(0), "{v}");
    assert_eq!(v["data"]["candidates"], 6);
    assert_eq!(v["data"]["bad"], bad);
    assert_eq!(v["data"]["first_bad"]["pr"], 105);
}

#[test]
fn default_run_is_gate_test_all() {
    let (dir, good) = history();
    let out = bosun()
        .arg("-C")
        .arg(dir.path())
        .args(["main-triage", "--good", &good])
        .output()
        .unwrap();
    let v = one_object(&out.stdout);
    let run = v["data"]["run"].as_str().unwrap();
    assert!(run.ends_with("gate test --all"), "{v}");
    assert_eq!(out.status.code(), Some(5), "{v}");
    assert_ne!(out.status.code(), Some(0), "{v}");
    assert_eq!(worktrees(dir.path()), 1);
}

#[test]
fn rev_starting_with_dash_is_a_usage_error() {
    let (dir, _) = history();
    let (code, v, _) = triage(dir.path(), "--output=x", "true");
    assert_eq!(code, 2, "{v}");
    assert!(!v["error"]["fix"].as_str().unwrap().is_empty());
}
