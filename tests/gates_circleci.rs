//! `bosun gate` on CircleCI (spec §7): the base and `--all` follow `CIRCLE_BRANCH`.
#![cfg(unix)]

mod common;

use common::{bosun, one_object};
use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

struct Repo {
    tmp: tempfile::TempDir,
    root: PathBuf,
}

fn git(root: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .current_dir(root)
        .args(args)
        .output()
        .expect("git");
    assert!(out.status.success(), "git {args:?}: {out:?}");
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

fn write(root: &Path, rel: &str, text: &str) {
    let p = root.join(rel);
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    fs::write(p, text).unwrap();
}

fn commit(root: &Path, rel: &str) {
    write(root, rel, "// changed\n");
    git(root, &["add", "."]);
    git(root, &["commit", "-q", "-m", "change"]);
}

/// Two packages; main moved on (package b) after `work` forked; `origin/main` is main. Fake `cargo` logs argv.
fn repo() -> Repo {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().canonicalize().unwrap().join("repo");
    fs::create_dir_all(&root).unwrap();
    git(&root, &["init", "-q", "-b", "main"]);
    git(&root, &["config", "user.name", "t"]);
    git(&root, &["config", "user.email", "t@example.com"]);
    for (rel, text) in [
        ("Cargo.toml", "[workspace]\nmembers = [\"a\", \"b\"]\n"),
        ("a/Cargo.toml", "[package]\nname = \"a\"\n"),
        ("b/Cargo.toml", "[package]\nname = \"b\"\n"),
        ("a/src/lib.rs", ""),
        ("b/src/lib.rs", ""),
        ("docs/x.md", "x\n"),
        ("tests/t.rs", "#[test]\nfn t() {\n    assert!(true);\n}\n"),
    ] {
        write(&root, rel, text);
    }
    git(&root, &["add", "."]);
    git(&root, &["commit", "-q", "-m", "base"]);
    git(&root, &["branch", "work"]);
    commit(&root, "b/src/lib.rs");
    git(&root, &["update-ref", "refs/remotes/origin/main", "main"]);
    git(&root, &["checkout", "-q", "work"]);
    let log = tmp.path().join("cargo.log");
    let script = format!(
        "#!/bin/sh\necho \"$@\" >> {log}\n\
         [ \"$1\" = metadata ] && printf '{{\"packages\":[{{\"name\":\"a\",\"manifest_path\":\"{root}/a/Cargo.toml\"}},{{\"name\":\"b\",\"manifest_path\":\"{root}/b/Cargo.toml\"}}]}}'\n\
         exit 0\n",
        log = log.display(),
        root = root.display()
    );
    write(tmp.path(), "bin/cargo", &script);
    let path = tmp.path().join("bin/cargo");
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
    Repo { tmp, root }
}

impl Repo {
    fn gate(&self, env: &[(&str, &str)], args: &[&str]) -> Value {
        let (code, v) = self.exec(env, "gate", args);
        assert_eq!(code, 0, "{v}");
        v
    }

    fn exec(&self, env: &[(&str, &str)], sub: &str, args: &[&str]) -> (i32, Value) {
        let path = format!(
            "{}:{}",
            self.tmp.path().join("bin").display(),
            std::env::var("PATH").unwrap_or_default()
        );
        let mut cmd = bosun();
        cmd.env("PATH", path).env_remove("GITHUB_BASE_REF");
        cmd.envs(env.iter().copied());
        let out = cmd.arg("-C").arg(&self.root).arg(sub).args(args);
        let out = out.output().unwrap();
        (out.status.code().unwrap(), one_object(&out.stdout))
    }

    fn calls(&self) -> Vec<String> {
        fs::read_to_string(self.tmp.path().join("cargo.log"))
            .unwrap_or_default()
            .lines()
            .filter(|l| !l.starts_with("metadata"))
            .map(String::from)
            .collect()
    }
}

const PR: [(&str, &str); 2] = [("CIRCLECI", "true"), ("CIRCLE_BRANCH", "work")];
const ALL: &str = "nextest run --locked --workspace";

fn run(file: &str, env: &[(&str, &str)], args: &[&str]) -> (Repo, Value) {
    let r = repo();
    commit(&r.root, file);
    let v = r.gate(env, args);
    (r, v)
}

#[test]
fn pr_diffs_against_the_merge_base_with_origin_main() {
    let (r, v) = run("a/src/lib.rs", &PR, &["test"]);
    // b changed on main after the fork; only the PR's own change (package a) counts.
    assert_eq!(r.calls(), vec!["nextest run --locked -p a"]);
    let fork = git(&r.root, &["merge-base", "HEAD", "origin/main"]);
    assert_eq!(v["data"]["ci"]["provider"], "circleci");
    assert_eq!(v["data"]["ci"]["event"], "pr");
    assert_eq!(v["data"]["ci"]["base"], fork.as_str());
}

#[test]
fn docs_only_pr_skips_flags_win_and_a_missing_base_fails_closed() {
    let (r, v) = run("docs/x.md", &PR, &["test"]);
    assert_eq!(v["data"]["skipped"], true, "{v}");
    assert!(r.calls().is_empty());
    let v = r.gate(&PR, &["test", "--base", "origin/main"]);
    assert_eq!(v["data"]["skipped"], true, "{v}");
    let v = r.gate(&PR, &["test", "--base", "origin/main", "--all"]);
    assert_eq!(v["data"]["skipped"], false, "{v}");
    assert_eq!(r.calls(), vec![ALL]);
    // Without origin/main there is no base: fail closed, never skip.
    git(&r.root, &["update-ref", "-d", "refs/remotes/origin/main"]);
    let (code, v) = r.exec(&PR, "gate", &["test"]);
    assert_eq!(code, 1, "{v}");
    assert!(!v["error"]["fix"].as_str().unwrap().is_empty(), "{v}");
}

#[test]
fn default_push_merge_queue_and_branchless_pipelines_run_all() {
    for event in ["push", "merge-queue", "unknown"] {
        let r = repo();
        let sha = git(&r.root, &["rev-parse", "origin/main"]);
        let env = match event {
            "push" => ("CIRCLE_BRANCH", "main".to_string()),
            "merge-queue" => (
                "CIRCLE_BRANCH",
                format!("gh-readonly-queue/main/pr-7-{sha}"),
            ),
            _ => ("CIRCLE_TAG", "v1".to_string()),
        };
        let v = r.gate(&[("CIRCLECI", "true"), (env.0, &env.1)], &["test"]);
        assert_eq!(v["data"]["skipped"], false, "{event}: {v}");
        assert_eq!(v["data"]["ci"]["event"], event);
        assert_eq!(r.calls(), vec![ALL], "{event}");
        if event == "merge-queue" {
            assert_eq!(v["data"]["ci"]["base"], sha.as_str());
        }
    }
}

#[test]
fn github_and_local_runs_carry_no_ci_block() {
    let env = [("GITHUB_ACTIONS", "true")];
    let (r, v) = run("a/src/lib.rs", &env, &["test", "--base", "main"]);
    assert!(v["data"].get("ci").is_none(), "{v}");
    assert_eq!(r.calls(), vec!["nextest run --locked -p a"]);
}

/// The legacy base (origin/main == HEAD) sees nothing; only the queue's base sha finds the deletion.
#[test]
fn test_integrity_uses_the_merge_queue_base() {
    let r = repo();
    let sha = git(&r.root, &["rev-parse", "origin/main"]);
    fs::remove_file(r.root.join("tests/t.rs")).unwrap();
    git(&r.root, &["add", "-A"]);
    git(&r.root, &["commit", "-q", "-m", "drop the test"]);
    git(&r.root, &["update-ref", "refs/remotes/origin/main", "HEAD"]);
    let branch = format!("gh-readonly-queue/main/pr-7-{sha}");
    let env = [("CIRCLECI", "true"), ("CIRCLE_BRANCH", &branch)];
    let (code, v) = r.exec(&env, "test-integrity", &[]);
    assert_eq!(code, 1, "{v}");
    assert_eq!(v["data"]["findings"][0]["kind"], "deleted-test", "{v}");
}
