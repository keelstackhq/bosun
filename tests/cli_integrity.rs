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

const TWO_ASSERTS: &str = "#[test]\nfn t() {\n    assert!(true);\n    assert_eq!(1, 1);\n}\n";

/// A repo on branch `work` (base `main`) with `tests/t.rs`, `tests/u.rs` (pending UC) and a snapshot.
fn repo() -> Repo {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().canonicalize().unwrap().join("repo");
    fs::create_dir_all(&root).unwrap();
    git(&root, &["init", "-q", "-b", "main"]);
    git(&root, &["config", "user.name", "t"]);
    git(&root, &["config", "user.email", "t@example.com"]);
    let r = Repo { _tmp: tmp, root };
    r.put("tests/t.rs", TWO_ASSERTS);
    r.put(
        "tests/u.rs",
        "#[test]\n#[ignore = \"UC-M1-2\"]\nfn u() {\n    assert!(true);\n}\n",
    );
    r.put("tests/snapshots/a.snap", "one\n");
    r.put("src/lib.rs", "pub fn f() {}\n");
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

    fn edit(&self, rel: &str, text: &str) {
        self.put(rel, text);
        self.commit();
    }

    fn check(&self, env: &[(&str, &str)]) -> (i32, Value) {
        let mut cmd = bosun();
        cmd.env_remove("GITHUB_BASE_REF")
            .env_remove("BOSUN_PR_BODY")
            .env_remove("BOSUN_INTEGRITY_ALLOW");
        for (k, v) in env {
            cmd.env(k, v);
        }
        let out = cmd
            .arg("-C")
            .arg(&self.root)
            .args(["test-integrity", "--base", "main"])
            .output()
            .unwrap();
        (out.status.code().unwrap(), one_object(&out.stdout))
    }
}

fn kinds(v: &Value) -> Vec<String> {
    v["data"]["findings"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| f["kind"].as_str().unwrap().to_string())
        .collect()
}

#[test]
fn unchanged_tests_pass() {
    let r = repo();
    r.edit("src/lib.rs", "pub fn f() { /* x */ }\n");
    let (code, v) = r.check(&[]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["base"], "main");
    assert_eq!(v["data"]["findings"], serde_json::json!([]));
}

#[test]
fn deleted_test_file_is_refused() {
    let r = repo();
    fs::remove_file(r.root.join("tests/t.rs")).unwrap();
    r.commit();
    let (code, v) = r.check(&[]);
    assert_eq!(code, 1, "{v}");
    assert_eq!(kinds(&v), vec!["deleted-test"]);
    assert_eq!(v["data"]["findings"][0]["file"], "tests/t.rs");
    assert_eq!(v["data"]["findings"][0]["allowed"], false);
    assert_eq!(v["error"]["message"], "test-integrity: 1 findings");
    assert!(!v["error"]["fix"].as_str().unwrap().is_empty());
}

const RS_WITH_TEST: &str = "pub fn f() {}\n#[cfg(test)]\nmod tests {\n    #[test]\n    fn t() {\n        assert!(true);\n    }\n}\n";

#[test]
fn rs_file_losing_all_its_tests_is_a_deleted_test() {
    let r = repo();
    r.edit("src/lib.rs", RS_WITH_TEST);
    git(&r.root, &["checkout", "-q", "-b", "work2"]);
    r.edit("src/lib.rs", "pub fn f() {}\n");
    let out = bosun()
        .arg("-C")
        .arg(&r.root)
        .args(["test-integrity", "--base", "work"])
        .env_remove("BOSUN_INTEGRITY_ALLOW")
        .output()
        .unwrap();
    let v = one_object(&out.stdout);
    assert_eq!(out.status.code(), Some(1), "{v}");
    assert_eq!(kinds(&v), vec!["deleted-test"]);
    assert_eq!(v["data"]["findings"][0]["file"], "src/lib.rs");
}

#[test]
fn allow_list_lets_a_deleted_test_through() {
    let r = repo();
    fs::remove_file(r.root.join("tests/t.rs")).unwrap();
    r.commit();
    let (code, v) = r.check(&[("BOSUN_INTEGRITY_ALLOW", "delete-test")]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["findings"][0]["kind"], "deleted-test");
    assert_eq!(v["data"]["findings"][0]["allowed"], true);
}

#[test]
fn renamed_test_file_is_not_a_deletion() {
    let r = repo();
    git(&r.root, &["mv", "tests/t.rs", "tests/renamed.rs"]);
    r.commit();
    let (code, v) = r.check(&[]);
    assert_eq!(code, 0, "{v}");
    assert!(v["data"]["files_checked"].as_u64().unwrap() >= 1);
}

#[test]
fn added_ignore_is_refused() {
    let r = repo();
    r.edit("tests/t.rs", &format!("#[ignore]\n{TWO_ASSERTS}"));
    let (code, v) = r.check(&[]);
    assert_eq!(code, 1, "{v}");
    assert_eq!(kinds(&v), vec!["new-skip"]);
    let (code, _) = r.check(&[("BOSUN_INTEGRITY_ALLOW", "skip")]);
    assert_eq!(code, 0);
}

#[test]
fn added_uc_ignore_is_a_skip_unless_allowed() {
    let r = repo();
    r.edit(
        "tests/t.rs",
        &format!("#[ignore = \"UC-M1-3\"]\n{TWO_ASSERTS}"),
    );
    let (code, v) = r.check(&[]);
    assert_eq!(code, 1, "{v}");
    assert_eq!(kinds(&v), vec!["new-skip"]);
    let (code, _) = r.check(&[("BOSUN_INTEGRITY_ALLOW", "skip")]);
    assert_eq!(code, 0);
}

#[test]
fn existing_uc_marker_staying_put_is_fine() {
    let r = repo();
    r.edit(
        "tests/u.rs",
        "// moved\n#[test]\n#[ignore = \"UC-M1-2\"]\nfn u() {\n    assert!(true);\n}\n",
    );
    let (code, v) = r.check(&[]);
    assert_eq!(code, 0, "{v}");
}

#[test]
fn dropped_test_attribute_is_refused() {
    let r = repo();
    r.edit(
        "tests/t.rs",
        "fn t() {\n    assert!(true);\n    assert_eq!(1, 1);\n}\n",
    );
    let (code, v) = r.check(&[]);
    assert_eq!(code, 1, "{v}");
    assert_eq!(kinds(&v), vec!["fewer-tests"]);
    let (code, _) = r.check(&[("BOSUN_INTEGRITY_ALLOW", "delete-test")]);
    assert_eq!(code, 0);
}

#[test]
fn deleting_a_test_without_asserts_is_refused() {
    let r = repo();
    // main gains a second test with no assertions; work starts from there and deletes it.
    r.edit(
        "tests/t.rs",
        &format!("{TWO_ASSERTS}#[test]\nfn smoke() {{\n    run();\n}}\n"),
    );
    git(&r.root, &["checkout", "-q", "main"]);
    git(&r.root, &["merge", "-q", "--ff-only", "work"]);
    git(&r.root, &["checkout", "-q", "-b", "work2"]);
    r.edit("tests/t.rs", TWO_ASSERTS);
    let (code, v) = r.check(&[]);
    assert_eq!(code, 1, "{v}");
    assert_eq!(kinds(&v), vec!["fewer-tests"]);
}

#[test]
fn branch_behind_main_gets_no_false_fewer_asserts() {
    let r = repo();
    // main moves on: t.rs now has five assertions.
    git(&r.root, &["checkout", "-q", "main"]);
    r.put(
        "tests/t.rs",
        "#[test]\nfn t() {\n    assert!(true);\n    assert!(true);\n    assert!(true);\n    assert!(true);\n    assert!(true);\n}\n",
    );
    r.commit();
    git(&r.root, &["checkout", "-q", "work"]);
    // work, cut before that, adds one assertion to its own copy (two -> three).
    r.edit(
        "tests/t.rs",
        "#[test]\nfn t() {\n    assert!(true);\n    assert_eq!(1, 1);\n    assert_ne!(1, 2);\n}\n",
    );
    let (code, v) = r.check(&[]);
    assert_eq!(code, 0, "{v}");
}

#[test]
fn removing_a_uc_marker_passes() {
    let r = repo();
    r.edit("tests/u.rs", "#[test]\nfn u() {\n    assert!(true);\n}\n");
    let (code, v) = r.check(&[]);
    assert_eq!(code, 0, "{v}");
}

#[test]
fn fewer_asserts_are_refused_and_more_are_fine() {
    let r = repo();
    r.edit("tests/t.rs", "#[test]\nfn t() {\n    assert!(true);\n}\n");
    let (code, v) = r.check(&[]);
    assert_eq!(code, 1, "{v}");
    assert_eq!(kinds(&v), vec!["fewer-asserts"]);
    assert_eq!(v["data"]["findings"][0]["base"], 2);
    assert_eq!(v["data"]["findings"][0]["head"], 1);
    let (code, _) = r.check(&[("BOSUN_INTEGRITY_ALLOW", "fewer-asserts")]);
    assert_eq!(code, 0);

    r.edit(
        "tests/t.rs",
        "#[test]\nfn t() {\n    assert!(true);\n    assert_eq!(1, 1);\n    assert_ne!(1, 2);\n}\n",
    );
    let (code, v) = r.check(&[]);
    assert_eq!(code, 0, "{v}");
}

#[test]
fn snapshot_change_needs_a_reason_in_the_pr_body() {
    let r = repo();
    r.edit("tests/snapshots/a.snap", "two\n");
    let (code, v) = r.check(&[]);
    assert_eq!(code, 1, "{v}");
    assert_eq!(kinds(&v), vec!["snapshot-change"]);
    let (code, _) = r.check(&[("BOSUN_PR_BODY", "text\nsnapshot-change:   \n")]);
    assert_eq!(code, 1);
    let (code, v) = r.check(&[("BOSUN_PR_BODY", "text\nsnapshot-change: new field\n")]);
    assert_eq!(code, 0, "{v}");
}

#[test]
fn stdout_is_one_json_object() {
    let r = repo();
    let out = bosun()
        .env_remove("GITHUB_BASE_REF")
        .arg("-C")
        .arg(&r.root)
        .args(["gate", "test-integrity", "--base", "main"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(0));
    let v = one_object(&out.stdout);
    assert_eq!(v["command"], "gate");
    assert_eq!(v["data"]["files_checked"], 0);
}
