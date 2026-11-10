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
    let (code, v, _) = run_env(repo, session, args, &[("CLAUDECODE", "1")]);
    (code, v)
}

/// As [`run`] with `env` set (CI variables cleared first); also returns stderr.
fn run_env(
    repo: &Path,
    session: &str,
    args: &[&str],
    env: &[(&str, &str)],
) -> (i32, Value, String) {
    let mut cmd = bosun();
    cmd.current_dir(repo)
        .env("BOSUN_SESSION", session)
        .env_remove("CLAUDECODE")
        .env_remove("CI")
        .args(args);
    for (k, v) in env {
        cmd.env(k, v);
    }
    let out = cmd.output().unwrap();
    let v = common::one_object(&out.stdout);
    let err = String::from_utf8_lossy(&out.stderr).to_string();
    (out.status.code().unwrap(), v, err)
}

fn hook(repo: &Path, input: &Value) {
    hook_as(repo, "s", input);
}

fn hook_as(repo: &Path, session: &str, input: &Value) {
    let out = bosun()
        .current_dir(repo)
        .env("BOSUN_SESSION", session)
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

#[test]
fn the_third_run_warns_and_a_refusal_is_sticky() {
    let tmp = repo();
    run(tmp.path(), "s", CALL);
    run(tmp.path(), "s", CALL);
    let (_, _, err) = run_env(tmp.path(), "s", CALL, &[("CLAUDECODE", "1")]);
    assert!(
        err.contains("cli-repeat: this exact command ran 3 times"),
        "{err}"
    );
    run(tmp.path(), "s", CALL);
    let (code, v) = run(tmp.path(), "s", CALL);
    assert_eq!(code, 6);
    let fix = v["error"]["fix"].as_str().unwrap();
    assert!(fix.starts_with("cli-repeat:"), "{fix}");
    // The exit-6 refusal leaves a sticky marker for the hook.
    let refusals = tmp.path().join(".git/bosun/refusals");
    assert_eq!(std::fs::read_dir(refusals).unwrap().count(), 1);
}

#[test]
fn outside_claude_code_or_in_ci_nothing_is_counted() {
    let tmp = repo();
    for _ in 0..6 {
        assert_ne!(run_env(tmp.path(), "s", CALL, &[]).0, 6);
    }
    for _ in 0..6 {
        let env = [("CLAUDECODE", "1"), ("CI", "true")];
        assert_ne!(run_env(tmp.path(), "s", CALL, &env).0, 6);
    }
    assert!(!tmp.path().join(".git/bosun/cli-repeat").exists());
}

#[test]
fn polls_with_global_flags_and_bash_polls_leave_the_count() {
    let tmp = repo();
    let dir = tmp.path().to_str().unwrap();
    for _ in 0..4 {
        run(tmp.path(), "s", CALL);
    }
    for _ in 0..6 {
        assert_ne!(run(tmp.path(), "s", &["-C", dir, "--json", "status"]).0, 6);
    }
    // A Bash poll in the hook does not reset; the 5th call is still refused.
    hook(
        tmp.path(),
        &json!({"tool_name": "Bash", "tool_input": {"command": "git status"}}),
    );
    assert_eq!(run(tmp.path(), "s", CALL).0, 6);
}

#[test]
fn the_callers_tool_call_resets_its_task_sessions_count() {
    let tmp = repo();
    for _ in 0..4 {
        run(tmp.path(), "s/7", CALL);
    }
    hook_as(
        tmp.path(),
        "s",
        &json!({"tool_name": "Read", "tool_input": {"file_path": "x"}}),
    );
    assert_ne!(run(tmp.path(), "s/7", CALL).0, 6);
}

#[test]
fn outside_a_git_repo_nothing_is_counted() {
    let dir = tempfile::tempdir().unwrap();
    for _ in 0..6 {
        assert_ne!(run(dir.path(), "s", CALL).0, 6);
    }
}

#[test]
fn a_denied_tool_call_does_not_reset_the_count() {
    let tmp = repo();
    for _ in 0..4 {
        run(tmp.path(), "s", CALL);
    }
    // The merge-lock denies this call: it never ran, so the loop goes on counting.
    hook(
        tmp.path(),
        &json!({"tool_name": "Bash", "tool_input": {"command": "gh pr merge 5 --squash"}}),
    );
    assert_eq!(run(tmp.path(), "s", CALL).0, 6);
}
