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
    hook_env(cwd, command, &[])
}

/// `hook` with extra environment for the hook process.
fn hook_env(cwd: &Path, command: &str, env: &[(&str, &str)]) -> Option<String> {
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
        .envs(env.iter().copied())
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
fn aliases_that_push_or_run_a_shell_are_refused() {
    let r = repo(None);
    git(r.path(), &["config", "alias.p", "push"]);
    git(r.path(), &["config", "alias.q", "!git push"]);
    git(r.path(), &["config", "alias.st", "status"]);
    git(r.path(), &["config", "alias.pp", "-c x=y push"]);
    git(r.path(), &["config", "alias.sp", "remote -v && git push"]);
    for cmd in [
        "git p origin feat:main",
        "git q",
        "git \"p\" origin",
        "git pp origin feat:main",
        "git sp",
        "git -c user.name=x p origin",
        "cargo test && git -C . p origin",
    ] {
        let why = hook(r.path(), cmd).expect(cmd);
        assert!(why.contains("alias"), "{why}");
    }
    for cmd in [
        "git st",
        "git commit -m 'fix p'",
        "ls st p q",
        "echo git status p",
    ] {
        assert_eq!(hook(r.path(), cmd), None, "{cmd}");
    }
    // Read where git runs: `-C` moves to a checkout with its own aliases.
    let other = repo(None);
    git(other.path(), &["config", "alias.x", "push"]);
    let at = format!("git -C {} x origin feat:main", other.path().display());
    assert!(hook(r.path(), &at).expect("alias at -C").contains("alias"));
    assert_eq!(hook(r.path(), "git x origin feat:main"), None);
    // A `cd` or `pushd` moves where the alias is read; one the guard cannot read leaves only
    // builtins.
    let o = other.path().display();
    for cmd in [
        format!("cd {o} && git x origin feat:main"),
        format!("cd {o}; git x origin feat:main"),
        format!("pushd {o} && git x origin feat:main"),
        format!("cd \"{o}\" && git x origin feat:main"),
        "cd $D && git x origin feat:main".to_string(),
    ] {
        assert!(hook(r.path(), &cmd).expect(&cmd).contains("alias"), "{cmd}");
    }
    // CDPATH or a path git cannot open as read: only builtins pass.
    let spaced = tempfile::tempdir().unwrap();
    let my = spaced.path().join("my dir");
    fs::create_dir(&my).unwrap();
    git(&my, &["init", "-q"]);
    git(&my, &["config", "alias.p", "push"]);
    let (base, s) = (spaced.path().display(), my.display());
    for cmd in [
        format!("CDPATH={base} cd 'my dir' && git p origin feat:main"),
        format!("export CDPATH={base}; cd x && git p origin"),
        format!("cd \"{s}\" && git p origin"),
        format!("cd '{s}' && git p origin"),
    ] {
        assert!(hook(r.path(), &cmd).expect(&cmd).contains("alias"), "{cmd}");
    }
    assert_eq!(hook(r.path(), &format!("cd \"{s}\" && git status")), None);
    assert_eq!(hook(r.path(), &format!("cd {o} && git status")), None);
    assert_eq!(hook(r.path(), "cd $D && git status"), None);
}

#[test]
fn cdpath_in_the_hook_environment_makes_a_cd_unknown() {
    // `cd sub` lands in CDPATH's `sub` (alias p=push), not the checkout's own `sub`.
    let r = repo(None);
    fs::create_dir(r.path().join("sub")).unwrap();
    let base = tempfile::tempdir().unwrap();
    let sub = base.path().join("sub");
    fs::create_dir(&sub).unwrap();
    git(&sub, &["init", "-q"]);
    git(&sub, &["config", "alias.p", "push"]);
    let cmd = "cd sub && git p origin feat:main";
    assert_eq!(
        hook(r.path(), cmd),
        None,
        "without CDPATH the checkout's sub has no alias"
    );
    let cdpath = base.path().to_str().unwrap();
    let why = hook_env(r.path(), cmd, &[("CDPATH", cdpath)]).expect("CDPATH");
    assert!(why.contains("alias"), "{why}");
    assert_eq!(
        hook_env(r.path(), "cd sub && git status", &[("CDPATH", cdpath)]),
        None
    );
}

#[test]
fn unreadable_config_fails_closed() {
    let r = repo(None);
    fs::write(r.path().join(".git/config"), "[broken\n").unwrap();
    let why = hook(r.path(), "git push").expect("broken config");
    assert!(why.contains("cannot read git config"), "{why}");
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
