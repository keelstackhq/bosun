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

const TEST_TS: &str =
    "describe('x', () => {\n  it('a', () => {\n    expect(1).toBe(1);\n  });\n});\n";

/// A repo on branch `work` (base `main`) with `src/a.test.ts`.
fn repo() -> Repo {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().canonicalize().unwrap().join("repo");
    fs::create_dir_all(&root).unwrap();
    git(&root, &["init", "-q", "-b", "main"]);
    git(&root, &["config", "user.name", "t"]);
    git(&root, &["config", "user.email", "t@example.com"]);
    let r = Repo { _tmp: tmp, root };
    r.put("src/a.test.ts", TEST_TS);
    r.put("src/a.ts", "export const a = 1;\n");
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

#[test]
fn new_it_skip_in_a_ts_test_is_refused() {
    let r = repo();
    r.edit("src/a.test.ts", &TEST_TS.replace("it('a'", "it.skip('a'"));
    let (code, v) = r.check(&[]);
    assert_eq!(code, 1, "{v}");
    assert_eq!(v["data"]["findings"][0]["kind"], "new-skip");
    assert_eq!(v["data"]["findings"][0]["file"], "src/a.test.ts");
    let (code, _) = r.check(&[("BOSUN_INTEGRITY_ALLOW", "skip")]);
    assert_eq!(code, 0);
}

#[test]
fn removed_expect_in_a_ts_test_is_refused() {
    let r = repo();
    r.edit("src/a.test.ts", &TEST_TS.replace("expect(1).toBe(1);", ""));
    let (code, v) = r.check(&[]);
    assert_eq!(code, 1, "{v}");
    assert_eq!(v["data"]["findings"][0]["kind"], "fewer-asserts");
}

#[test]
fn unchanged_ts_tests_pass() {
    let r = repo();
    r.edit("src/a.ts", "export const a = 2;\n");
    let (code, v) = r.check(&[]);
    assert_eq!(code, 0, "{v}");
}
