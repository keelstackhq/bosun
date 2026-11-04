//! Plan task field `done-by: #N`: import and amend make the task merged when PR N is merged.

mod common;

use common::fake::FakeGithub;
use serde_json::Value;
use std::path::Path;

const T0: &str = "2026-10-23T10:00:00Z";

const PLAN: &str = "# Plan: widget

project: widget

## M1: Lifecycle

area: lifecycle

### Tasks

- T1: State machine
  touch: src/core/state.rs
- T2: Stop rules
  touch: src/core/stop.rs

### Done when

- UC-M1-1: a task moves planned -> merged.
";

const T2: &str = "- T2: Stop rules\n  touch: src/core/stop.rs\n";

fn with_done_by(by: &str) -> String {
    PLAN.replace(T2, &format!("{T2}  done-by: {by}\n"))
}

fn plan(fake: &FakeGithub, dir: &Path, text: &str, args: &[&str]) -> (i32, Value) {
    let p = dir.join("plan.md");
    std::fs::write(&p, text).unwrap();
    let file = p.to_str().unwrap().to_string();
    let mut a = vec!["-C", dir.to_str().unwrap(), "plan", args[0], &file];
    a.extend_from_slice(&args[1..]);
    common::run(fake, "session_test", T0, &a)
}

fn find(fake: &FakeGithub, marker: &str) -> u64 {
    let s = fake.state.lock().unwrap();
    s.issues
        .iter()
        .find(|i| i.body.starts_with(marker))
        .unwrap_or_else(|| panic!("no issue with {marker}"))
        .number
}

fn merged_pr(fake: &FakeGithub, n: u64) {
    fake.pull(n, "feat: done", "claude/done", "");
    fake.pull_merged(n, "abc");
}

fn is_merged(fake: &FakeGithub, n: u64) -> bool {
    let i = fake.issue_state(n);
    fake.labels_of(n)
        .contains(&"bosun:state/merged".to_string())
        && i.state == "closed"
        && i.state_reason.as_deref() == Some("completed")
}

#[test]
fn import_makes_a_done_by_task_merged_when_the_pr_merged() {
    let (tmp, fake) = (tempfile::tempdir().unwrap(), FakeGithub::start());
    merged_pr(&fake, 5);
    let (code, v) = plan(&fake, tmp.path(), &with_done_by("#5"), &["import"]);
    assert_eq!(code, 0, "{v}");
    let t2 = find(&fake, "<!-- bosun:plan M1/T2 -->");
    assert!(is_merged(&fake, t2));
    assert!(
        fake.comments_of(t2)
            .iter()
            .any(|c| c.body.contains("Done by PR #5"))
    );
    let t1 = find(&fake, "<!-- bosun:plan M1/T1 -->");
    assert_eq!(fake.issue_state(t1).state, "open");
    assert!(
        !fake
            .labels_of(t1)
            .contains(&"bosun:state/merged".to_string())
    );
}

#[test]
fn an_unmerged_done_by_pr_leaves_the_task_open() {
    let (tmp, fake) = (tempfile::tempdir().unwrap(), FakeGithub::start());
    fake.pull(5, "feat: wip", "claude/wip", "");
    let (code, v) = plan(&fake, tmp.path(), &with_done_by("#5"), &["import"]);
    assert_eq!(code, 0, "{v}");
    let t2 = find(&fake, "<!-- bosun:plan M1/T2 -->");
    assert_eq!(fake.issue_state(t2).state, "open");
    assert!(
        !fake
            .labels_of(t2)
            .contains(&"bosun:state/merged".to_string())
    );
}

#[test]
fn amend_merges_an_existing_open_task_that_gains_done_by() {
    let (tmp, fake) = (tempfile::tempdir().unwrap(), FakeGithub::start());
    let (code, v) = plan(&fake, tmp.path(), PLAN, &["import"]);
    assert_eq!(code, 0, "{v}");
    let t2 = find(&fake, "<!-- bosun:plan M1/T2 -->");
    assert_eq!(fake.issue_state(t2).state, "open");
    merged_pr(&fake, 5);
    let (code, v) = plan(&fake, tmp.path(), &with_done_by("5"), &["amend"]);
    assert_eq!(code, 0, "{v}");
    assert!(is_merged(&fake, t2));
}

#[test]
fn a_bad_done_by_value_is_a_plan_error() {
    let (tmp, fake) = (tempfile::tempdir().unwrap(), FakeGithub::start());
    let (code, v) = plan(&fake, tmp.path(), &with_done_by("soon"), &["check"]);
    assert_eq!(code, 1, "{v}");
    assert!(v.to_string().contains("done-by"), "{v}");
}
