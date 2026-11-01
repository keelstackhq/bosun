//! A plan task's `allow:` reaches the task metadata, so CI can read it (plan format, spec §7).
mod common;

use common::fake::FakeGithub;
use serde_json::Value;
use std::path::Path;

const T0: &str = "2026-10-23T10:00:00Z";

fn plan_text(allow: &str) -> String {
    format!(
        "# Plan: widget

project: widget

## M1: Seed cleanup

area: seed

### Tasks

- T1: Drop the seed crate
  touch: crates/seed/**
  {allow}

### Done when

- UC-M1-1: the seed crate is gone.
"
    )
}

fn plan(fake: &FakeGithub, dir: &Path, text: &str, cmd: &str) -> (i32, Value) {
    let file = dir.join("plan.md");
    std::fs::write(&file, text).unwrap();
    let args = [
        "-C",
        dir.to_str().unwrap(),
        "plan",
        cmd,
        file.to_str().unwrap(),
    ];
    common::run(fake, "session_test", T0, &args)
}

fn meta_of(fake: &FakeGithub, marker: &str) -> Value {
    let s = fake.state.lock().unwrap();
    let body = &s
        .issues
        .iter()
        .find(|i| i.body.starts_with(marker))
        .unwrap_or_else(|| panic!("no issue with {marker}"))
        .body;
    let json = body.rsplit("<!-- bosun:task ").next().unwrap();
    serde_json::from_str(json.split(" -->").next().unwrap()).unwrap()
}

#[test]
fn import_stores_the_allowed_kinds_in_the_task_metadata() {
    let (tmp, fake) = (tempfile::tempdir().unwrap(), FakeGithub::start());
    let text = plan_text("allow: delete-test, fewer-asserts");
    // `allow:` counts only as merged: the same plan is on the default branch.
    fake.content("plan.md", "main", &text);
    let (code, v) = plan(&fake, tmp.path(), &text, "import");
    assert_eq!(code, 0, "{v}");
    let m = meta_of(&fake, "<!-- bosun:plan M1/T1 -->");
    assert_eq!(
        m["allow"],
        serde_json::json!(["delete-test", "fewer-asserts"])
    );
}

#[test]
fn without_allow_the_task_allows_nothing() {
    let (tmp, fake) = (tempfile::tempdir().unwrap(), FakeGithub::start());
    let (code, v) = plan(&fake, tmp.path(), &plan_text(""), "import");
    assert_eq!(code, 0, "{v}");
    let m = meta_of(&fake, "<!-- bosun:plan M1/T1 -->");
    assert_eq!(m["allow"], serde_json::json!([]));
}

#[test]
fn an_unknown_kind_is_a_plan_error() {
    let (tmp, fake) = (tempfile::tempdir().unwrap(), FakeGithub::start());
    let text = plan_text("allow: delete-test, everything");
    let (code, v) = plan(&fake, tmp.path(), &text, "check");
    assert_eq!(code, 1, "{v}");
    let all = v.to_string();
    assert!(all.contains("everything"), "{all}");
    assert!(
        all.contains("delete-test, skip, only, fewer-asserts"),
        "{all}"
    );
}
