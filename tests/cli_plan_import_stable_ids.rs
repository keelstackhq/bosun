//! Prerequisite ids that were inferred (a missing or duplicate number) are never imported: what
//! reaches GitHub is written in the file. Readers stay lenient and warn.
mod common;

use common::bosun;
use common::fake::FakeGithub;
use serde_json::Value;
use std::path::Path;

const BASE: &str = "# Plan: widget\n\nproject: widget\n\n## M1: Lifecycle\n\narea: lifecycle\n\nThe machine.\n\n\
### Tasks\n\n- T1: One\n  touch: src/a.rs\n\n### Done when\n\n- UC-M1-1: works.\n";
const BARE: &str =
    "\n## Prerequisites\n\n- H: token\n  on: phone\n  steps: add it\n  blocks: M1/T1\n";
const FORMAT_ONLY: &str =
    "\n## Prerequisites\n\n* H1 - token\n  on: phone\n  steps: add it\n  blocks: M1/T1\n";

fn write(dir: &Path, section: &str) {
    std::fs::write(dir.join("plan.md"), format!("{BASE}{section}")).unwrap();
}

fn import(fake: &FakeGithub, dir: &Path, extra: &[&str]) -> (i32, Value) {
    let mut a = vec!["-C", dir.to_str().unwrap(), "plan", "import", "plan.md"];
    a.extend(extra);
    common::run(fake, "session_test", "2026-10-27T10:00:00Z", &a)
}

#[test]
fn a_real_import_refuses_inferred_ids_and_names_the_fix() {
    let (tmp, fake) = (tempfile::tempdir().unwrap(), FakeGithub::start());
    write(tmp.path(), BARE);
    let (code, v) = import(&fake, tmp.path(), &[]);
    assert_eq!(code, 1, "{v}");
    assert_eq!(
        v["error"]["fix"],
        "run `bosun plan import plan.md --fix` (rewrites the ids into the file) and commit it"
    );
    // Nothing was written, nothing reached GitHub.
    let now = std::fs::read_to_string(tmp.path().join("plan.md")).unwrap();
    assert_eq!(now, format!("{BASE}{BARE}"));
    assert!(v["next"].as_array().is_none_or(|n| n.is_empty()));
}

#[test]
fn a_dry_run_still_reads_inferred_ids() {
    let (tmp, fake) = (tempfile::tempdir().unwrap(), FakeGithub::start());
    write(tmp.path(), BARE);
    let (code, v) = import(&fake, tmp.path(), &["--dry-run"]);
    assert_eq!(code, 0, "{v}");
}

#[test]
fn fix_writes_the_ids_and_the_import_goes_on() {
    let (tmp, fake) = (tempfile::tempdir().unwrap(), FakeGithub::start());
    write(tmp.path(), BARE);
    let (code, v) = import(&fake, tmp.path(), &["--fix"]);
    // The id refusal is behind us: the fake's main has no plan file, so a later check stops it.
    let msg = v["error"]["message"].as_str().unwrap_or_default();
    assert!(!msg.contains("inferred"), "{code} {v}");
    let now = std::fs::read_to_string(tmp.path().join("plan.md")).unwrap();
    assert!(now.contains("- H1: token\n"), "{now}");
}

#[test]
fn a_formatting_only_fix_keeps_its_id_and_imports() {
    let (tmp, fake) = (tempfile::tempdir().unwrap(), FakeGithub::start());
    write(tmp.path(), FORMAT_ONLY);
    let (_, v) = import(&fake, tmp.path(), &[]);
    let msg = v["error"]["message"].as_str().unwrap_or_default();
    assert!(!msg.contains("inferred"), "{v}");
}

#[test]
fn plan_check_warns_that_inferred_ids_are_not_stable() {
    let tmp = tempfile::tempdir().unwrap();
    write(tmp.path(), BARE);
    let out = bosun()
        .env_remove("GITHUB_TOKEN")
        .arg("-C")
        .arg(tmp.path())
        .args(["plan", "check", "plan.md"])
        .output()
        .unwrap();
    let v = common::one_object(&out.stdout);
    assert_eq!(out.status.code(), Some(0), "{v}");
    assert!(
        v["data"]["warnings"].to_string().contains(
            "prerequisite ids are inferred; run `bosun plan check <file> --fix` to make them stable"
        ),
        "{v}"
    );
}

#[test]
fn the_checklist_warns_and_still_reads() {
    let tmp = tempfile::tempdir().unwrap();
    write(tmp.path(), BARE);
    let out = bosun()
        .env_remove("GITHUB_TOKEN")
        .env("BOSUN_NO_GH", "1")
        .arg("-C")
        .arg(tmp.path())
        .args(["prereqs", "plan.md", "--no-setup"])
        .output()
        .unwrap();
    let v = common::one_object(&out.stdout);
    assert_eq!(out.status.code(), Some(0), "{v}");
    assert!(
        v["data"]["warnings"]
            .to_string()
            .contains("prerequisite ids are inferred"),
        "{v}"
    );
}
