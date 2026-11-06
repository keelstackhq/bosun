//! A wildcard touch is resolved against the default branch's tree: a subtree holding a gated
//! file (`packages/hoist.toml`) makes the task gated; an unreadable tree is a plan error.

mod common;

use common::fake::FakeGithub;
use serde_json::Value;

const T0: &str = "2026-10-23T10:00:00Z";

const PLAN: &str = "# Plan: widget

project: widget

## M1: Pkg
area: core

### Tasks

- T1: pkg
  touch: packages/**
  size: 450

### Done when

- UC-M1-1: ok.
";

fn plan(fake: &FakeGithub, dir: &std::path::Path, cmd: &str) -> (i32, Value) {
    common::run(
        fake,
        "session_test",
        T0,
        &["-C", dir.to_str().unwrap(), "plan", cmd, "plan.md"],
    )
}

fn world(entries: &[(&str, &str, &str, &str)], truncated: bool) -> (tempfile::TempDir, FakeGithub) {
    let (tmp, fake) = (tempfile::tempdir().unwrap(), FakeGithub::start());
    std::fs::write(tmp.path().join("plan.md"), PLAN).unwrap();
    fake.git_tree("main", entries, truncated);
    (tmp, fake)
}

#[test]
fn a_subtree_holding_a_gated_file_makes_a_big_task_a_size_error() {
    let (tmp, fake) = world(&[("packages/a/hoist.toml", "100644", "blob", "s1")], false);
    for cmd in ["import", "amend"] {
        let (code, v) = plan(&fake, tmp.path(), cmd);
        assert_eq!(code, 1, "{cmd}: {v}");
        let m = v["data"]["errors"][0]["message"].as_str().unwrap();
        assert!(m.contains("over 400 changed lines for a gated task"), "{v}");
    }
}

#[test]
fn a_clean_subtree_is_not_gated() {
    let (tmp, fake) = world(&[("packages/a/lib.rs", "100644", "blob", "s1")], false);
    let (code, v) = plan(&fake, tmp.path(), "import");
    assert_eq!(code, 0, "{v}");
}

#[test]
fn an_unreadable_or_truncated_tree_is_a_plan_error_at_the_task() {
    let (tmp, fake) = world(&[("packages/a/lib.rs", "100644", "blob", "s1")], true);
    let (code, v) = plan(&fake, tmp.path(), "import");
    assert_eq!(code, 1, "{v}");
    let e = &v["data"]["errors"][0];
    assert!(
        e["message"].as_str().unwrap().contains("cannot be checked"),
        "{v}"
    );
    assert!(e["fix"].as_str().unwrap().contains("risk: high"), "{v}");
    let (tmp, fake) = (tempfile::tempdir().unwrap(), FakeGithub::start());
    fake.no_main_tree();
    std::fs::write(tmp.path().join("plan.md"), PLAN).unwrap();
    let (code, v) = plan(&fake, tmp.path(), "import");
    assert_eq!(code, 1, "{v}");
}
