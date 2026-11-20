//! `bosun plan import`: lenient prerequisite bullets (`data.prereqs_fixed`), `--fix`, and a
//! missing GitHub token (`--dry-run` and `--fix` work offline; a real import refuses with the
//! login fix and an empty `next`).
mod common;

use common::bosun;
use common::fake::FakeGithub;
use serde_json::{Value, json};
use std::path::Path;

const BASE: &str = "# Plan: widget\n\nproject: widget\n\n## M1: Lifecycle\n\narea: lifecycle\n\nThe machine.\n\n\
### Tasks\n\n- T1: One\n  touch: src/a.rs\n- T2: Two\n  touch: src/b.rs\n  depends: T1\n\n\
### Done when\n\n- UC-M1-1: works.\n";
const SECTION: &str = "\n## Prerequisites\n\n\
- H: token\n  on: phone\n  steps: add it\n  blocks: M1/T1\n\
* H3 - runner\n  on: computer\n  steps: set it up\n  blocks: M1/T2\n\n## Notes\n\n- H: x\n";

fn plan_text() -> String {
    format!("{BASE}{SECTION}")
}

fn write(dir: &Path) {
    std::fs::write(dir.join("plan.md"), plan_text()).unwrap();
}

fn with_token(fake: &FakeGithub, dir: &Path, extra: &[&str]) -> (i32, Value) {
    let mut a = vec!["-C", dir.to_str().unwrap(), "plan", "import", "plan.md"];
    a.extend(extra);
    common::run(fake, "session_test", "2026-10-27T10:00:00Z", &a)
}

/// No token at all, and an API address nothing listens on.
fn offline(dir: &Path, extra: &[&str]) -> (i32, Value) {
    let out = bosun()
        .env_remove("GITHUB_TOKEN")
        .arg("-C")
        .arg(dir)
        .args(["plan", "import", "plan.md"])
        .args(extra)
        .output()
        .unwrap();
    let v = common::one_object(&out.stdout);
    if v["ok"] == false {
        assert!(!v["error"]["fix"].as_str().unwrap().trim().is_empty());
    }
    (out.status.code().unwrap(), v)
}

fn fixed_rows(v: &Value) -> Vec<(u64, String, String)> {
    v["data"]["prereqs_fixed"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| {
            let s = |k: &str| r[k].as_str().unwrap().to_string();
            (r["line"].as_u64().unwrap(), s("from"), s("to"))
        })
        .collect()
}

#[test]
fn import_reads_sloppy_bullets_and_reports_what_it_fixed() {
    let (tmp, fake) = (tempfile::tempdir().unwrap(), FakeGithub::start());
    write(tmp.path());
    let (code, v) = with_token(&fake, tmp.path(), &["--dry-run"]);
    assert_eq!(code, 0, "{v}");
    let rows = fixed_rows(&v);
    let first = (BASE.lines().count() + 4) as u64;
    assert_eq!(
        rows,
        [
            (first, "- H: token".to_string(), "- H1: token".to_string()),
            (
                first + 4,
                "* H3 - runner".to_string(),
                "- H3: runner".to_string()
            ),
        ]
    );
    // The file is not touched without --fix.
    let now = std::fs::read_to_string(tmp.path().join("plan.md")).unwrap();
    assert_eq!(now, plan_text());
    assert!(v["data"].get("fixed_file").is_none());
}

#[test]
fn fix_rewrites_only_the_prerequisite_section() {
    let (tmp, fake) = (tempfile::tempdir().unwrap(), FakeGithub::start());
    write(tmp.path());
    let (code, v) = with_token(&fake, tmp.path(), &["--dry-run", "--fix"]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["fixed_file"], "plan.md");
    let now = std::fs::read_to_string(tmp.path().join("plan.md")).unwrap();
    let want = plan_text()
        .replace("- H: token", "- H1: token")
        .replace("* H3 - runner", "- H3: runner");
    assert_eq!(now, want);
    assert!(now.ends_with("## Notes\n\n- H: x\n"));
    // A second run has nothing left to fix.
    let (code, v) = with_token(&fake, tmp.path(), &["--dry-run", "--fix"]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["prereqs_fixed"], json!([]));
    assert!(v["data"].get("fixed_file").is_none());
}

#[test]
fn plan_check_fix_rewrites_the_section_too() {
    let tmp = tempfile::tempdir().unwrap();
    write(tmp.path());
    let out = bosun()
        .arg("-C")
        .arg(tmp.path())
        .args(["plan", "check", "plan.md", "--fix"])
        .output()
        .unwrap();
    let v = common::one_object(&out.stdout);
    assert_eq!(out.status.code(), Some(0), "{v}");
    assert_eq!(v["data"]["fixed_file"], "plan.md");
    assert_eq!(v["data"]["prereqs_fixed"].as_array().unwrap().len(), 2);
    let now = std::fs::read_to_string(tmp.path().join("plan.md")).unwrap();
    assert!(now.contains("- H1: token\n") && now.contains("- H3: runner\n"));
}

#[test]
fn without_a_token_dry_run_lists_every_issue_offline() {
    let tmp = tempfile::tempdir().unwrap();
    write(tmp.path());
    let (code, v) = offline(tmp.path(), &["--dry-run"]);
    assert_eq!(code, 0, "{v}");
    let d = &v["data"];
    assert_eq!(d["dry_run"], true);
    assert_eq!(d["offline"], true);
    assert_eq!(d["epics"][0]["action"], "would-create");
    let ids: Vec<&str> = d["tasks"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["id"].as_str().unwrap())
        .collect();
    assert_eq!(ids, ["M1/P", "M1/T1", "M1/T2"]);
    assert_eq!(d["prereqs_fixed"].as_array().unwrap().len(), 2);
    assert!(v["next"].as_array().unwrap().is_empty());
}

#[test]
fn without_a_token_fix_works_offline() {
    let tmp = tempfile::tempdir().unwrap();
    write(tmp.path());
    let (code, v) = offline(tmp.path(), &["--dry-run", "--fix"]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["fixed_file"], "plan.md");
    let now = std::fs::read_to_string(tmp.path().join("plan.md")).unwrap();
    assert!(now.contains("- H1: token\n"));
}

#[test]
fn without_a_token_a_real_import_refuses_with_an_empty_next() {
    let tmp = tempfile::tempdir().unwrap();
    write(tmp.path());
    let (code, v) = offline(tmp.path(), &[]);
    assert_eq!(code, 1, "{v}");
    assert_eq!(v["ok"], false);
    assert_eq!(
        v["error"]["fix"],
        "plan import writes task metadata, which only bosun may write: run `gh auth login` (bosun reads `gh auth token`) or export GITHUB_TOKEN, then rerun `bosun plan import <file>`; `--dry-run` and `--fix` work offline"
    );
    // The guard refuses issue_write for task metadata: nothing is handed over.
    assert!(v["next"].as_array().is_none_or(|n| n.is_empty()));
    assert!(!v.to_string().contains("issue_write"));
    // The would-create list stays in data: one epic, the plan task and the tasks.
    assert_eq!(v["data"]["offline"], true);
    assert_eq!(v["data"]["imported"], false);
    assert_eq!(v["data"]["epics"][0]["action"], "would-create");
    assert!(v["data"]["tasks"].as_array().unwrap().len() >= 3);
}

#[test]
fn plan_check_without_fix_warns_about_fixable_bullets_and_exits_0() {
    let tmp = tempfile::tempdir().unwrap();
    write(tmp.path());
    let out = bosun()
        .env_remove("GITHUB_TOKEN")
        .arg("-C")
        .arg(tmp.path())
        .args(["plan", "check", "plan.md"])
        .output()
        .unwrap();
    let v = common::one_object(&out.stdout);
    assert_eq!(out.status.code(), Some(0), "{v}");
    assert_eq!(v["data"]["prereqs_fixed"].as_array().unwrap().len(), 2);
    let w = v["data"]["warnings"].as_array().unwrap();
    assert!(
        w.iter()
            .any(|x| x["message"].as_str().unwrap().contains("canonical")),
        "{v}"
    );
    // The file is untouched.
    let now = std::fs::read_to_string(tmp.path().join("plan.md")).unwrap();
    assert_eq!(now, plan_text());
}

#[test]
fn the_checklist_reads_the_bullets_plan_import_accepted() {
    let tmp = tempfile::tempdir().unwrap();
    write(tmp.path());
    let out = bosun()
        .env_remove("GITHUB_TOKEN")
        .env("BOSUN_NO_GH", "1")
        .arg("-C")
        .arg(tmp.path())
        .args(["prereqs", "plan.md", "--no-setup"])
        .output()
        .unwrap();
    let v = common::one_object(&out.stdout);
    let text = v.to_string();
    assert!(!text.contains("not a valid"), "{v}");
    assert!(text.contains("H1") && text.contains("H3"), "{v}");
}
