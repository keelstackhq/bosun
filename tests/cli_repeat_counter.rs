//! The CLI's own repeat counter (spec §4): a loop of the same bosun call inside one script,
//! which the PreToolUse hook sees as one tool call, trips the same limit as the repeat-lock.
mod common;

use std::path::Path;
use std::process::Command;

use common::bosun;
use serde_json::{Value, json};

fn git(dir: &Path, args: &[&str]) {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .output()
        .unwrap();
    assert!(out.status.success(), "git {args:?}: {out:?}");
}

fn repo() -> tempfile::TempDir {
    let tmp = tempfile::tempdir().unwrap();
    git(tmp.path(), &["init", "-q", "-b", "main"]);
    tmp
}

/// `bosun <args>` as `session` in `repo`; (exit code, envelope).
fn run(repo: &Path, session: &str, args: &[&str]) -> (i32, Value) {
    let out = bosun()
        .current_dir(repo)
        .env("BOSUN_SESSION", session)
        .args(args)
        .output()
        .unwrap();
    let v = common::one_object(&out.stdout);
    (out.status.code().unwrap(), v)
}

fn hook(repo: &Path, input: &Value) {
    let out = bosun()
        .current_dir(repo)
        .env("BOSUN_SESSION", "s")
        .args(["hook", "pre-tool-use"])
        .write_stdin(input.to_string())
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(0));
}

// `bosun release` with no GitHub reachable exits 1 every time: a command an agent might retry.
const CALL: &[&str] = &["release", "999"];

#[test]
fn a_loop_of_the_same_call_is_refused_from_the_fifth() {
    let tmp = repo();
    for i in 1..=4 {
        let (code, v) = run(tmp.path(), "s", CALL);
        assert_ne!(code, 6, "call {i}: {v}");
    }
    let (code, v) = run(tmp.path(), "s", CALL);
    assert_eq!(code, 6, "{v}");
    assert_eq!(v["error"]["kind"], "limit");
    let fix = v["error"]["fix"].as_str().unwrap();
    assert!(fix.contains("change the approach"), "{fix}");
    // Still refused while the loop goes on.
    assert_eq!(run(tmp.path(), "s", CALL).0, 6);
}

#[test]
fn another_call_or_a_tool_call_resets_the_count() {
    let tmp = repo();
    for _ in 0..4 {
        run(tmp.path(), "s", CALL);
    }
    run(tmp.path(), "s", &["release", "998"]);
    for _ in 0..4 {
        assert_ne!(run(tmp.path(), "s", CALL).0, 6);
    }
    // The next tool call (the agent acts between runs) starts the count again.
    hook(
        tmp.path(),
        &json!({"tool_name": "Read", "tool_input": {"file_path": "x"}}),
    );
    for _ in 0..4 {
        assert_ne!(run(tmp.path(), "s", CALL).0, 6);
    }
}

#[test]
fn polls_and_other_sessions_are_not_counted() {
    let tmp = repo();
    for _ in 0..4 {
        run(tmp.path(), "s", CALL);
    }
    // Another session (a task session of a sibling) counts apart.
    assert_ne!(run(tmp.path(), "t", CALL).0, 6);
    // A poll leaves the count as is.
    for _ in 0..6 {
        assert_ne!(run(tmp.path(), "s", &["status"]).0, 6);
    }
    assert_eq!(run(tmp.path(), "s", CALL).0, 6);
}
