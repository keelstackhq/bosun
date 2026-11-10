//! `bosun hook pre-compact` and `bosun hook session-start` with source `compact`: the state
//! block that survives compaction (spec §4).

mod common;

use std::path::Path;
use std::process::Command;

use common::fake::FakeGithub;
use common::{TOKEN_CANARY, bosun, lease_note};
use serde_json::{Value, json};

const T0: &str = "2026-10-22T10:00:00Z";
const LIVE: &str = "2026-10-22T10:20:00Z";

fn git(dir: &Path, args: &[&str]) {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .output()
        .expect("git");
    assert!(out.status.success(), "git {args:?}: {out:?}");
}

/// A repo on branch `feat/x` with one commit.
fn repo() -> tempfile::TempDir {
    let tmp = tempfile::tempdir().unwrap();
    let d = tmp.path();
    git(d, &["init", "-q", "-b", "feat/x"]);
    git(
        d,
        &[
            "remote",
            "add",
            "origin",
            "https://github.com/acme/widget.git",
        ],
    );
    git(
        d,
        &[
            "-c",
            "user.name=t",
            "-c",
            "user.email=t@t",
            "commit",
            "-q",
            "--allow-empty",
            "-m",
            "c",
        ],
    );
    tmp
}

fn leased(fake: &FakeGithub) {
    fake.issue(4, "other", &["bosun:state/claimed"], "x");
    fake.comment(4, &lease_note("session_other", LIVE, "src/core/**"));
    fake.issue(5, "mine", &["bosun:state/claimed"], "x");
    fake.comment(5, &lease_note("session_me", LIVE, "docs/**"));
}

struct Out {
    code: i32,
    stdout: String,
    stderr: String,
}

fn hook(fake: Option<&FakeGithub>, repo: &Path, sub: &str, input: &Value) -> Out {
    let mut cmd = bosun();
    cmd.current_dir(repo)
        .env("BOSUN_SESSION", "session_me")
        .env("BOSUN_ORIGIN", "spawned")
        .env("BOSUN_NOW", T0)
        .args(["hook", sub])
        .write_stdin(input.to_string());
    if let Some(f) = fake {
        cmd.env("BOSUN_GITHUB_API", f.url());
    }
    let out = cmd.output().unwrap();
    let o = Out {
        code: out.status.code().unwrap(),
        stdout: String::from_utf8_lossy(&out.stdout).into(),
        stderr: String::from_utf8_lossy(&out.stderr).into(),
    };
    assert!(!format!("{}{}", o.stdout, o.stderr).contains(TOKEN_CANARY));
    o
}

fn input(repo: &Path, event: &str) -> Value {
    json!({"hook_event_name": event, "trigger": "auto", "source": "compact",
        "cwd": repo.to_str().unwrap()})
}

fn head(repo: &Path) -> String {
    let out = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(["rev-parse", "HEAD"])
        .output()
        .unwrap();
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

#[test]
fn pre_compact_prints_the_state_block_as_plain_text() {
    let (fake, tmp) = (FakeGithub::start(), repo());
    leased(&fake);
    fake.pull(12, "my task", "feat/x", "Closes #5");
    let o = hook(
        Some(&fake),
        tmp.path(),
        "pre-compact",
        &input(tmp.path(), "PreCompact"),
    );
    assert_eq!(o.code, 0, "{}", o.stderr);
    assert!(
        serde_json::from_str::<Value>(&o.stdout).is_err(),
        "plain text, never a decision"
    );
    let short = &head(tmp.path())[..12];
    for needle in [
        "session_me",
        "#5",
        "docs/**",
        "feat/x",
        "PR #12",
        "sha12",
        short,
        "Next:",
    ] {
        assert!(o.stdout.contains(needle), "{needle} missing: {}", o.stdout);
    }
    assert!(!o.stdout.contains("#4"), "only own leases: {}", o.stdout);
}

#[test]
fn pre_compact_without_github_still_prints_and_never_blocks() {
    let tmp = repo();
    let o = hook(
        None,
        tmp.path(),
        "pre-compact",
        &input(tmp.path(), "PreCompact"),
    );
    assert_eq!(o.code, 0, "{}", o.stderr);
    for needle in ["session_me", "feat/x", "Next:"] {
        assert!(o.stdout.contains(needle), "{needle} missing: {}", o.stdout);
    }
}

#[test]
fn pre_compact_names_the_last_refusal() {
    let tmp = repo();
    let store = tmp.path().join(".git/bosun/refusals");
    std::fs::create_dir_all(&store).unwrap();
    let r = json!({"session": "session_me", "command": "pr", "exit_code": 5,
        "fix": "ask the owner", "at": "2026-10-22T09:59:00Z"});
    std::fs::write(store.join("a.json"), r.to_string()).unwrap();
    let o = hook(
        None,
        tmp.path(),
        "pre-compact",
        &input(tmp.path(), "PreCompact"),
    );
    assert!(o.stdout.contains("bosun pr"), "{}", o.stdout);
    assert!(o.stdout.contains("ask the owner"), "{}", o.stdout);
}

#[test]
fn malformed_input_still_allows_compaction() {
    let tmp = repo();
    let mut cmd = bosun();
    cmd.current_dir(tmp.path())
        .env("BOSUN_NOW", T0)
        .args(["hook", "pre-compact"])
        .write_stdin("not json");
    let out = cmd.output().unwrap();
    assert_eq!(out.status.code(), Some(0));
}

#[test]
fn session_start_after_compaction_adds_the_state_block() {
    let (fake, tmp) = (FakeGithub::start(), repo());
    leased(&fake);
    fake.pull(12, "my task", "feat/x", "Closes #5");
    let o = hook(
        Some(&fake),
        tmp.path(),
        "session-start",
        &input(tmp.path(), "SessionStart"),
    );
    assert_eq!(o.code, 0, "{}", o.stderr);
    let v: Value = common::one_object(o.stdout.as_bytes());
    let text = v["hookSpecificOutput"]["additionalContext"]
        .as_str()
        .unwrap();
    for needle in ["bosun state", "PR #12", "docs/**", "Next:"] {
        assert!(text.contains(needle), "{needle} missing: {text}");
    }
}

#[test]
fn session_start_on_startup_has_no_state_block() {
    let (fake, tmp) = (FakeGithub::start(), repo());
    leased(&fake);
    let mut i = input(tmp.path(), "SessionStart");
    i["source"] = json!("startup");
    let o = hook(Some(&fake), tmp.path(), "session-start", &i);
    let v: Value = common::one_object(o.stdout.as_bytes());
    let text = v["hookSpecificOutput"]["additionalContext"]
        .as_str()
        .unwrap();
    assert!(!text.contains("bosun state"), "{text}");
}
