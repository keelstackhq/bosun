//! Rule 6 (critical path at most 4 tasks) counts only open tasks (F15).

mod common;

use common::fake::FakeGithub;
use serde_json::Value;
use std::path::Path;

const T0: &str = "2026-10-23T10:00:00Z";

fn chain(n: usize) -> String {
    let tasks: String = (1..=n)
        .map(|k| {
            let dep = if k > 1 {
                format!("  depends: T{}\n", k - 1)
            } else {
                String::new()
            };
            format!("- T{k}: t\n  touch: src/a{k}.rs\n{dep}")
        })
        .collect();
    format!(
        "# Plan: x\n\nproject: widget\n\n## M1: One\n\narea: a\n\n### Tasks\n\n{tasks}\n### Done when\n\n- UC-M1-1: ok\n"
    )
}

fn check(fake: &FakeGithub, dir: &Path, text: &str) -> (i32, Value) {
    std::fs::write(dir.join("plan.md"), text).unwrap();
    let file = dir.join("plan.md");
    common::run(
        fake,
        "session_test",
        T0,
        &[
            "-C",
            dir.to_str().unwrap(),
            "plan",
            "check",
            file.to_str().unwrap(),
        ],
    )
}

#[test]
fn merged_tasks_do_not_count_toward_the_critical_path() {
    let (tmp, fake) = (tempfile::tempdir().unwrap(), FakeGithub::start());
    let (code, v) = check(&fake, tmp.path(), &chain(5));
    assert_eq!(code, 1, "five open tasks are too many: {v}");

    fake.issue(
        40,
        "M1/T1: t",
        &["bosun:state/merged"],
        "<!-- bosun:plan M1/T1 -->\n",
    );
    let (code, v) = check(&fake, tmp.path(), &chain(5));
    assert_eq!(code, 0, "one of five is merged: {v}");

    let (code, v) = check(&fake, tmp.path(), &chain(6));
    assert_eq!(code, 1, "five open of six: {v}");
    assert!(
        v["error"]["fix"]
            .as_str()
            .unwrap()
            .contains("critical path of M1 is 5")
    );
}

#[test]
fn a_closed_not_planned_task_still_counts() {
    let (tmp, fake) = (tempfile::tempdir().unwrap(), FakeGithub::start());
    fake.issue(40, "M1/T1: t", &[], "<!-- bosun:plan M1/T1 -->\n");
    fake.close_issue_as(40, T0, "not_planned");
    let (code, v) = check(&fake, tmp.path(), &chain(5));
    assert_eq!(code, 1, "{v}");
}
