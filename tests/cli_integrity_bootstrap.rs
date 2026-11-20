//! test-integrity in the bootstrap PR (spec §7): when the base has no `.bosun/`, template-shipped
//! tests may go; a base `.bosun/template-tests` lists such tests for later.

mod common;

use common::{bosun, one_object};
use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

struct Repo {
    _tmp: tempfile::TempDir,
    root: PathBuf,
}

fn git(root: &Path, args: &[&str]) {
    let out = Command::new("git")
        .current_dir(root)
        .args(args)
        .output()
        .expect("git");
    assert!(out.status.success(), "git {args:?}: {out:?}");
}

const TWO_TESTS: &str =
    "#[test]\nfn a() {\n    assert!(true);\n}\n\n#[test]\nfn b() {\n    let _ = 1;\n}\n";
const ONE_TEST: &str = "#[test]\nfn a() {\n    assert!(true);\n}\n";

/// Base `main` holds the template tests and `base_files`; branch `work` starts there.
fn repo(base_files: &[(&str, &str)]) -> Repo {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().canonicalize().unwrap().join("repo");
    fs::create_dir_all(&root).unwrap();
    git(&root, &["init", "-q", "-b", "main"]);
    git(&root, &["config", "user.name", "t"]);
    git(&root, &["config", "user.email", "t@example.com"]);
    let r = Repo { _tmp: tmp, root };
    r.put("tests/t.rs", TWO_TESTS);
    r.put("tests/other.rs", ONE_TEST);
    for (p, t) in base_files {
        r.put(p, t);
    }
    git(&r.root, &["add", "."]);
    git(&r.root, &["commit", "-q", "-m", "base"]);
    git(&r.root, &["checkout", "-q", "-b", "work"]);
    r
}

impl Repo {
    fn put(&self, rel: &str, text: &str) {
        let p = self.root.join(rel);
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(p, text).unwrap();
    }

    fn commit(&self) {
        git(&self.root, &["add", "-A"]);
        git(&self.root, &["commit", "-q", "-m", "change"]);
    }

    fn check(&self) -> (i32, Value) {
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
}

fn finding<'a>(v: &'a Value, kind: &str) -> &'a Value {
    v["data"]["findings"]
        .as_array()
        .unwrap()
        .iter()
        .find(|f| f["kind"] == kind)
        .unwrap_or_else(|| panic!("no {kind} in {v}"))
}

const BOOTSTRAP: &str = "bootstrap: base has no .bosun";

#[test]
fn bootstrap_may_delete_a_template_test_file() {
    let r = repo(&[]);
    fs::remove_file(r.root.join("tests/t.rs")).unwrap();
    r.commit();
    let (code, v) = r.check();
    assert_eq!(code, 0, "{v}");
    let f = finding(&v, "deleted-test");
    assert_eq!(f["allowed"], true);
    assert_eq!(f["reason"], BOOTSTRAP);
}

#[test]
fn bootstrap_may_drop_a_test_from_a_file() {
    let r = repo(&[]);
    r.put("tests/t.rs", ONE_TEST);
    r.commit();
    let (code, v) = r.check();
    assert_eq!(code, 0, "{v}");
    let f = finding(&v, "fewer-tests");
    assert_eq!(f["allowed"], true);
    assert_eq!(f["reason"], BOOTSTRAP);
}

#[test]
fn bootstrap_still_refuses_a_new_skip_and_fewer_asserts() {
    let r = repo(&[]);
    r.put(
        "tests/t.rs",
        "#[test]\n#[ignore]\nfn a() {\n    let _ = 1;\n}\n\n#[test]\nfn b() {\n    let _ = 1;\n}\n",
    );
    r.commit();
    let (code, v) = r.check();
    assert_eq!(code, 1, "{v}");
    assert_eq!(finding(&v, "new-skip")["allowed"], false);
    assert_eq!(finding(&v, "fewer-asserts")["allowed"], false);
}

#[test]
fn with_a_bosun_dir_in_the_base_a_deleted_test_is_refused() {
    let r = repo(&[(".bosun/bosun.toml", "")]);
    fs::remove_file(r.root.join("tests/t.rs")).unwrap();
    r.commit();
    let (code, v) = r.check();
    assert_eq!(code, 1, "{v}");
    assert_eq!(finding(&v, "deleted-test")["allowed"], false);
}

#[test]
fn a_base_template_tests_file_allows_the_listed_paths_only() {
    let r = repo(&[(
        ".bosun/template-tests",
        "# shipped by the template\ntests/t.rs\n",
    )]);
    fs::remove_file(r.root.join("tests/t.rs")).unwrap();
    r.commit();
    let (code, v) = r.check();
    assert_eq!(code, 0, "{v}");
    let f = finding(&v, "deleted-test");
    assert_eq!(f["allowed"], true);
    assert_eq!(
        f["reason"],
        "template-tests: listed in base .bosun/template-tests"
    );

    fs::remove_file(r.root.join("tests/other.rs")).unwrap();
    r.commit();
    let (code, v) = r.check();
    assert_eq!(code, 1, "{v}");
}

#[test]
fn a_template_tests_file_added_in_the_pr_allows_nothing() {
    let r = repo(&[(".bosun/bosun.toml", "")]);
    r.put(".bosun/template-tests", "tests/**\n");
    fs::remove_file(r.root.join("tests/t.rs")).unwrap();
    r.commit();
    let (code, v) = r.check();
    assert_eq!(code, 1, "{v}");
    assert_eq!(finding(&v, "deleted-test")["allowed"], false);
}

#[test]
fn a_head_that_deletes_bosun_does_not_make_itself_a_bootstrap() {
    let r = repo(&[(".bosun/bosun.toml", "")]);
    fs::remove_dir_all(r.root.join(".bosun")).unwrap();
    fs::remove_file(r.root.join("tests/t.rs")).unwrap();
    r.commit();
    let (code, v) = r.check();
    assert_eq!(code, 1, "{v}");
    assert_eq!(finding(&v, "deleted-test")["allowed"], false);
}
