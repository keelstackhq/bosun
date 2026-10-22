mod common;

use std::path::Path;
use std::process::Command;

use common::fake::FakeGithub;
use common::{TOKEN_CANARY, bosun, lease_note};
use serde_json::{Value, json};

const T0: &str = "2026-10-22T10:00:00Z";
const T1: &str = "2026-10-22T10:00:30Z";
const T2: &str = "2026-10-22T10:02:00Z";
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

fn repo() -> tempfile::TempDir {
    let tmp = tempfile::tempdir().unwrap();
    git(tmp.path(), &["init", "-q", "-b", "main"]);
    let url = "https://github.com/acme/widget.git";
    git(tmp.path(), &["remote", "add", "origin", url]);
    tmp
}

/// Issue 4 holds a live lease of `session_other` on `src/core/**`; issue 5 is `session_me`'s.
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

fn hook(fake: Option<&FakeGithub>, repo: &Path, now: &str, sub: &str, input: &Value) -> Out {
    let mut cmd = bosun();
    cmd.current_dir(repo)
        .env("BOSUN_SESSION", "session_me")
        .env("BOSUN_NOW", now)
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

fn call(repo: &Path, tool: &str, input: Value) -> Value {
    json!({"session_id":"s","tool_name":tool,"tool_input":input,"cwd":repo.to_str().unwrap()})
}

fn pre(fake: &FakeGithub, repo: &Path, tool: &str, input: Value) -> Out {
    hook(
        Some(fake),
        repo,
        T0,
        "pre-tool-use",
        &call(repo, tool, input),
    )
}

fn denied(o: &Out) -> String {
    assert_eq!(o.code, 0, "{}", o.stderr);
    let v: Value = serde_json::from_str(&o.stdout).expect("one JSON object");
    assert_eq!(v.as_object().unwrap().len(), 1);
    let h = &v["hookSpecificOutput"];
    assert_eq!(h.as_object().unwrap().len(), 3);
    assert_eq!(h["hookEventName"], "PreToolUse");
    assert_eq!(h["permissionDecision"], "deny");
    let reason = h["permissionDecisionReason"].as_str().unwrap();
    assert!(!reason.is_empty());
    reason.to_string()
}

fn allowed(o: &Out) {
    assert_eq!(o.code, 0, "{}", o.stderr);
    assert_eq!(o.stdout, "", "allow prints nothing");
}

#[test]
fn edit_in_another_sessions_lease_is_denied() {
    let (fake, tmp) = (FakeGithub::start(), repo());
    leased(&fake);
    let abs = tmp.path().join("src/core/guard.rs");
    let input = json!({"file_path": abs.to_str().unwrap()});
    let reason = denied(&pre(&fake, tmp.path(), "Edit", input));
    assert!(
        reason.contains("session_other") && reason.contains("#4"),
        "{reason}"
    );
    let input = json!({"file_path": "src/core/x.rs"});
    denied(&pre(&fake, tmp.path(), "Write", input));
}

#[test]
fn own_lease_unrelated_and_outside_paths_are_allowed() {
    let (fake, tmp) = (FakeGithub::start(), repo());
    leased(&fake);
    for p in ["docs/spec.md", "src/cli/mod.rs", "/etc/hosts"] {
        allowed(&pre(&fake, tmp.path(), "Edit", json!({"file_path": p})));
    }
    let read = json!({"file_path": "src/core/a.rs"});
    allowed(&pre(&fake, tmp.path(), "Read", read));
}

#[test]
fn label_and_merge_locks_deny_without_the_network() {
    let (fake, tmp) = (FakeGithub::start(), repo());
    let bash = |c: &str| pre(&fake, tmp.path(), "Bash", json!({"command": c}));
    denied(&bash("gh issue edit 4 --add-label bosun:state/merged"));
    denied(&bash("gh pr merge 3"));
    denied(&bash("git push --force"));
    allowed(&bash("bosun release 4"));
    allowed(&bash("ls"));
    let merge = pre(
        &fake,
        tmp.path(),
        "mcp__github__merge_pull_request",
        json!({"pullNumber": 3}),
    );
    denied(&merge);
    let label = json!({"labels": ["bosun:state/merged"]});
    denied(&pre(&fake, tmp.path(), "mcp__github__issue_write", label));
    assert!(
        fake.requests().is_empty(),
        "label and merge rules skip GitHub"
    );
}

#[test]
fn second_call_within_the_cache_age_makes_no_request() {
    let (fake, tmp) = (FakeGithub::start(), repo());
    leased(&fake);
    let edit = json!({"file_path": "src/core/a.rs"});
    denied(&pre(&fake, tmp.path(), "Edit", edit.clone()));
    let first = fake.requests().len();
    assert!(first > 0);
    let c = call(tmp.path(), "Edit", edit);
    denied(&hook(Some(&fake), tmp.path(), T1, "pre-tool-use", &c));
    assert_eq!(fake.requests().len(), first, "served from the cache");
    hook(Some(&fake), tmp.path(), T2, "pre-tool-use", &c);
    assert!(fake.requests().len() > first, "older than 60 s: refreshed");
    assert!(tmp.path().join(".git/bosun/leases.json").exists());
}

#[test]
fn github_down_without_a_cache_allows_with_a_warning() {
    let tmp = repo();
    let c = call(tmp.path(), "Edit", json!({"file_path": "src/core/a.rs"}));
    let o = hook(None, tmp.path(), T0, "pre-tool-use", &c);
    allowed(&o);
    assert!(o.stderr.contains("warning"), "{}", o.stderr);
}

#[test]
fn stale_cache_is_used_while_github_is_down() {
    let (fake, tmp) = (FakeGithub::start(), repo());
    leased(&fake);
    let edit = json!({"file_path": "src/core/a.rs"});
    denied(&pre(&fake, tmp.path(), "Edit", edit.clone()));
    let c = call(tmp.path(), "Edit", edit);
    let o = hook(None, tmp.path(), T2, "pre-tool-use", &c);
    denied(&o);
    assert!(o.stderr.contains("warning"));
}

#[test]
fn malformed_stdin_allows() {
    let tmp = repo();
    let out = bosun()
        .current_dir(tmp.path())
        .args(["hook", "pre-tool-use"])
        .write_stdin("not json")
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(0));
    assert!(out.stdout.is_empty());
    assert!(String::from_utf8_lossy(&out.stderr).contains("warning"));
}

#[test]
fn session_start_joins_and_prints_the_context() {
    let (fake, tmp) = (FakeGithub::start(), repo());
    leased(&fake);
    fake.variable("BOSUN_HALT", "0");
    fake.issue(9, "red", &["bosun:main-red"], "x");
    let input = json!({"hook_event_name":"SessionStart","cwd":tmp.path().to_str().unwrap()});
    let o = hook(Some(&fake), tmp.path(), T0, "session-start", &input);
    assert_eq!(o.code, 0, "{}", o.stderr);
    let v: Value = serde_json::from_str(&o.stdout).unwrap();
    assert_eq!(v["hookSpecificOutput"]["hookEventName"], "SessionStart");
    let text = v["hookSpecificOutput"]["additionalContext"]
        .as_str()
        .unwrap();
    for needle in ["session_me", "local", "#5", "#9", "bosun status"] {
        assert!(text.contains(needle), "{needle} missing: {text}");
    }
    assert!(!text.contains("#4"), "only own leases: {text}");
    let s = fake.state.lock().unwrap();
    let registry = s
        .issues
        .iter()
        .find(|i| i.labels.iter().any(|l| l == "bosun:registry"))
        .expect("registry issue");
    let joined = s
        .comments
        .iter()
        .any(|c| c.issue == registry.number && c.body.contains("session_me"));
    assert!(joined, "join comment posted");
    drop(s);
    assert!(tmp.path().join(".git/bosun/leases.json").exists());
}

#[test]
fn session_start_explains_when_github_is_unreachable() {
    let tmp = repo();
    let input = json!({"cwd": tmp.path().to_str().unwrap()});
    let o = hook(None, tmp.path(), T0, "session-start", &input);
    assert_eq!(o.code, 0);
    let v: Value = serde_json::from_str(&o.stdout).unwrap();
    let text = v["hookSpecificOutput"]["additionalContext"]
        .as_str()
        .unwrap();
    assert!(text.contains("could not reach GitHub"), "{text}");
    assert!(text.contains("GH_TOKEN"), "{text}");
}
