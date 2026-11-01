//! `plan import` closes the plan tasks of a plan file that is already byte-identical on the
//! default branch (F5): nothing is left to commit. `--keep-plan-tasks-open` opts out.

mod common;

use common::fake::FakeGithub;
use serde_json::{Value, json};
use std::path::Path;

const T0: &str = "2026-10-23T10:00:00Z";

const PLAN: &str = "# Plan: widget

project: widget

## M1: Lifecycle

area: lifecycle

### Tasks

- T1: State machine
  touch: src/core/state.rs

### Done when

- UC-M1-1: a task moves planned -> merged.

## M2: Intake

area: intake

### Tasks

- T1: Import
  touch: src/cli/plan.rs

### Done when

- UC-M2-1: importing twice creates no duplicates.
";

fn plan(fake: &FakeGithub, dir: &Path, args: &[&str]) -> (i32, Value) {
    let mut a = vec!["-C", dir.to_str().unwrap(), "plan"];
    a.extend_from_slice(args);
    common::run(fake, "session_test", T0, &a)
}

fn issue_of(fake: &FakeGithub, marker: &str) -> (Vec<String>, String, Option<String>) {
    let s = fake.state.lock().unwrap();
    let i = s
        .issues
        .iter()
        .find(|i| i.body.contains(marker))
        .unwrap_or_else(|| panic!("no issue with {marker}"));
    (i.labels.clone(), i.state.clone(), i.state_reason.clone())
}

fn state(fake: &FakeGithub, marker: &str) -> Vec<String> {
    issue_of(fake, marker)
        .0
        .into_iter()
        .filter(|l| l.starts_with("bosun:state/"))
        .collect()
}

fn setup(text: &str) -> (tempfile::TempDir, FakeGithub, String) {
    let (tmp, fake) = (tempfile::tempdir().unwrap(), FakeGithub::start());
    std::fs::write(tmp.path().join("plan.md"), text).unwrap();
    let file = tmp.path().join("plan.md").to_str().unwrap().to_string();
    (tmp, fake, file)
}

#[test]
fn a_plan_already_on_main_closes_its_plan_tasks_on_the_first_import() {
    let (tmp, fake, file) = setup(PLAN);
    fake.content("plan.md", "main", PLAN);
    let (code, v) = plan(&fake, tmp.path(), &["import", &file]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["on_default"], json!(["M1/P", "M2/P"]));
    for p in ["M1/P", "M2/P"] {
        let (labels, st, reason) = issue_of(&fake, &format!("<!-- bosun:plan {p} -->"));
        assert!(labels.contains(&"bosun:state/merged".to_string()), "{p}");
        assert_eq!(
            (st.as_str(), reason.as_deref()),
            ("closed", Some("completed"))
        );
    }
    assert_eq!(v["data"]["promoted"], json!(["M1", "M1/T1", "M2", "M2/T1"]));
}

#[test]
fn a_file_that_differs_on_main_closes_nothing_and_ready_ones_close_later() {
    let (tmp, fake, file) = setup(PLAN);
    // One task title differs on main: the files are not the same blob.
    fake.content(
        "plan.md",
        "main",
        &PLAN.replace("- T1: Import", "- T1: Import it"),
    );
    let (code, v) = plan(&fake, tmp.path(), &["import", &file]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["on_default"], json!([]));
    for p in ["M1/P", "M2/P"] {
        let marker = format!("<!-- bosun:plan {p} -->");
        assert_eq!(state(&fake, &marker), ["bosun:state/ready"], "{p}");
    }
    assert_eq!(v["data"]["promoted"], json!([]));

    // A trailing newline is a different file too.
    fake.content("plan.md", "main", &format!("{PLAN}\n"));
    let (_, v) = plan(&fake, tmp.path(), &["import", &file]);
    assert_eq!(v["data"]["on_default"], json!([]));

    // Main catches up: the existing `ready` plan tasks are closed.
    fake.content("plan.md", "main", PLAN);
    let (code, v) = plan(&fake, tmp.path(), &["import", &file]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["on_default"], json!(["M1/P", "M2/P"]));
    assert_eq!(
        state(&fake, "<!-- bosun:plan M2/P -->"),
        ["bosun:state/merged"]
    );
    assert_eq!(issue_of(&fake, "<!-- bosun:plan M2/P -->").1, "closed");
    assert_eq!(v["data"]["promoted"], json!(["M1", "M1/T1", "M2", "M2/T1"]));
}

#[test]
fn a_dry_run_only_reports_it() {
    let (tmp, fake, file) = setup(PLAN);
    fake.content("plan.md", "main", PLAN);
    let (code, v) = plan(&fake, tmp.path(), &["import", &file, "--dry-run"]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["on_default"], json!(["M1/P", "M2/P"]));
    assert!(fake.requests().iter().all(|r| r.method == "GET"));
}

#[test]
fn keep_plan_tasks_open_only_lists_them() {
    let (tmp, fake, file) = setup(PLAN);
    fake.content("plan.md", "main", PLAN);
    let (code, v) = plan(
        &fake,
        tmp.path(),
        &["import", &file, "--keep-plan-tasks-open"],
    );
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["on_default"], json!([]));
    assert_eq!(v["data"]["on_default_open"], json!(["M1/P", "M2/P"]));
    assert_eq!(
        state(&fake, "<!-- bosun:plan M1/P -->"),
        ["bosun:state/ready"]
    );
    assert_eq!(v["data"]["promoted"], json!([]));
}

#[test]
fn plan_amend_never_closes_them() {
    let (tmp, fake, file) = setup(PLAN);
    fake.content("plan.md", "main", PLAN);
    let (code, v) = plan(&fake, tmp.path(), &["amend", &file]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["on_default"], json!([]));
    assert_eq!(
        state(&fake, "<!-- bosun:plan M1/P -->"),
        ["bosun:state/ready"]
    );
}

#[test]
fn a_plan_task_someone_works_on_is_left_alone() {
    let (tmp, fake, file) = setup(PLAN);
    plan(&fake, tmp.path(), &["import", &file]);
    let n = {
        let mut s = fake.state.lock().unwrap();
        let i = s
            .issues
            .iter_mut()
            .find(|i| i.body.contains("<!-- bosun:plan M1/P -->"))
            .unwrap();
        i.labels = vec!["bosun:state/review".into()];
        i.number
    };
    fake.content("plan.md", "main", PLAN);
    let (code, v) = plan(&fake, tmp.path(), &["import", &file]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["on_default"], json!(["M2/P"]));
    assert_eq!(fake.issue_state(n).labels, ["bosun:state/review"]);
}

#[test]
fn a_plan_long_only_by_merged_tasks_still_closes_on_default() {
    // M1 has five chained tasks, T1 merged: four open. The copy on main must still compare.
    let long = PLAN.replace(
        "- T1: State machine\n  touch: src/core/state.rs\n",
        "- T1: State machine\n  touch: src/core/state.rs\n- T2: b\n  touch: src/b.rs\n  depends: T1\n- T3: c\n  touch: src/c.rs\n  depends: T2\n- T4: d\n  touch: src/d.rs\n  depends: T3\n- T5: e\n  touch: src/e.rs\n  depends: T4\n",
    );
    let (tmp, fake, file) = setup(&long);
    fake.issue(
        40,
        "M1/T1",
        &["bosun:state/merged"],
        "<!-- bosun:plan M1/T1 -->\n",
    );
    fake.content("plan.md", "main", &long);
    let (code, v) = plan(&fake, tmp.path(), &["import", &file]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["on_default"], json!(["M1/P", "M2/P"]));
}
