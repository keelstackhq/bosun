mod common;

use common::{bosun, one_object};
use serde_json::Value;
use std::fs;
use std::os::unix::fs::PermissionsExt;
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

/// A pnpm workspace (packages/a, packages/b) on branch `work` (base `main`), with a fake `pnpm`
/// on PATH that logs its argv.
fn repo() -> Repo {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().canonicalize().unwrap().join("repo");
    fs::create_dir_all(&root).unwrap();
    git(&root, &["init", "-q", "-b", "main"]);
    git(&root, &["config", "user.name", "t"]);
    git(&root, &["config", "user.email", "t@example.com"]);
    write(
        &root,
        ".bosun/bosun.toml",
        "[[stack]]\npath = \".\"\nprofile = \"ts-pnpm\"\n",
    );
    write(
        &root,
        "package.json",
        "{\"name\":\"root\",\"private\":true,\"scripts\":{\"lint\":\"x\"}}",
    );
    write(
        &root,
        "pnpm-workspace.yaml",
        "packages:\n  - \"packages/*\"\n",
    );
    write(&root, "packages/a/package.json", "{\"name\":\"@w/a\"}");
    write(&root, "packages/a/src/x.ts", "");
    write(&root, "packages/b/package.json", "{\"name\":\"b\"}");
    write(&root, "packages/b/src/x.ts", "");
    write(&root, "docs/x.md", "x\n");
    git(&root, &["add", "."]);
    git(&root, &["commit", "-q", "-m", "base"]);
    git(&root, &["checkout", "-q", "-b", "work"]);
    let bin = tmp.path().join("bin");
    fs::create_dir_all(&bin).unwrap();
    let log = tmp.path().join("pnpm.log");
    let script = format!("#!/bin/sh\necho \"$@\" >> {}\nexit 0\n", log.display());
    let path = bin.join("pnpm");
    fs::write(&path, script).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
    Repo {
        _tmp: tmp,
        root,
        bin,
        log,
    }
}

impl Repo {
    fn change(&self, rel: &str) {
        write(&self.root, rel, "// changed\n");
        git(&self.root, &["add", "."]);
        git(&self.root, &["commit", "-q", "-m", "change"]);
    }

    fn gate(&self, args: &[&str]) -> (i32, Value) {
        let path = format!(
            "{}:{}",
            self.bin.display(),
            std::env::var("PATH").unwrap_or_default()
        );
        let out = bosun()
            .env("PATH", path)
            .env_remove("GITHUB_BASE_REF")
            .arg("-C")
            .arg(&self.root)
            .arg("gate")
            .args(args)
            .output()
            .unwrap();
        (out.status.code().unwrap(), one_object(&out.stdout))
    }

    fn calls(&self) -> Vec<String> {
        fs::read_to_string(&self.log)
            .unwrap_or_default()
            .lines()
            .map(String::from)
            .collect()
    }
}

#[test]
fn change_in_one_package_tests_only_that_package() {
    let r = repo();
    r.change("packages/a/src/x.ts");
    let (code, v) = r.gate(&["test", "--base", "main"]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(r.calls(), vec!["--filter ...@w/a run test"]);
    assert_eq!(v["data"]["packages"], serde_json::json!(["@w/a"]));
}

#[test]
fn root_change_tests_every_package() {
    let r = repo();
    r.change("pnpm-lock.yaml");
    let (code, v) = r.gate(&["test", "--base", "main"]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(r.calls(), vec!["-r --include-workspace-root run test"]);
    assert_eq!(v["data"]["packages"], serde_json::json!(["@w/a", "b"]));
}

#[test]
fn doc_only_change_skips_the_test_gate() {
    let r = repo();
    r.change("docs/x.md");
    let (code, v) = r.gate(&["test", "--base", "main"]);
    assert_eq!(code, 0);
    assert_eq!(v["data"]["skipped"], true);
    assert!(r.calls().is_empty());
}

#[test]
fn fmt_lint_types_deps_call_the_root_scripts() {
    let r = repo();
    r.change("packages/b/src/x.ts");
    write(&r.root, "pnpm-lock.yaml", "lockfileVersion: '9.0'\n");
    for gate in ["fmt", "lint", "types", "deps"] {
        let (code, v) = r.gate(&[gate, "--base", "main"]);
        assert_eq!(code, 0, "{gate}: {v}");
    }
    assert_eq!(
        r.calls(),
        vec![
            "run format:check",
            "run lint",
            "run typecheck",
            "audit --prod --audit-level high"
        ]
    );
}

#[test]
fn unknown_profile_fix_lists_ts_pnpm() {
    let r = repo();
    write(
        &r.root,
        ".bosun/bosun.toml",
        "[[stack]]\npath = \".\"\nprofile = \"nope\"\n",
    );
    git(&r.root, &["add", "."]);
    git(&r.root, &["commit", "-q", "-m", "cfg"]);
    let (code, v) = r.gate(&["fmt", "--base", "main"]);
    assert_eq!(code, 2, "{v}");
    let fix = v["error"]["fix"].as_str().unwrap();
    assert!(fix.contains("rust") && fix.contains("ts-pnpm"), "{fix}");
}

#[test]
fn deps_gate_is_skipped_without_a_lockfile() {
    let r = repo();
    r.change("packages/b/src/x.ts");
    let (code, v) = r.gate(&["deps", "--base", "main"]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["skipped"], true);
    assert_eq!(v["data"]["reason"], "no pnpm-lock.yaml");
    assert!(r.calls().is_empty());
}

#[test]
fn non_code_file_in_a_package_still_runs_its_tests() {
    let r = repo();
    r.change("packages/a/__snapshots__/x.snap");
    let (code, v) = r.gate(&["test", "--base", "main"]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(r.calls(), vec!["--filter ...@w/a run test"]);
}

#[test]
fn flow_list_workspace_file_finds_the_packages() {
    let r = repo();
    write(&r.root, "pnpm-workspace.yaml", "packages: ['packages/*']\n");
    git(&r.root, &["add", "."]);
    git(&r.root, &["commit", "-q", "-m", "flow"]);
    git(&r.root, &["branch", "-f", "main", "work"]);
    r.change("packages/b/src/x.ts");
    let (code, v) = r.gate(&["test", "--base", "main"]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(r.calls(), vec!["--filter ...b run test"]);
}

#[test]
fn unparsable_workspace_globs_are_an_error_with_a_fix() {
    let r = repo();
    write(
        &r.root,
        "pnpm-workspace.yaml",
        "packages:\n  - 'packages/{a,b}'\n",
    );
    git(&r.root, &["add", "."]);
    git(&r.root, &["commit", "-q", "-m", "braces"]);
    git(&r.root, &["branch", "-f", "main", "work"]);
    r.change("packages/b/src/x.ts");
    let (code, v) = r.gate(&["test", "--base", "main"]);
    assert_eq!(code, 1, "{v}");
    assert!(!v["error"]["fix"].as_str().unwrap().is_empty());
    assert!(r.calls().is_empty());
}
