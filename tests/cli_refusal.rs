//! Sticky refusals (spec §4): a bosun exit 4/5/6 (3 on claim) leaves a per-session marker the
//! PreToolUse hook injects on the next tool call; halted denies writes until `bosun status`
//! shows the halt lifted; a later success of the same command clears it.
mod common;

use std::path::Path;
use std::process::Command;

use common::fake::FakeGithub;
use common::{bosun, lease_note, one_object};
use serde_json::{Value, json};

const NOW: &str = "2026-10-22T10:00:00Z";
const LIVE: &str = "2026-10-22T10:20:00Z";

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
    git(
        tmp.path(),
        &[
            "remote",
            "add",
            "origin",
            "https://github.com/acme/widget.git",
        ],
    );
    tmp
}

fn cmd(fake: &FakeGithub, repo: &Path, args: &[&str]) -> (i32, Value) {
    let out = bosun()
        .current_dir(repo)
        .env("BOSUN_GITHUB_API", fake.url())
        .env("BOSUN_SESSION", "session_me")
        .env("BOSUN_NOW", NOW)
        .args(args)
        .output()
        .unwrap();
    (out.status.code().unwrap(), one_object(&out.stdout))
}

/// The hook's stdout as JSON (`Null` when it printed nothing). The hook process sees only
/// the caller id, as Claude Code starts it; the marker of `session_me/7` is still its own.
fn hook(repo: &Path, session: &str, input: &Value) -> Value {
    let out = bosun()
        .current_dir(repo)
        .env("BOSUN_SESSION", session)
        .env("BOSUN_NOW", NOW)
        .args(["hook", "pre-tool-use"])
        .write_stdin(input.to_string())
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(0), "{out:?}");
    let text = String::from_utf8_lossy(&out.stdout);
    let text = text.trim();
    if text.is_empty() {
        Value::Null
    } else {
        serde_json::from_str(text).unwrap()
    }
}

fn read(repo: &Path) -> Value {
    json!({"tool_name": "Read", "tool_input": {"file_path": repo.join("x").to_str()}})
}

/// A write outside the repo: no other lock judges it.
fn write_outside() -> Value {
    json!({"tool_name": "Write", "tool_input": {"file_path": "/tmp/bosun-refusal-test.txt",
        "content": "x"}})
}

fn markers(repo: &Path) -> usize {
    std::fs::read_dir(repo.join(".git/bosun/refusals")).map_or(0, |d| {
        d.filter(|e| {
            e.as_ref()
                .is_ok_and(|e| e.path().extension().is_some_and(|x| x == "json"))
        })
        .count()
    })
}

#[test]
fn halted_marker_is_injected_denies_writes_and_clears_on_status() {
    let fake = FakeGithub::start();
    fake.variable("BOSUN_HALT", "1");
    fake.issue(7, "task", &["bosun:state/ready"], "x");
    let tmp = repo();
    let (code, v) = cmd(&fake, tmp.path(), &["claim", "7"]);
    assert_eq!(code, 4, "{v}");
    assert_eq!(markers(tmp.path()), 1);

    // Any tool call sees the fix; a write is denied while halted.
    let out = hook(tmp.path(), "session_me", &read(tmp.path()));
    let ctx = out["hookSpecificOutput"]["additionalContext"]
        .as_str()
        .unwrap_or_default();
    assert!(ctx.contains("bosun claim"), "{out}");
    assert!(ctx.contains("BOSUN_HALT=0"), "{out}");
    let out = hook(tmp.path(), "session_me", &write_outside());
    assert_eq!(out["hookSpecificOutput"]["permissionDecision"], "deny");
    let why = out["hookSpecificOutput"]["permissionDecisionReason"]
        .as_str()
        .unwrap();
    assert!(why.contains("bosun status"), "{why}");
    // Another session's hook sees nothing.
    assert_eq!(
        hook(tmp.path(), "session_other", &write_outside()),
        Value::Null
    );

    // Status while still halted keeps it; once lifted it clears.
    let (code, _) = cmd(&fake, tmp.path(), &["status"]);
    assert_eq!(code, 0);
    assert_eq!(markers(tmp.path()), 1);
    fake.variable("BOSUN_HALT", "0");
    let (code, v) = cmd(&fake, tmp.path(), &["status"]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(markers(tmp.path()), 0);
    assert_eq!(
        hook(tmp.path(), "session_me", &write_outside()),
        Value::Null
    );
}

#[test]
fn conflict_on_claim_is_sticky_and_a_later_claim_clears_it() {
    let fake = FakeGithub::start();
    fake.issue(4, "held", &["bosun:state/claimed"], "x");
    fake.comment(4, &lease_note("session_other", LIVE, "src/**"));
    fake.issue(5, "free", &["bosun:state/ready"], "x");
    let tmp = repo();
    let (code, v) = cmd(&fake, tmp.path(), &["claim", "4", "--touch", "src/**"]);
    assert_eq!(code, 3, "{v}");
    assert_eq!(markers(tmp.path()), 1);

    // Injected once on the next call of any kind; not a deny.
    let out = hook(tmp.path(), "session_me", &read(tmp.path()));
    let hso = &out["hookSpecificOutput"];
    assert!(hso["permissionDecision"].is_null(), "{out}");
    let ctx = hso["additionalContext"].as_str().unwrap();
    assert!(ctx.contains("exit 3"), "{ctx}");
    assert!(ctx.contains("bosun claim"), "{ctx}");
    assert_eq!(
        hook(tmp.path(), "session_me", &write_outside()),
        Value::Null
    );

    let (code, v) = cmd(&fake, tmp.path(), &["claim", "5", "--touch", "docs/**"]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(markers(tmp.path()), 0);
}

#[test]
fn a_task_session_marker_reaches_the_callers_hook() {
    let fake = FakeGithub::start();
    fake.variable("BOSUN_HALT", "1");
    fake.issue(7, "task", &["bosun:state/ready"], "x");
    let tmp = repo();
    let out = bosun()
        .current_dir(tmp.path())
        .env("BOSUN_GITHUB_API", fake.url())
        .env("BOSUN_SESSION", "session_me/7")
        .args(["claim", "7"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(4));
    let out = hook(tmp.path(), "session_me", &write_outside());
    assert_eq!(out["hookSpecificOutput"]["permissionDecision"], "deny");
    // The caller's own `bosun status` clears its task session's marker once the halt lifts.
    fake.variable("BOSUN_HALT", "0");
    let (code, v) = cmd(&fake, tmp.path(), &["status"]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(markers(tmp.path()), 0);
}

#[test]
fn a_main_red_halt_is_shown_but_leaves_writes_open() {
    let fake = FakeGithub::start();
    fake.issue(9, "main red", &["bosun:main-red"], "x");
    fake.issue(7, "task", &["bosun:state/ready"], "x");
    let tmp = repo();
    let (code, v) = cmd(&fake, tmp.path(), &["claim", "7"]);
    assert_eq!(code, 4, "{v}");
    assert_eq!(markers(tmp.path()), 1);
    let out = hook(tmp.path(), "session_me", &write_outside());
    let hso = &out["hookSpecificOutput"];
    assert!(hso["permissionDecision"].is_null(), "{out}");
    let ctx = hso["additionalContext"].as_str().unwrap();
    assert!(
        ctx.contains("mainfix") && !ctx.contains("halt lifted"),
        "{ctx}"
    );
}

#[test]
fn a_lifted_kill_switch_unlocks_writes_while_main_is_red() {
    let fake = FakeGithub::start();
    fake.variable("BOSUN_HALT", "1");
    fake.issue(9, "main red", &["bosun:main-red"], "x");
    fake.issue(7, "task", &[], "x");
    let tmp = repo();
    let (code, v) = cmd(&fake, tmp.path(), &["claim", "7"]);
    assert_eq!(code, 4, "{v}");
    let out = hook(tmp.path(), "session_me", &write_outside());
    assert_eq!(out["hookSpecificOutput"]["permissionDecision"], "deny");
    fake.variable("BOSUN_HALT", "0");
    let (code, v) = cmd(&fake, tmp.path(), &["status"]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["main_red"], 9, "{v}");
    assert_eq!(markers(tmp.path()), 0);
    assert_eq!(
        hook(tmp.path(), "session_me", &write_outside()),
        Value::Null
    );
}

/// Without `BOSUN_SESSION` both sides fall back to `local-<host>`; a refusal shown once and
/// cleared is gone for good, and the same refusal again is shown again.
#[test]
fn a_local_host_marker_is_shown_once_and_cleared_for_good() {
    let fake = FakeGithub::start();
    fake.issue(4, "held", &[], "x");
    fake.comment(4, &lease_note("session_other", LIVE, "src/**"));
    let tmp = repo();
    let run = |args: &[&str]| {
        let out = bosun()
            .current_dir(tmp.path())
            .env("BOSUN_GITHUB_API", fake.url())
            .env_remove("BOSUN_SESSION")
            .env("HOSTNAME", "box")
            .args(args)
            .output()
            .unwrap();
        out.status.code().unwrap()
    };
    let local_hook = || {
        let out = bosun()
            .current_dir(tmp.path())
            .env_remove("BOSUN_SESSION")
            .env("HOSTNAME", "box")
            .args(["hook", "pre-tool-use"])
            .write_stdin(read(tmp.path()).to_string())
            .output()
            .unwrap();
        String::from_utf8_lossy(&out.stdout).trim().to_string()
    };
    assert_eq!(run(&["claim", "4", "--touch", "src/**"]), 3);
    assert!(local_hook().contains("bosun refusal"));
    assert_eq!(local_hook(), "", "shown once");
    assert_eq!(run(&["claim", "4", "--touch", "src/**"]), 3);
    assert!(
        local_hook().contains("bosun refusal"),
        "a new refusal shows again"
    );
    fake.issue(5, "free", &["bosun:state/ready"], "x");
    assert_eq!(run(&["claim", "5", "--touch", "docs/**"]), 0);
    assert_eq!(markers(tmp.path()), 0);
    assert_eq!(local_hook(), "");
}
