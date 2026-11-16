//! Merge-lock (spec §4): a fetch or pull follows the `remote.<r>.fetch` refspecs and the
//! protocol and URL rewrite config of the checkout where git runs; the hook asks git for them.

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
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .arg("-C")
        .arg(dir)
        .args(cfg)
        .args(args)
        .output()
        .unwrap();
    assert!(out.status.success(), "git {args:?}: {out:?}");
}

/// A repo on branch `feat` with an `origin` remote.
fn repo_at(d: &Path) {
    fs::create_dir_all(d).unwrap();
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
}

fn repo() -> tempfile::TempDir {
    let tmp = tempfile::tempdir().unwrap();
    repo_at(tmp.path());
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
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env_remove("CDPATH")
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
fn configured_refspecs_that_move_the_default_branch_or_tags_refuse_fetch_and_pull() {
    for (key, value) in [
        ("remote.origin.fetch", "+refs/heads/feat:refs/heads/main"),
        ("remote.origin.fetch", "+refs/*:refs/*"),
        ("remote.origin.fetch", "+refs/heads/*:refs/heads/*"),
        ("remote.evil.fetch", "+refs/heads/main:refs/heads/main"),
        ("remote.evil.fetch", "+refs/tags/*:refs/tags/*"),
        ("protocol.ext.allow", "always"),
        ("protocol.allow", "always"),
        ("url.ext::sh p.sh #.insteadOf", "https://github.com/"),
        ("url.fd::3.insteadOf", "https://github.com/"),
        // Review round 1: more keys that run a command or retarget a fetch.
        ("protocol.allow", "user"),
        ("remote.origin.url", "ext::sh p.sh"),
        ("remote.origin.uploadpack", "sh p.sh"),
        ("remote.origin.receivepack", "sh p.sh"),
        ("core.sshCommand", "sh p.sh"),
        ("url./tmp/evil.insteadOf", "https://github.com/"),
        ("url.evil2.insteadOf", "https://github.com/"),
        ("remote.evil.tagOpt", "--tags"),
        ("remote.origin.fetch", "+refs/heads/feat:refs/heads/MAIN"),
    ] {
        let r = repo();
        git(r.path(), &["config", "--add", key, value]);
        for cmd in [
            "git fetch",
            "git fetch origin",
            "git pull",
            "git pull --rebase origin feat",
        ] {
            let why = hook(r.path(), cmd).unwrap_or_else(|| panic!("{key} {value}: {cmd}"));
            assert!(why.contains("merge-lock"), "{key}: {why}");
        }
    }
}

#[test]
fn ordinary_fetch_config_passes() {
    let r = repo();
    git(
        r.path(),
        &[
            "config",
            "--add",
            "remote.origin.fetch",
            "+refs/tags/*:refs/tags/*",
        ],
    );
    git(r.path(), &["config", "protocol.ext.allow", "never"]);
    for cmd in [
        "git fetch",
        "git fetch origin main:main",
        "git pull --rebase",
        "git status",
    ] {
        assert_eq!(hook(r.path(), cmd), None, "{cmd}");
    }
}

#[test]
fn config_is_read_where_git_runs() {
    let r = repo();
    let other = r.path().join("other");
    repo_at(&other);
    git(
        &other,
        &[
            "config",
            "--add",
            "remote.origin.fetch",
            "+refs/heads/feat:refs/heads/main",
        ],
    );
    for cmd in [
        "cd other && git fetch",
        "git -C other fetch origin",
        "cd other && git pull",
        "git --no-pager -C other fetch origin",
        "git -P -C other fetch",
        "git -Cother fetch origin",
        "git --git-dir=other/.git fetch origin",
        "git --git-dir other/.git fetch origin",
        "git --work-tree=other -C other fetch",
        "pushd other && git fetch",
        "git -C other remote update",
        "cd other && git remote update origin",
    ] {
        assert!(hook(r.path(), cmd).is_some(), "{cmd}");
    }
    assert_eq!(hook(r.path(), "git fetch"), None);
}

#[test]
fn git_core_programs_count_as_git() {
    let r = repo();
    for cmd in [
        "/usr/lib/git-core/git-fetch . +feat:main",
        "git-fetch . +feat:main",
        "/usr/lib/git-core/git-push origin HEAD:main",
        "git-pull . feat:main",
    ] {
        assert!(hook(r.path(), cmd).is_some(), "{cmd}");
    }
}
