//! `bosun prereqs` with no file named reads `plans/*.md` (else `docs/superpowers/plans/*.md`),
//! and without `--post` needs no GitHub token.
mod common;

use common::bosun;
use serde_json::Value;
use std::path::Path;

const PLAN: &str = "# Plan: Ship it\n\nproject: x\n\n\
## M1: One\n\narea: core\ndepends: seed\n\nWords.\n\n### Tasks\n\
- T1: a\n  touch: docs/a.md\n- T2: b\n  touch: src/x.rs\n  depends: T1\n\
### Done when\n- UC-M1-1: works\n\n\
## Prerequisites\n\n\
- H1: npm token\n  on: phone\n  steps: add it\n  verify: secret release/NPM_TOKEN\n  blocks: M1/T2\n\
- H2: pc thing\n  on: computer\n  steps: run it\n  blocks: M1\n";

fn write(dir: &Path, rel: &str, text: &str) {
    let p = dir.join(rel);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, text).unwrap();
}

/// No token, and an API address nothing listens on.
fn prereqs(dir: &Path, extra: &[&str]) -> (i32, Value) {
    let out = bosun()
        .env_remove("GITHUB_TOKEN")
        .arg("-C")
        .arg(dir)
        .arg("prereqs")
        .args(extra)
        .output()
        .unwrap();
    let v = common::one_object(&out.stdout);
    if v["ok"] == false {
        assert!(!v["error"]["fix"].as_str().unwrap().trim().is_empty());
    }
    (out.status.code().unwrap(), v)
}

fn files(v: &Value) -> Vec<&str> {
    let plans = v["data"]["plans"].as_array().unwrap();
    plans.iter().map(|p| p["file"].as_str().unwrap()).collect()
}

#[test]
fn plans_dir_is_the_default_and_needs_no_token() {
    let tmp = tempfile::tempdir().unwrap();
    write(tmp.path(), "plans/b.md", PLAN);
    write(tmp.path(), "plans/a.md", PLAN);
    write(tmp.path(), "plans/notes.txt", "not a plan");
    write(tmp.path(), "docs/superpowers/plans/c.md", PLAN);
    let (code, v) = prereqs(tmp.path(), &["--no-setup"]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(files(&v), ["plans/a.md", "plans/b.md"]);
    let plan = &v["data"]["plans"][0];
    assert_eq!(plan["issue"], Value::Null);
    let ids: Vec<&str> = (plan["items"].as_array().unwrap().iter())
        .map(|i| i["id"].as_str().unwrap())
        .collect();
    assert!(ids.contains(&"H1") && ids.contains(&"H2"), "{ids:?}");
    // Nothing was read from GitHub, so the API item is open, not guessed.
    assert_eq!(v["data"]["open"], 4);
    let warnings = v["data"]["warnings"].as_array().unwrap();
    assert!(
        warnings
            .iter()
            .any(|w| w.as_str().unwrap().contains("no GitHub token"))
    );
}

#[test]
fn superpowers_plans_are_the_fallback() {
    let tmp = tempfile::tempdir().unwrap();
    write(tmp.path(), "docs/superpowers/plans/z.md", PLAN);
    write(tmp.path(), "docs/superpowers/plans/y.md", PLAN);
    let (code, v) = prereqs(tmp.path(), &["--no-setup"]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(
        files(&v),
        ["docs/superpowers/plans/y.md", "docs/superpowers/plans/z.md"]
    );
}

#[test]
fn no_plan_anywhere_is_a_usage_error_with_a_fix() {
    let tmp = tempfile::tempdir().unwrap();
    let (code, v) = prereqs(tmp.path(), &["--no-setup"]);
    assert_eq!(code, 2, "{v}");
    let fix = v["error"]["fix"].as_str().unwrap();
    assert!(fix.contains("plans/"), "{fix}");
}

#[test]
fn an_explicit_file_still_works_without_a_token() {
    let tmp = tempfile::tempdir().unwrap();
    write(tmp.path(), "other/p.md", PLAN);
    let (code, v) = prereqs(tmp.path(), &["--no-setup", "other/p.md"]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(files(&v), ["other/p.md"]);
}

#[test]
fn post_still_needs_a_token() {
    let tmp = tempfile::tempdir().unwrap();
    write(tmp.path(), "plans/a.md", PLAN);
    let (code, v) = prereqs(tmp.path(), &["--no-setup", "--post"]);
    assert_eq!(code, 1, "{v}");
    assert!(v["error"]["fix"].as_str().unwrap().contains("GITHUB_TOKEN"));
}
