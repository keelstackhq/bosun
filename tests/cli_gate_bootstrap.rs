//! Bootstrap PRs (spec §7): a gate whose config file the base lacks is neutral, and a `--locked`
//! gate without a Cargo.lock gives one clear error. Unix only: fake `cargo` shell script.
#![cfg(unix)]

mod common;

use common::{bosun, one_object};
use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

struct Repo {
    _tmp: tempfile::TempDir,
    root: PathBuf,
    bin: PathBuf,
    log: PathBuf,
}

fn git(root: &Path, args: &[&str]) {
    let out = Command::new("git")
        .current_dir(root)
        .args(args)
        .output()
        .expect("git");
    assert!(out.status.success(), "git {args:?}: {out:?}");
}

fn write(root: &Path, rel: &str, text: &str) {
    let p = root.join(rel);
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    fs::write(p, text).unwrap();
}

/// Base `main` has a crate and the given extra files; branch `work` starts there.
fn repo(base_files: &[&str]) -> Repo {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().canonicalize().unwrap().join("repo");
    fs::create_dir_all(&root).unwrap();
    git(&root, &["init", "-q", "-b", "main"]);
    git(&root, &["config", "user.name", "t"]);
    git(&root, &["config", "user.email", "t@example.com"]);
    write(&root, "Cargo.toml", "[package]\nname = \"a\"\n");
    write(&root, "src/lib.rs", "");
    for f in base_files {
        write(&root, f, "# x\n");
    }
    git(&root, &["add", "."]);
    git(&root, &["commit", "-q", "-m", "base"]);
    git(&root, &["checkout", "-q", "-b", "work"]);
    let bin = tmp.path().join("bin");
    fs::create_dir_all(&bin).unwrap();
    let log = tmp.path().join("cargo.log");
    let script = format!(
        "#!/bin/sh\necho \"$@\" >> {log}\n\
         if [ \"$1\" = metadata ]; then\n\
         printf '{{\"packages\":[{{\"name\":\"a\",\"manifest_path\":\"{root}/Cargo.toml\",\"targets\":[{{\"kind\":[\"lib\"]}}]}}]}}'\nexit 0\nfi\n\
         exit ${{FAKE_CARGO_EXIT:-0}}\n",
        log = log.display(),
        root = root.display()
    );
    let path = bin.join("cargo");
    fs::write(&path, script).unwrap();
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
    Repo {
        _tmp: tmp,
        root,
        bin,
        log,
    }
}

impl Repo {
    fn commit(&self, rel: &str) {
        write(&self.root, rel, "// changed\n");
        git(&self.root, &["add", "."]);
        git(&self.root, &["commit", "-q", "-m", "change"]);
    }

    fn gate(&self, gate: &str, exit: &str) -> (i32, Value) {
        let path = format!(
            "{}:{}",
            self.bin.display(),
            std::env::var("PATH").unwrap_or_default()
        );
        let out = bosun()
            .env("PATH", path)
            .env("FAKE_CARGO_EXIT", exit)
            .env_remove("GITHUB_BASE_REF")
            .env_remove("CI")
            .arg("-C")
            .arg(&self.root)
            .args(["gate", gate, "--base", "main"])
            .output()
            .unwrap();
        (out.status.code().unwrap(), one_object(&out.stdout))
    }

    fn calls(&self) -> Vec<String> {
        fs::read_to_string(&self.log)
            .unwrap_or_default()
            .lines()
            .filter(|l| !l.starts_with("metadata"))
            .map(String::from)
            .collect()
    }
}

#[test]
fn deps_is_neutral_when_the_base_has_no_deny_toml() {
    let r = repo(&[]);
    write(&r.root, "deny.toml", "# new\n");
    r.commit("src/lib.rs");
    let (code, v) = r.gate("deps", "1");
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["skipped"], true);
    assert_eq!(
        v["data"]["reason"],
        "no deny.toml in base: bootstrap PR, gate neutral"
    );
    assert!(r.calls().is_empty());
}

#[test]
fn deps_runs_when_the_base_has_deny_toml() {
    let r = repo(&["deny.toml"]);
    r.commit("src/lib.rs");
    let (code, v) = r.gate("deps", "0");
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["skipped"], false);
    assert_eq!(r.calls(), vec!["deny check"]);
}

#[test]
fn a_locked_gate_without_cargo_lock_gives_one_clear_error() {
    let r = repo(&[]);
    r.commit("src/lib.rs");
    let (code, v) = r.gate("lint", "101");
    assert_eq!(code, 1, "{v}");
    assert!(
        v["error"]["message"]
            .as_str()
            .unwrap()
            .contains("Cargo.lock")
    );
    let fix = v["error"]["fix"].as_str().unwrap();
    assert!(fix.contains("cargo generate-lockfile"), "{fix}");
    assert!(fix.contains("commit Cargo.lock"), "{fix}");
}

#[test]
fn with_a_lockfile_the_failure_stays_the_plain_gate_failure() {
    let r = repo(&["Cargo.lock"]);
    r.commit("src/lib.rs");
    let (code, v) = r.gate("lint", "101");
    assert_eq!(code, 1, "{v}");
    assert_eq!(v["error"]["message"], "gate lint failed (exit 101)");
}

#[test]
fn a_gate_without_locked_keeps_its_plain_failure_without_a_lockfile() {
    let r = repo(&["deny.toml"]);
    r.commit("src/lib.rs");
    let (code, v) = r.gate("deps", "1");
    assert_eq!(code, 1, "{v}");
    assert_eq!(v["error"]["message"], "gate deps failed (exit 1)");
}
