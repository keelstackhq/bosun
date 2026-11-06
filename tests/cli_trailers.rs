//! `bosun trailers`: stamp, install (a real commit gets stamped), list.

mod common;

use std::path::Path;
use std::process::Command;

use common::{bosun, one_object};
use serde_json::Value;

fn git(dir: &Path, args: &[&str], envs: &[(&str, &str)]) -> String {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(["-c", "user.name=t", "-c", "user.email=t@t"])
        .args(args)
        .envs(envs.iter().copied())
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

fn repo() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    git(dir.path(), &["init", "-q", "-b", "main"], &[]);
    dir
}

fn run(dir: &Path, envs: &[(&str, &str)], args: &[&str]) -> (i32, Value) {
    let mut c = bosun();
    for (k, v) in envs {
        c.env(k, v);
    }
    let out = c
        .arg("-C")
        .arg(dir)
        .arg("trailers")
        .args(args)
        .output()
        .unwrap();
    (out.status.code().unwrap(), one_object(&out.stdout))
}

#[test]
fn stamp_appends_session_task_and_model_from_the_env() {
    let dir = repo();
    let f = dir.path().join("msg");
    std::fs::write(&f, "feat: x\n").unwrap();
    let envs = [("BOSUN_TASK", "12"), ("BOSUN_MODEL", "m-1")];
    let (code, v) = run(dir.path(), &envs, &["stamp", f.to_str().unwrap()]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["stamped"], true);
    assert_eq!(
        std::fs::read_to_string(&f).unwrap(),
        "feat: x\n\nBosun-Session: session_test\nBosun-Task: 12\nBosun-Model: m-1\n"
    );
    let (_, v) = run(dir.path(), &envs, &["stamp", f.to_str().unwrap()]);
    assert_eq!(v["data"]["stamped"], false, "second run changes nothing");
}

#[test]
fn stamp_without_task_or_model_adds_only_the_session() {
    let dir = repo();
    let f = dir.path().join("msg");
    std::fs::write(&f, "feat: x\n").unwrap();
    let (code, _) = run(dir.path(), &[], &["stamp", f.to_str().unwrap()]);
    assert_eq!(code, 0);
    let t = std::fs::read_to_string(&f).unwrap();
    assert!(
        t.contains("Bosun-Session: session_test") && !t.contains("Bosun-Task"),
        "{t}"
    );
}

#[test]
fn an_installed_hook_stamps_a_real_commit_and_list_finds_it() {
    let dir = repo();
    let (code, v) = run(dir.path(), &[], &["install"]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(run(dir.path(), &[], &["install"]).0, 0, "idempotent");
    let bin = assert_cmd::cargo::cargo_bin("bosun");
    let path = format!(
        "{}:{}",
        bin.parent().unwrap().display(),
        std::env::var("PATH").unwrap()
    );
    let envs = [
        ("PATH", path.as_str()),
        ("BOSUN_SESSION", "session_test"),
        ("BOSUN_TASK", "5"),
    ];
    git(
        dir.path(),
        &["commit", "-q", "--allow-empty", "-m", "feat: a"],
        &envs,
    );
    git(
        dir.path(),
        &["commit", "-q", "--allow-empty", "-m", "feat: other"],
        &[("PATH", path.as_str()), ("BOSUN_SESSION", "session_other")],
    );
    let body = git(dir.path(), &["log", "-2", "--format=%B"], &[]);
    assert!(body.contains("Bosun-Task: 5"), "{body}");
    let (code, v) = run(dir.path(), &[], &["list", "session_test"]);
    assert_eq!(code, 0, "{v}");
    let commits = v["data"]["commits"].as_array().unwrap();
    assert_eq!(commits.len(), 1, "{v}");
    assert_eq!(commits[0]["subject"], "feat: a");
    let sha = commits[0]["sha"].as_str().unwrap();
    assert!(
        v["data"]["revert"]
            .as_str()
            .unwrap()
            .contains(&format!("git revert --no-edit {sha}")),
        "{v}"
    );
    let (_, none) = run(dir.path(), &[], &["list", "session_nobody"]);
    assert_eq!(none["data"]["revert"], Value::Null);
}

#[test]
fn install_will_not_replace_another_hook() {
    let dir = repo();
    let hook = dir.path().join(".git/hooks/commit-msg");
    std::fs::create_dir_all(hook.parent().unwrap()).unwrap();
    std::fs::write(&hook, "#!/bin/sh\necho mine\n").unwrap();
    let (code, v) = run(dir.path(), &[], &["install"]);
    assert_eq!(code, 3, "{v}");
    assert!(!v["error"]["fix"].as_str().unwrap().is_empty());
    assert_eq!(
        std::fs::read_to_string(&hook).unwrap(),
        "#!/bin/sh\necho mine\n"
    );
}
