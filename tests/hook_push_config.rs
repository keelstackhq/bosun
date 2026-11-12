//! Merge-lock (spec §4): pushes whose target comes from git config (`remote.<r>.push`,
//! `remote.<r>.mirror`, `push.default`) are read in the checkout where git runs.

mod common;

use std::fs;
use std::path::Path;
use std::process::Command;

use common::bosun;
use common::fake::FakeGithub;
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

/// A repo on branch `feat` with one commit after `main`.
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

#[test]
fn push_refspecs_or_mirror_from_config_refuse_every_push() {
    for (key, value) in [
        ("remote.origin.push", "HEAD:refs/heads/main"),
        ("remote.origin.push", "refs/heads/feat:refs/heads/feat"),
        ("remote.backup.push", "+refs/heads/*:refs/heads/*"),
        ("remote.origin.mirror", "true"),
    ] {
        let r = repo(None);
        git(r.path(), &["config", key, value]);
        for cmd in ["git push", "git push origin", "git push -u origin feat"] {
            let why = hook(r.path(), cmd).expect(cmd);
            assert!(why.contains(key), "{key}: {why}");
        }
    }
}

#[test]
fn push_default_that_can_reach_the_default_branch_refuses_a_bare_push() {
    let r = repo(None);
    git(r.path(), &["config", "push.default", "matching"]);
    assert!(
        hook(r.path(), "git push")
            .expect("matching")
            .contains("push.default")
    );
    assert_eq!(hook(r.path(), "git push -u origin feat"), None);
    git(r.path(), &["config", "push.default", "upstream"]);
    git(r.path(), &["branch", "--set-upstream-to=origin/main"]);
    for cmd in ["git push", "git push origin", "git -C . push"] {
        let why = hook(r.path(), cmd).expect(cmd);
        assert!(why.contains("upstream"), "{why}");
    }
    let why = hook(r.path(), "cd . && git push").expect("chained");
    assert!(why.contains("as its own command"), "{why}");
    git(
        r.path(),
        &["update-ref", "refs/remotes/origin/feat", "HEAD"],
    );
    git(r.path(), &["branch", "--set-upstream-to=origin/feat"]);
    assert_eq!(hook(r.path(), "git push"), None);
    git(r.path(), &["branch", "--unset-upstream"]);
    assert!(
        hook(r.path(), "git push")
            .expect("no upstream")
            .contains("upstream")
    );
}

#[test]
fn plain_config_pushes_as_today() {
    let r = repo(None);
    for cmd in ["git push", "git push origin", "git push -u origin feat"] {
        assert_eq!(hook(r.path(), cmd), None, "{cmd}");
    }
    git(r.path(), &["config", "push.default", "current"]);
    assert_eq!(hook(r.path(), "git push"), None);
}
