//! `push_gates = "push"` (spec §7): the PreToolUse hook refuses a push whose tree lacks a
//! passing stamp for a ready gate; `ready` (and no `[mode]`) pushes as today.

mod common;

use std::fs;
use std::path::Path;
use std::process::Command;

use common::bosun;
use common::fake::FakeGithub;
use common::stamps;
use serde_json::{Value, json};

fn git(dir: &Path, args: &[&str]) {
    let cfg = ["-c", "user.name=t", "-c", "user.email=t@example.com"];
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(cfg)
        .args(args)
        .output()
        .unwrap();
    assert!(out.status.success(), "git {args:?}: {out:?}");
}

/// A repo on branch `feat` with one commit holding `.bosun/bosun.toml` = `config`.
fn repo(config: Option<&str>) -> tempfile::TempDir {
    let tmp = tempfile::tempdir().unwrap();
    let d = tmp.path();
    git(d, &["init", "-q", "-b", "main"]);
    git(
        d,
        &[
            "remote",
            "add",
            "origin",
            "https://github.com/acme/widget.git",
        ],
    );
    fs::write(d.join("a.txt"), "x").unwrap();
    git(d, &["add", "-A"]);
    git(d, &["commit", "-q", "-m", "base"]);
    git(d, &["update-ref", "refs/remotes/origin/main", "HEAD"]);
    git(d, &["checkout", "-q", "-b", "feat"]);
    if let Some(c) = config {
        fs::create_dir_all(d.join(".bosun")).unwrap();
        fs::write(d.join(".bosun/bosun.toml"), c).unwrap();
    }
    fs::write(d.join("a.txt"), "y").unwrap();
    git(d, &["add", "-A"]);
    git(d, &["commit", "-q", "-m", "work"]);
    tmp
}

/// The hook's deny reason for a Bash `command` run in `cwd`, `None` when it allows it.
fn hook(cwd: &Path, command: &str) -> Option<String> {
    let fake = FakeGithub::start();
    let input = json!({"session_id": "s", "tool_name": "Bash", "cwd": cwd.to_str().unwrap(),
        "tool_input": {"command": command}});
    let out = bosun()
        .current_dir(cwd)
        .env("BOSUN_SESSION", "session_me")
        .env("BOSUN_NOW", "2026-11-12T10:00:00Z")
        .env("BOSUN_GITHUB_API", fake.url())
        .args(["hook", "pre-tool-use"])
        .write_stdin(input.to_string())
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(0));
    let v: Value = serde_json::from_str(&String::from_utf8_lossy(&out.stdout)).ok()?;
    (v["hookSpecificOutput"]["permissionDecision"] == "deny").then(|| {
        v["hookSpecificOutput"]["permissionDecisionReason"]
            .as_str()
            .unwrap()
            .into()
    })
}

const STEADY: &str = "project = \"widget\"\n\n[mode]\nname = \"steady\"\n";
const PUSH: &str = "git push -u origin feat";

#[test]
fn steady_refuses_a_push_without_stamps_and_names_the_gate_loop() {
    let r = repo(Some(STEADY));
    let why = hook(r.path(), PUSH).expect("denied");
    assert!(why.contains("push-gates"), "{why}");
    for g in stamps::READY_GATES {
        assert!(why.contains(g), "{why}");
    }
    assert!(why.contains("bosun gate $g"), "{why}");
}

#[test]
fn steady_allows_a_push_once_every_ready_gate_is_stamped() {
    let r = repo(Some(STEADY));
    stamps::stamp(r.path(), &["fmt", "lint"]);
    let why = hook(r.path(), PUSH).expect("still missing gates");
    assert!(
        why.contains("test-integrity") && !why.contains("lint,"),
        "{why}"
    );
    stamps::stamp_all(r.path());
    assert_eq!(hook(r.path(), PUSH), None);
}

#[test]
fn without_mode_or_with_ready_a_push_needs_no_stamps() {
    for config in [
        None,
        Some("project = \"widget\"\n"),
        Some("project = \"widget\"\n\n[mode]\nname = \"steady\"\npush_gates = \"ready\"\n"),
    ] {
        let r = repo(config);
        assert_eq!(hook(r.path(), PUSH), None, "{config:?}");
    }
}

#[test]
fn the_pushed_tree_is_read_where_git_runs_and_from_the_pushed_ref() {
    let r = repo(Some(STEADY));
    let outside = tempfile::tempdir().unwrap();
    let at = r.path().to_str().unwrap();
    for cmd in [
        format!("cd {at} && git push origin feat"),
        format!("git -C {at} push origin feat"),
    ] {
        assert!(hook(outside.path(), &cmd).is_some(), "{cmd}");
    }
    // Stamps for HEAD do not vouch for another pushed branch.
    stamps::stamp_all(r.path());
    git(r.path(), &["branch", "old", "main"]);
    assert_eq!(hook(r.path(), "git push origin feat"), None);
    let why = hook(r.path(), "git push origin old").expect("old is not stamped");
    assert!(why.contains("`old`"), "{why}");
    // A script that pushes counts too.
    fs::write(r.path().join("ship.sh"), "git push origin old\n").unwrap();
    assert!(hook(r.path(), "bash ship.sh").is_some());
}

#[test]
fn a_push_whose_checkout_cannot_be_told_is_refused_in_push_mode() {
    let r = repo(Some(STEADY));
    stamps::stamp_all(r.path());
    for cmd in [
        "python3 -c \"import subprocess; subprocess.run(['git', 'push', 'origin', 'feat'])\"",
        "git --git-dir=../w/.git push origin feat",
    ] {
        let why = hook(r.path(), cmd).expect(cmd);
        assert!(why.contains("cannot tell which checkout"), "{why}");
    }
    let plain = repo(None);
    let py = "python3 -c \"import subprocess; subprocess.run(['git', 'push', 'origin', 'feat'])\"";
    assert_eq!(hook(plain.path(), py), None);
}
