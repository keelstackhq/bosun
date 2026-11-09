//! `bosun pin --latest --pr`: branch, commit, push and a draft PR in one go (spec D35).
mod common;

use std::path::Path;
use std::process::Command;

use common::fake::FakeGithub;
use common::npm::{Registry, repo};
use common::{bosun, one_object};
use serde_json::Value;

fn git(dir: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(["-c", "user.name=t", "-c", "user.email=t@t"])
        .args(args)
        .output()
        .unwrap();
    assert!(out.status.success(), "git {args:?}: {out:?}");
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

struct Env {
    work: tempfile::TempDir,
    origin: tempfile::TempDir,
    reg: Registry,
}

impl Env {
    fn new() -> Env {
        let work = repo();
        let origin = tempfile::tempdir().unwrap();
        git(origin.path(), &["init", "-q", "--bare", "-b", "main"]);
        git(work.path(), &["init", "-q", "-b", "main"]);
        git(work.path(), &["add", "-A"]);
        git(work.path(), &["commit", "-q", "-m", "x"]);
        let url = origin.path().to_str().unwrap();
        git(work.path(), &["remote", "add", "origin", url]);
        let reg = Registry::start(
            &[
                ("splice", "0.4.0"),
                ("hoist", "0.4.1"),
                ("berth", "0.1.0"),
                ("bosun", "0.9.0"),
            ],
            None,
            None,
        );
        Env { work, origin, reg }
    }

    fn run(&self, fake: &FakeGithub) -> (i32, Value) {
        let out = bosun()
            .env("BOSUN_NPM_REGISTRY", &self.reg.base)
            .env("BOSUN_GITHUB_API", fake.url())
            .arg("-C")
            .arg(self.work.path())
            .args(["pin", "--latest", "--pr"])
            .output()
            .unwrap();
        (out.status.code().unwrap(), one_object(&out.stdout))
    }
}

#[test]
fn pin_pr_pushes_the_branch_and_opens_a_draft_pr() {
    let (e, fake) = (Env::new(), FakeGithub::start());
    let (code, v) = e.run(&fake);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["pr"]["number"], 1, "{v}");
    assert_eq!(v["next"], serde_json::json!([]));
    assert!(fake.pull_is_draft(1));
    let pushed = git(e.origin.path(), &["log", "-1", "--format=%s", "bosun/pin"]);
    assert_eq!(pushed, "chore: pin hoist 0.4.1");
    assert_eq!(git(e.work.path(), &["status", "--porcelain"]), "");
    let left = v["data"]["left"].to_string();
    assert!(
        left.contains("ready") && left.contains("next session"),
        "{left}"
    );
}

#[test]
fn pin_pr_refuses_a_dirty_tree_before_writing() {
    let (e, fake) = (Env::new(), FakeGithub::start());
    std::fs::write(e.work.path().join("stray.txt"), "x").unwrap();
    let (code, v) = e.run(&fake);
    assert_eq!(code, 3, "{v}");
    assert!(!v["error"]["fix"].as_str().unwrap().is_empty());
    assert_eq!(e.reg.tarballs(), 0);
}

#[test]
fn pin_pr_leaves_an_open_pin_pr_alone() {
    let (e, fake) = (Env::new(), FakeGithub::start());
    fake.pull(7, "chore: pin hoist 0.4.0", "bosun/pin", "");
    let (code, v) = e.run(&fake);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["pr"]["number"], 7);
    assert_eq!(v["data"]["changed"][0]["tool"], "hoist");
    assert!(v["data"]["left"].to_string().contains("#7"));
    assert_eq!(e.reg.tarballs(), 0);
    assert_eq!(git(e.work.path(), &["status", "--porcelain"]), "");
}

#[test]
fn pin_pr_refuses_a_stale_local_branch_before_writing() {
    let (e, fake) = (Env::new(), FakeGithub::start());
    git(e.work.path(), &["branch", "bosun/pin"]);
    let (code, v) = e.run(&fake);
    assert_eq!(code, 3, "{v}");
    assert!(v["error"]["fix"].as_str().unwrap().contains("branch -D"));
    assert_eq!(git(e.work.path(), &["status", "--porcelain"]), "");
    assert_eq!(e.reg.tarballs(), 0);
}

#[test]
fn pin_pr_rolls_back_when_the_push_is_rejected() {
    let (e, fake) = (Env::new(), FakeGithub::start());
    // A hook that rejects every push to origin.
    let hook = e.origin.path().join("hooks/pre-receive");
    std::fs::create_dir_all(hook.parent().unwrap()).unwrap();
    std::fs::write(&hook, "#!/bin/sh\nexit 1\n").unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&hook, std::fs::Permissions::from_mode(0o755)).unwrap();
    let (code, v) = e.run(&fake);
    assert_eq!(code, 1, "{v}");
    assert!(v["error"]["fix"].as_str().unwrap().contains("rolled back"));
    assert_eq!(
        git(e.work.path(), &["rev-parse", "--abbrev-ref", "HEAD"]),
        "main"
    );
    assert_eq!(git(e.work.path(), &["status", "--porcelain"]), "");
    assert_eq!(git(e.work.path(), &["branch", "--list", "bosun/pin"]), "");
}

#[test]
fn pin_pr_stops_when_halted_before_any_download() {
    let (e, fake) = (Env::new(), FakeGithub::start());
    fake.variable("BOSUN_HALT", "1");
    let (code, v) = e.run(&fake);
    assert_eq!(code, 4, "{v}");
    assert_eq!(e.reg.tarballs(), 0);
    assert_eq!(git(e.work.path(), &["status", "--porcelain"]), "");
}

#[test]
fn pin_pr_refuses_a_stale_origin_branch() {
    let (e, fake) = (Env::new(), FakeGithub::start());
    git(e.work.path(), &["push", "-q", "origin", "main:bosun/pin"]);
    let (code, v) = e.run(&fake);
    assert_eq!(code, 3, "{v}");
    assert!(v["error"]["fix"].as_str().unwrap().contains("owner"));
    assert_eq!(e.reg.tarballs(), 0);
}

#[test]
fn pin_pr_needs_the_default_branch() {
    let (e, fake) = (Env::new(), FakeGithub::start());
    git(e.work.path(), &["switch", "-q", "-c", "other"]);
    let (code, v) = e.run(&fake);
    assert_eq!(code, 3, "{v}");
    assert!(
        v["error"]["fix"]
            .as_str()
            .unwrap()
            .contains("git switch main")
    );
    assert_eq!(e.reg.tarballs(), 0);
}
