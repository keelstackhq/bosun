//! `risk-check --local` judges `origin/main...HEAD` before a push, offline (spec §8).
mod common;

use std::fs;
use std::path::Path;
use std::process::Command;

use common::bosun;
use serde_json::Value;

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

fn write(dir: &Path, path: &str, body: &str) {
    let f = dir.join(path);
    fs::create_dir_all(f.parent().unwrap()).unwrap();
    fs::write(f, body).unwrap();
}

fn local(dir: &Path) -> (i32, Value) {
    local_in(dir)
}

fn local_in(cwd: &Path) -> (i32, Value) {
    let out = bosun()
        .current_dir(cwd)
        .env("BOSUN_NOW", "2026-11-17T10:00:00Z")
        .args(["risk-check", "--local"])
        .output()
        .unwrap();
    let v = serde_json::from_slice(&out.stdout).unwrap();
    (out.status.code().unwrap(), v)
}

#[test]
fn local_judges_the_branch_against_origin_main() {
    let tmp = tempfile::tempdir().unwrap();
    let d = tmp.path();
    git(d, &["init", "-q", "-b", "main"]);
    write(d, "src/lib.rs", "pub fn a() {}\n");
    write(d, "tests/common/mod.rs", "pub fn h() {}\n");
    git(d, &["add", "-A"]);
    git(d, &["commit", "-q", "-m", "base"]);
    git(d, &["update-ref", "refs/remotes/origin/main", "HEAD"]);
    git(d, &["checkout", "-q", "-b", "feat"]);
    write(
        d,
        "tests/common/mod.rs",
        "#![allow(dead_code)]\npub fn h() {}\n",
    );
    write(d, "docs/a.md", "x\n");
    git(d, &["add", "-A"]);
    git(d, &["commit", "-q", "-m", "ok"]);
    let (code, v) = local(d);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["command"], "risk-check");
    assert_eq!(v["data"]["local"], true);
    assert_eq!(v["data"]["level"], "normal", "{v}");
    write(d, "src/lib.rs", "#[allow(clippy::all)]\npub fn a() {}\n");
    git(d, &["commit", "-qam", "weaken"]);
    let (code, v) = local(d);
    assert_eq!(code, 5, "{v}");
    assert_eq!(v["data"]["level"], "high");
    let ev = v["data"]["matches"].to_string();
    assert!(
        ev.contains("R-weaken") && ev.contains("src/lib.rs: lint allow/expect 0 -> 1"),
        "{ev}"
    );
    assert!(!v["error"]["fix"].as_str().unwrap().is_empty());
    // From a subdirectory, with colour forced on, the same verdict.
    git(d, &["config", "color.ui", "always"]);
    let (code, v) = local_in(&d.join("src"));
    assert_eq!(code, 5, "{v}");
    assert!(
        v["data"]["matches"]
            .to_string()
            .contains("lint allow/expect 0 -> 1"),
        "{v}"
    );
}
