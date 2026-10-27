//! Unix only: the fakes are shell scripts made executable with unix permissions.
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

/// A two-package workspace on branch `work` (base `main`), plus a fake `cargo` that logs its argv.
fn repo() -> Repo {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().canonicalize().unwrap().join("repo");
    fs::create_dir_all(&root).unwrap();
    git(&root, &["init", "-q", "-b", "main"]);
    git(&root, &["config", "user.name", "t"]);
    git(&root, &["config", "user.email", "t@example.com"]);
    write(
        &root,
        "Cargo.toml",
        "[workspace]\nmembers = [\"a\", \"b\"]\n",
    );
    write(&root, "a/Cargo.toml", "[package]\nname = \"a\"\n");
    write(&root, "a/src/lib.rs", "");
    write(&root, "b/Cargo.toml", "[package]\nname = \"b\"\n");
    write(&root, "b/src/lib.rs", "");
    write(&root, "docs/x.md", "x\n");
    git(&root, &["add", "."]);
    git(&root, &["commit", "-q", "-m", "base"]);
    git(&root, &["checkout", "-q", "-b", "work"]);
    let bin = tmp.path().join("bin");
    fs::create_dir_all(&bin).unwrap();
    let log = tmp.path().join("cargo.log");
    let script = format!(
        "#!/bin/sh\necho \"$@\" >> {log}\n\
         if [ \"$1\" = metadata ]; then\n\
         printf '{{\"packages\":[{{\"name\":\"a\",\"manifest_path\":\"{root}/a/Cargo.toml\"}},{{\"name\":\"b\",\"manifest_path\":\"{root}/b/Cargo.toml\"}}]}}'\n\
         exit 0\nfi\n\
         echo fake cargo stdout\nexit ${{FAKE_CARGO_EXIT:-0}}\n",
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
    fn change(&self, rel: &str) {
        write(&self.root, rel, "// changed\n");
        git(&self.root, &["add", "."]);
        git(&self.root, &["commit", "-q", "-m", "change"]);
    }

    fn gate(&self, args: &[&str], exit: &str) -> (i32, Value, String) {
        let path = format!(
            "{}:{}",
            self.bin.display(),
            std::env::var("PATH").unwrap_or_default()
        );
        let out = bosun()
            .env("PATH", path)
            .env("FAKE_CARGO_EXIT", exit)
            .env_remove("GITHUB_BASE_REF")
            .arg("-C")
            .arg(&self.root)
            .arg("gate")
            .args(args)
            .output()
            .unwrap();
        let v = one_object(&out.stdout);
        (
            out.status.code().unwrap(),
            v,
            String::from_utf8_lossy(&out.stderr).into_owned(),
        )
    }

    /// Logged cargo calls other than `metadata`.
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
fn change_in_one_package_tests_only_that_package() {
    let r = repo();
    r.change("b/src/lib.rs");
    let (code, v, _) = r.gate(&["test", "--base", "main"], "0");
    assert_eq!(code, 0, "{v}");
    assert_eq!(r.calls(), vec!["nextest run --locked -p b"]);
    assert_eq!(v["data"]["packages"], serde_json::json!(["b"]));
    assert_eq!(v["data"]["gate"], "test");
    assert_eq!(v["data"]["exit_status"], 0);
    assert_eq!(v["data"]["skipped"], false);
    assert_eq!(v["data"]["over_budget"], false);
}

#[test]
fn root_manifest_change_tests_the_workspace() {
    let r = repo();
    r.change("Cargo.toml");
    let (code, _, _) = r.gate(&["test", "--base", "main"], "0");
    assert_eq!(code, 0);
    assert_eq!(r.calls(), vec!["nextest run --locked --workspace"]);
}

#[test]
fn all_flag_tests_the_workspace() {
    let r = repo();
    r.change("docs/x.md");
    let (code, _, _) = r.gate(&["test", "--base", "main", "--all"], "0");
    assert_eq!(code, 0);
    assert_eq!(r.calls(), vec!["nextest run --locked --workspace"]);
}

#[test]
fn doc_only_change_skips_the_test_gate() {
    let r = repo();
    r.change("docs/x.md");
    let (code, v, _) = r.gate(&["test", "--base", "main"], "0");
    assert_eq!(code, 0);
    assert_eq!(v["data"]["skipped"], true);
    assert!(r.calls().is_empty());
}

#[test]
fn staged_and_untracked_changes_select_the_gate() {
    let r = repo();
    write(&r.root, "b/src/lib.rs", "// staged\n");
    git(&r.root, &["add", "b/src/lib.rs"]);
    let (code, v, _) = r.gate(&["test", "--base", "main"], "0");
    assert_eq!(code, 0, "{v}");
    assert_ne!(v["data"]["skipped"], true, "{v}");
    assert_eq!(r.calls(), vec!["nextest run --locked -p b"]);
    assert_eq!(
        v["data"]["uncommitted"],
        serde_json::json!(["b/src/lib.rs"])
    );

    let r = repo();
    write(&r.root, "a/src/new.rs", "// untracked\n");
    let (code, v, _) = r.gate(&["test", "--base", "main"], "0");
    assert_eq!(code, 0, "{v}");
    assert_eq!(r.calls(), vec!["nextest run --locked -p a"]);
    assert_eq!(
        v["data"]["uncommitted"],
        serde_json::json!(["a/src/new.rs"])
    );
}

#[test]
fn an_unstaged_edit_of_a_tracked_file_selects_the_gate() {
    let r = repo();
    write(&r.root, "a/src/lib.rs", "// edited, not added\n");
    let (code, v, _) = r.gate(&["test", "--base", "main"], "0");
    assert_eq!(code, 0, "{v}");
    assert_eq!(r.calls(), vec!["nextest run --locked -p a"]);
    assert_eq!(
        v["data"]["uncommitted"],
        serde_json::json!(["a/src/lib.rs"])
    );
    // A doc edit affects no package: the gate is skipped, and the edit is still listed.
    let r = repo();
    write(&r.root, "docs/x.md", "edited\n");
    let (_, v, _) = r.gate(&["test", "--base", "main"], "0");
    assert_eq!(v["data"]["skipped"], true, "{v}");
    assert_eq!(v["data"]["uncommitted"], serde_json::json!(["docs/x.md"]));
}

#[test]
fn the_diff_file_includes_uncommitted_edits() {
    let r = repo();
    let out = r.root.join("seen.diff");
    let profile = format!(
        "[adapters]\npackages = \"cargo-metadata\"\npending = \"rust-ignore\"\nassertions = \"rust\"\n[gates.mutants]\ncmd = [\"cp\", \"{{diff}}\", \"{}\"]\nbudget_secs = 5\n",
        out.display()
    );
    write(&r.root, ".bosun/profiles/rust.toml", &profile);
    write(&r.root, "a/src/lib.rs", "// only in the working tree\n");
    let (code, v, _) = r.gate(&["mutants", "--base", "main"], "0");
    assert_eq!(code, 0, "{v}");
    let seen = fs::read_to_string(out).unwrap();
    assert!(seen.contains("only in the working tree"), "{seen}");
}

#[test]
fn failing_command_is_exit_1_with_status() {
    let r = repo();
    r.change("a/src/lib.rs");
    let (code, v, _) = r.gate(&["test", "--base", "main"], "3");
    assert_eq!(code, 1);
    assert_eq!(v["data"]["exit_status"], 3);
    assert_eq!(v["error"]["message"], "gate test failed (exit 3)");
    assert_eq!(
        v["error"]["fix"],
        "fix the failures above, then rerun `bosun gate test`"
    );
}

#[test]
fn unknown_gate_is_usage_and_lists_gates() {
    let r = repo();
    r.change("a/src/lib.rs");
    let (code, v, _) = r.gate(&["nope", "--base", "main"], "0");
    assert_eq!(code, 2);
    let fix = v["error"]["fix"].as_str().unwrap();
    assert!(fix.contains("fmt") && fix.contains("test"), "{fix}");
}

const FMT_ONLY: &str = "[adapters]\npackages = \"cargo-metadata\"\npending = \"rust-ignore\"\nassertions = \"rust\"\n[gates.fmt]\ncmd = [\"cargo\", \"fmt\", \"--all\"]\nbudget_secs = 1\n";

#[test]
fn gate_the_profile_lacks_is_usage() {
    let r = repo();
    write(&r.root, ".bosun/profiles/rust.toml", FMT_ONLY);
    r.change("a/src/lib.rs");
    let (code, _, _) = r.gate(&["lint", "--base", "main"], "0");
    assert_eq!(code, 2);
}

#[test]
fn repo_profile_overrides_the_built_in() {
    let r = repo();
    write(&r.root, ".bosun/profiles/rust.toml", FMT_ONLY);
    r.change("a/src/lib.rs");
    let (code, _, _) = r.gate(&["fmt", "--base", "main"], "0");
    assert_eq!(code, 0);
    assert_eq!(r.calls(), vec!["fmt --all"]);
}

#[test]
fn stdout_stays_one_json_object_and_child_output_goes_to_stderr() {
    let r = repo();
    r.change("a/src/lib.rs");
    let (code, v, err) = r.gate(&["fmt", "--base", "main"], "0");
    assert_eq!(code, 0);
    assert_eq!(v["command"], "gate");
    assert!(err.contains("fake cargo stdout"), "{err}");
}

#[test]
fn bad_base_is_exit_1() {
    let r = repo();
    let (code, v, _) = r.gate(&["fmt", "--base", "no-such-ref"], "0");
    assert_eq!(code, 1);
    assert!(!v["error"]["fix"].as_str().unwrap().is_empty());
}

/// A repo with only `.bosun/bosun.toml` (rust stack at `.`) and one commit on `work`.
fn empty_repo() -> Repo {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().canonicalize().unwrap().join("repo");
    fs::create_dir_all(&root).unwrap();
    git(&root, &["init", "-q", "-b", "main"]);
    git(&root, &["config", "user.name", "t"]);
    git(&root, &["config", "user.email", "t@example.com"]);
    write(
        &root,
        ".bosun/bosun.toml",
        "[[stack]]\npath = \".\"\nprofile = \"rust\"\n",
    );
    git(&root, &["add", "."]);
    git(&root, &["commit", "-q", "-m", "base"]);
    git(&root, &["checkout", "-q", "-b", "work"]);
    let bin = tmp.path().join("bin");
    let log = tmp.path().join("cargo.log");
    let r = Repo {
        _tmp: tmp,
        root,
        bin,
        log,
    };
    r.change("README.md");
    r
}

#[test]
fn stack_without_manifest_is_skipped() {
    let r = empty_repo();
    for extra in [&[][..], &["--all"][..]] {
        let mut args = vec!["fmt", "--base", "main"];
        args.extend_from_slice(extra);
        let (code, v, _) = r.gate(&args, "0");
        assert_eq!(code, 0, "{v}");
        assert_eq!(v["data"]["skipped"], true);
        assert_eq!(v["data"]["exit_status"], 0);
        assert!(v["data"]["reason"].as_str().unwrap().contains("Cargo.toml"));
    }
}

#[test]
fn deleting_the_manifest_still_runs_the_gate() {
    let r = repo();
    git(&r.root, &["rm", "-q", "Cargo.toml"]);
    git(&r.root, &["commit", "-q", "-m", "delete manifest"]);
    let (code, v, _) = r.gate(&["fmt", "--base", "main"], "1");
    assert_eq!(code, 1, "{v}");
    assert_eq!(v["data"]["skipped"], false);
    assert_eq!(r.calls(), vec!["fmt --check"]);
    // The merge queue and main run with --all; the base still has the manifest.
    let (code, v, _) = r.gate(&["fmt", "--base", "main", "--all"], "1");
    assert_eq!(code, 1, "{v}");
    assert_eq!(v["data"]["skipped"], false);
}

#[test]
fn a_stack_in_a_subdirectory_skips_until_its_manifest_exists() {
    let r = repo();
    write(
        &r.root,
        ".bosun/bosun.toml",
        "[[stack]]\npath = \"web\"\nprofile = \"rust\"\n",
    );
    write(&r.root, "web/README.md", "x\n");
    git(&r.root, &["add", "."]);
    git(&r.root, &["commit", "-q", "-m", "web stack"]);
    let (code, v, _) = r.gate(&["fmt", "--base", "main"], "0");
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["reason"], "no Cargo.toml in web");
    write(&r.root, "web/Cargo.toml", "[package]\nname = \"web\"\n");
    git(&r.root, &["add", "."]);
    git(&r.root, &["commit", "-q", "-m", "web crate"]);
    let (code, v, _) = r.gate(&["fmt", "--base", "main"], "0");
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["skipped"], false);
    assert_eq!(r.calls(), vec!["fmt --check"]);
}

const KEYS: &str = "[adapters]\npackages = \"cargo-metadata\"\npending = \"rust-ignore\"\nassertions = \"rust\"\n\
[gates.test]\ncmd = [\"cargo\", \"{packages}\", \"run\", \"it\"]\neach_package = [\"--filter\", \"{package}\"]\nall = [\"--all\"]\n\
[gates.deps]\ncmd = [\"cargo\", \"deny\", \"check\"]\nrequires = \"Cargo.lock\"\n";

#[test]
fn packages_placeholder_puts_the_package_args_in_the_middle() {
    let r = repo();
    write(&r.root, ".bosun/profiles/rust.toml", KEYS);
    git(&r.root, &["add", "."]);
    git(&r.root, &["commit", "-q", "-m", "profile"]);
    git(&r.root, &["branch", "-f", "main", "work"]);
    r.change("a/src/lib.rs");
    r.change("b/src/lib.rs");
    let (code, v, _) = r.gate(&["test", "--base", "main"], "0");
    assert_eq!(code, 0, "{v}");
    assert_eq!(r.calls(), vec!["--filter a --filter b run it"]);
}

#[test]
fn requires_skips_a_gate_whose_file_never_existed() {
    let r = repo();
    write(&r.root, ".bosun/profiles/rust.toml", KEYS);
    r.change("a/src/lib.rs");
    for args in [
        &["deps", "--base", "main"][..],
        &["deps", "--base", "main", "--all"],
    ] {
        let (code, v, _) = r.gate(args, "0");
        assert_eq!(code, 0, "{v}");
        assert_eq!(v["data"]["skipped"], true);
        assert_eq!(v["data"]["reason"], "no Cargo.lock");
    }
    assert!(r.calls().is_empty());
    write(&r.root, "Cargo.lock", "# lock\n");
    git(&r.root, &["add", "."]);
    git(&r.root, &["commit", "-q", "-m", "lock"]);
    let (code, v, _) = r.gate(&["deps", "--base", "main"], "0");
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["skipped"], false);
    assert_eq!(r.calls(), vec!["deny check"]);
}

#[test]
fn deleting_the_required_file_still_runs_the_gate() {
    let r = repo();
    write(&r.root, ".bosun/profiles/rust.toml", KEYS);
    write(&r.root, "Cargo.lock", "# lock\n");
    git(&r.root, &["add", "."]);
    git(&r.root, &["commit", "-q", "-m", "lock"]);
    git(&r.root, &["branch", "-f", "main", "work"]);
    git(&r.root, &["rm", "-q", "Cargo.lock"]);
    git(&r.root, &["commit", "-q", "-m", "delete lock"]);
    let (code, v, _) = r.gate(&["deps", "--base", "main"], "1");
    assert_eq!(code, 1, "{v}");
    assert_eq!(v["data"]["skipped"], false);
    let (code, v, _) = r.gate(&["deps", "--base", "main", "--all"], "1");
    assert_eq!(code, 1, "{v}");
    assert_eq!(v["data"]["skipped"], false);
    assert_eq!(r.calls(), vec!["deny check", "deny check"]);
}
