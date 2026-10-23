//! M7 use cases: mixed stacks (UC-M7-3) and reduced integrity for generic adapters (UC-M7-5).
mod common;

use common::fake::FakeGithub;
use common::{bosun, one_object, run};
use serde_json::{Value, json};
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;

const CONFIG: &str = "[[stack]]\npath = \"crates/\"\nprofile = \"rust\"\n[[stack]]\npath = \"py/\"\nprofile = \"python\"\n";

const PY_PROFILE: &str = r#"
[adapters]
packages = "none"
pending = "regex"
assertions = "regex"
[adapters.regex]
test_files = ["**/test_*.py"]
pending = ['@pytest\.mark\.skip']
assertions = ['\bassert\b']
[gates.lint]
cmd = ["fakepy", "lint"]
[gates.test]
cmd = ["fakepy", "test"]
each_package = ["--pkg", "{package}"]
all = ["--everything"]
when_no_package = "skip"
"#;

const REASON: &str = "stack py/ uses regex adapters";

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
        .unwrap();
    assert!(out.status.success(), "git {args:?}: {out:?}");
}

fn write(root: &Path, rel: &str, text: &str) {
    let p = root.join(rel);
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    fs::write(p, text).unwrap();
}

fn script(path: &Path, body: &str) {
    fs::write(path, format!("#!/bin/sh\n{body}")).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
}

/// A rust stack at `crates/` and a python stack at `py/`, on branch `work` (base `main`), with
/// fake `cargo` and `fakepy` that log "<tool>|<cwd>|<args>".
fn repo() -> Repo {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().canonicalize().unwrap().join("repo");
    fs::create_dir_all(&root).unwrap();
    git(&root, &["init", "-q", "-b", "main"]);
    git(&root, &["config", "user.name", "t"]);
    git(&root, &["config", "user.email", "t@example.com"]);
    write(&root, ".bosun/bosun.toml", CONFIG);
    write(&root, ".bosun/profiles/python.toml", PY_PROFILE);
    write(
        &root,
        "crates/Cargo.toml",
        "[workspace]\nmembers = [\"a\"]\n",
    );
    write(&root, "crates/a/Cargo.toml", "[package]\nname = \"a\"\n");
    write(&root, "crates/a/src/lib.rs", "");
    write(&root, "py/app.py", "x = 1\n");
    write(
        &root,
        "py/test_app.py",
        "def test_a():\n    assert 1\n    assert 2\n",
    );
    write(&root, "docs/x.md", "x\n");
    git(&root, &["add", "."]);
    git(&root, &["commit", "-q", "-m", "base"]);
    git(&root, &["checkout", "-q", "-b", "work"]);
    let bin = tmp.path().join("bin");
    fs::create_dir_all(&bin).unwrap();
    let log = tmp.path().join("tools.log");
    let l = log.display();
    script(
        &bin.join("cargo"),
        &format!(
            "if [ \"$1\" = metadata ]; then\n\
             printf '{{\"packages\":[{{\"name\":\"a\",\"manifest_path\":\"{root}/crates/a/Cargo.toml\"}}]}}'\n\
             exit 0\nfi\necho \"cargo|$(pwd -P)|$*\" >> {l}\n",
            root = root.display()
        ),
    );
    script(
        &bin.join("fakepy"),
        &format!("echo \"fakepy|$(pwd -P)|$*\" >> {l}\n"),
    );
    Repo {
        _tmp: tmp,
        root,
        bin,
        log,
    }
}

impl Repo {
    fn change(&self, rels: &[&str]) {
        for rel in rels {
            write(&self.root, rel, "# changed\n");
        }
        git(&self.root, &["add", "."]);
        git(&self.root, &["commit", "-q", "-m", "change"]);
    }

    fn gate(&self, args: &[&str]) -> Value {
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
            .args(["--base", "main"])
            .output()
            .unwrap();
        assert_eq!(out.status.code(), Some(0), "{out:?}");
        one_object(&out.stdout)
    }

    fn integrity(&self) -> (i32, Value) {
        let out = bosun()
            .env_remove("GITHUB_BASE_REF")
            .env_remove("BOSUN_PR_BODY")
            .env_remove("BOSUN_INTEGRITY_ALLOW")
            .arg("-C")
            .arg(&self.root)
            .args(["test-integrity", "--base", "main"])
            .output()
            .unwrap();
        (out.status.code().unwrap(), one_object(&out.stdout))
    }

    fn calls(&self) -> Vec<String> {
        let root = self.root.display().to_string();
        fs::read_to_string(&self.log)
            .unwrap_or_default()
            .lines()
            .map(|l| l.replace(&root, "<root>"))
            .collect()
    }
}

#[test]
fn uc_m7_3_a_python_change_runs_the_python_profile_in_py() {
    let r = repo();
    r.change(&["py/app.py"]);
    let v = r.gate(&["test"]);
    assert_eq!(r.calls(), vec!["fakepy|<root>/py|test --everything"]);
    assert_eq!(v["data"]["stack"], "py/");
    assert_eq!(
        v["data"]["command"],
        json!(["fakepy", "test", "--everything"])
    );
    assert!(v["data"].get("runs").is_none());
}

#[test]
fn uc_m7_3_a_rust_change_runs_the_rust_profile_in_crates() {
    let r = repo();
    r.change(&["crates/a/src/lib.rs"]);
    let v = r.gate(&["test"]);
    assert_eq!(
        r.calls(),
        vec!["cargo|<root>/crates|nextest run --locked -p a"]
    );
    assert_eq!(v["data"]["stack"], "crates/");
    assert_eq!(v["data"]["packages"], json!(["a"]));
}

#[test]
fn uc_m7_3_a_change_in_both_stacks_runs_both_and_lists_both() {
    let r = repo();
    r.change(&["crates/a/src/lib.rs", "py/app.py"]);
    let v = r.gate(&["test"]);
    assert_eq!(
        r.calls(),
        vec![
            "cargo|<root>/crates|nextest run --locked -p a",
            "fakepy|<root>/py|test --everything"
        ]
    );
    let stacks: Vec<&str> = v["data"]["runs"]
        .as_array()
        .unwrap()
        .iter()
        .map(|x| x["stack"].as_str().unwrap())
        .collect();
    assert_eq!(stacks, ["crates/", "py/"]);
}

#[test]
fn uc_m7_3_all_runs_every_stack() {
    let r = repo();
    r.change(&["docs/x.md"]);
    let v = r.gate(&["test", "--all"]);
    assert_eq!(
        r.calls(),
        vec![
            "cargo|<root>/crates|nextest run --locked --workspace",
            "fakepy|<root>/py|test --everything"
        ]
    );
    assert_eq!(v["data"]["runs"].as_array().unwrap().len(), 2);
}

#[test]
fn uc_m7_3_a_gate_without_package_args_runs_in_the_stack_dir() {
    let r = repo();
    r.change(&["py/app.py"]);
    r.gate(&["lint"]);
    assert_eq!(r.calls(), vec!["fakepy|<root>/py|lint"]);
}

fn status_dir() -> tempfile::TempDir {
    let tmp = tempfile::tempdir().unwrap();
    write(tmp.path(), ".bosun/bosun.toml", CONFIG);
    write(tmp.path(), ".bosun/profiles/python.toml", PY_PROFILE);
    tmp
}

fn status(dir: &Path) -> Value {
    let fake = FakeGithub::start();
    let args = ["-C", dir.to_str().unwrap(), "status"];
    let (code, v) = run(&fake, "session_a", "2026-10-22T10:00:00Z", &args);
    assert_eq!(code, 0, "{v}");
    v
}

#[test]
fn uc_m7_5_status_shows_reduced_integrity_for_a_regex_stack() {
    let tmp = status_dir();
    let v = status(tmp.path());
    assert_eq!(v["data"]["integrity"], "reduced");
    assert_eq!(v["data"]["integrity_reason"], REASON);
}

#[test]
fn uc_m7_5_status_stays_full_when_a_custom_profile_uses_built_in_adapters() {
    let tmp = status_dir();
    let builtin = PY_PROFILE
        .replace("\"none\"", "\"cargo-metadata\"")
        .replace("pending = \"regex\"", "pending = \"rust-ignore\"")
        .replace("assertions = \"regex\"", "assertions = \"rust\"");
    write(tmp.path(), ".bosun/profiles/python.toml", &builtin);
    let v = status(tmp.path());
    assert_eq!(v["data"]["integrity"], "full");
    assert!(v["data"].get("integrity_reason").is_none());
}

#[test]
fn uc_m7_5_test_integrity_runs_the_regex_adapters_and_reports_reduced() {
    let r = repo();
    write(
        &r.root,
        "py/test_app.py",
        "import pytest\n@pytest.mark.skip\ndef test_a():\n    assert 1\n",
    );
    git(&r.root, &["add", "."]);
    git(&r.root, &["commit", "-q", "-m", "weaken"]);
    let (code, v) = r.integrity();
    assert_eq!(code, 1, "{v}");
    assert_eq!(v["data"]["integrity"], "reduced");
    assert_eq!(v["data"]["integrity_reason"], REASON);
    let kinds: Vec<&str> = v["data"]["findings"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| f["kind"].as_str().unwrap())
        .collect();
    assert_eq!(kinds, ["new-skip", "fewer-asserts"]);
}

#[test]
fn uc_m7_5_test_integrity_passes_an_unchanged_regex_stack() {
    let r = repo();
    r.change(&["py/app.py"]);
    let (code, v) = r.integrity();
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["integrity"], "reduced");
}

/// A repo with a regex stack at the repo root and `inner` (path, profile, profile text) below it,
/// with `files` committed on `main`. Returns the repo on a fresh branch.
fn nested(inner: (&str, &str, &str), files: &[(&str, &str)]) -> Repo {
    let r = repo();
    git(&r.root, &["checkout", "-q", "main"]);
    let (path, profile, profile_text) = inner;
    let config = format!(
        "[[stack]]\npath = \".\"\nprofile = \"python\"\n[[stack]]\npath = \"{path}\"\nprofile = \"{profile}\"\n"
    );
    write(&r.root, ".bosun/bosun.toml", &config);
    write(&r.root, ".bosun/profiles/python.toml", ROOT_PY);
    write(
        &r.root,
        &format!(".bosun/profiles/{profile}.toml"),
        profile_text,
    );
    for (rel, text) in files {
        write(&r.root, rel, text);
    }
    git(&r.root, &["add", "."]);
    git(&r.root, &["commit", "-q", "-m", "nested"]);
    git(&r.root, &["checkout", "-q", "-b", "work2"]);
    r
}

const ROOT_PY: &str = "[adapters]\npackages = \"none\"\npending = \"regex\"\nassertions = \"regex\"\n\
                       [adapters.regex]\ntest_files = [\"**/test_*.py\"]\npending = ['pytest\\.mark\\.skip']\nassertions = ['\\bassert\\b']\n\
                       [gates.lint]\ncmd = [\"fakepy\", \"lint\"]\n";

fn kinds(v: &Value) -> Vec<String> {
    v["data"]["findings"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| f["kind"].as_str().unwrap().to_string())
        .collect()
}

fn commit_all(r: &Repo) {
    git(&r.root, &["add", "."]);
    git(&r.root, &["commit", "-q", "-m", "change"]);
}

#[test]
fn uc_m7_5_a_regex_stack_at_the_root_does_not_claim_a_rust_stack_below() {
    let rust = "[adapters]\npackages = \"cargo-metadata\"\npending = \"rust-ignore\"\nassertions = \"rust\"\n[gates.fmt]\ncmd = [\"cargo\", \"fmt\"]\n";
    let test = "#[test]\nfn t() {\n    assert!(true);\n    assert_eq!(1, 1);\n}\n";
    let r = nested(("crates/", "rust", rust), &[("crates/a/tests/it.rs", test)]);
    let weak = "#[test]\nfn t() {\n    assert!(true);\n}\n";
    write(&r.root, "crates/a/tests/it.rs", weak);
    commit_all(&r);
    let (code, v) = r.integrity();
    assert_eq!(code, 1, "{v}");
    assert_eq!(kinds(&v), ["fewer-asserts"]);
}

#[test]
fn uc_m7_5_a_regex_stack_at_the_root_does_not_claim_a_ts_stack_below() {
    let ts = "[adapters]\npackages = \"none\"\npending = \"ts-skip\"\nassertions = \"ts\"\n[gates.fmt]\ncmd = [\"x\"]\n";
    let spec = "it('a', () => {\n  expect(1).toBe(1);\n});\n";
    let r = nested(("web/", "ts", ts), &[("web/a.test.ts", spec)]);
    write(
        &r.root,
        "web/a.test.ts",
        "it.skip('a', () => {\n  expect(1).toBe(1);\n});\n",
    );
    commit_all(&r);
    let (code, v) = r.integrity();
    assert_eq!(code, 1, "{v}");
    assert_eq!(kinds(&v), ["new-skip"]);
}

#[test]
fn uc_m7_5_an_unknown_adapter_name_is_never_full() {
    let typo = PY_PROFILE.replace("pending = \"regex\"", "pending = \"regexp\"");
    // status shows reduced, with the reason.
    let tmp = status_dir();
    write(tmp.path(), ".bosun/profiles/python.toml", &typo);
    let v = status(tmp.path());
    assert_eq!(v["data"]["integrity"], "reduced");
    let why = v["data"]["integrity_reason"].as_str().unwrap();
    assert!(why.contains("stack py/") && why.contains("regexp"), "{why}");
    // test-integrity and the gate refuse, with a fix that lists the valid names.
    let r = repo();
    write(&r.root, ".bosun/profiles/python.toml", &typo);
    r.change(&["py/app.py"]);
    let (code, v) = r.integrity();
    assert_eq!(code, 1, "{v}");
    let fix = v["error"]["fix"].as_str().unwrap();
    assert!(fix.contains("rust-ignore, ts-skip, regex"), "{fix}");
    let out = bosun()
        .env_remove("GITHUB_BASE_REF")
        .arg("-C")
        .arg(&r.root)
        .args(["gate", "lint", "--base", "main"])
        .output()
        .unwrap();
    let v = one_object(&out.stdout);
    assert_eq!(out.status.code(), Some(1), "{v}");
    assert!(v["error"]["message"].as_str().unwrap().contains("regexp"));
}
