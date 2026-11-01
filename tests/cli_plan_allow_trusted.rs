//! A task's `allow:` reaches its metadata only from the plan file on the default branch:
//! test-integrity trusts it, so it must be owner-approved (merged), never a local edit.
mod common;

use common::fake::FakeGithub;
use serde_json::{Value, json};
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

fn allow_of(fake: &FakeGithub) -> Value {
    let s = fake.state.lock().unwrap();
    let body = &s
        .issues
        .iter()
        .find(|i| i.body.starts_with("<!-- bosun:plan M1/T1 -->"))
        .expect("task issue")
        .body;
    let json = body.rsplit("<!-- bosun:task ").next().unwrap();
    let meta: Value = serde_json::from_str(json.split(" -->").next().unwrap()).unwrap();
    meta["allow"].clone()
}

fn warned(v: &Value, text: &str) -> bool {
    v["data"]["warnings"]
        .as_array()
        .unwrap()
        .iter()
        .any(|w| w.as_str().unwrap_or("").contains(text))
}

#[test]
fn a_local_allow_not_on_main_is_dropped_with_a_warning() {
    let (tmp, fake) = (tempfile::tempdir().unwrap(), FakeGithub::start());
    let local = plan_text("allow: delete-test");
    fake.content("plan.md", "main", &plan_text(""));
    let (code, v) = plan(&fake, tmp.path(), &local, "import");
    assert_eq!(code, 0, "{v}");
    assert_eq!(allow_of(&fake), json!([]));
    assert!(
        warned(&v, "M1/T1: `allow: delete-test` with this touch"),
        "{v}"
    );
}

#[test]
fn a_plan_missing_on_main_allows_nothing() {
    let (tmp, fake) = (tempfile::tempdir().unwrap(), FakeGithub::start());
    let (code, v) = plan(&fake, tmp.path(), &plan_text("allow: skip"), "import");
    assert_eq!(code, 0, "{v}");
    assert_eq!(allow_of(&fake), json!([]));
}

#[test]
fn a_different_allow_on_main_allows_nothing() {
    let (tmp, fake) = (tempfile::tempdir().unwrap(), FakeGithub::start());
    fake.content("plan.md", "main", &plan_text("allow: delete-test"));
    let local = plan_text("allow: delete-test, fewer-asserts");
    let (code, v) = plan(&fake, tmp.path(), &local, "import");
    assert_eq!(code, 0, "{v}");
    assert_eq!(allow_of(&fake), json!([]));
}

#[test]
fn the_same_allow_on_main_is_used() {
    let (tmp, fake) = (tempfile::tempdir().unwrap(), FakeGithub::start());
    let text = plan_text("allow: delete-test, fewer-asserts");
    // Main differs elsewhere (a task title): only the task's `allow` must match.
    fake.content(
        "plan.md",
        "main",
        &text.replace("Drop the seed crate", "Drop it"),
    );
    let (code, v) = plan(&fake, tmp.path(), &text, "import");
    assert_eq!(code, 0, "{v}");
    assert_eq!(allow_of(&fake), json!(["delete-test", "fewer-asserts"]));
    assert!(!warned(&v, "allow"), "{v}");
}

#[test]
fn an_api_error_allows_nothing_and_says_so() {
    let (tmp, fake) = (tempfile::tempdir().unwrap(), FakeGithub::start());
    fake.content("plan.md", "main", &plan_text("allow: delete-test"));
    fake.fail_path("/contents/plan.md", 500);
    let (code, v) = plan(
        &fake,
        tmp.path(),
        &plan_text("allow: delete-test"),
        "import",
    );
    assert_eq!(code, 0, "{v}");
    assert_eq!(allow_of(&fake), json!([]));
    assert!(
        warned(&v, "cannot read plan.md on the default branch"),
        "{v}"
    );
}

#[test]
fn amend_sets_the_allow_once_it_is_merged() {
    let (tmp, fake) = (tempfile::tempdir().unwrap(), FakeGithub::start());
    let text = plan_text("allow: delete-test");
    let (code, _) = plan(&fake, tmp.path(), &text, "import");
    assert_eq!(code, 0);
    assert_eq!(allow_of(&fake), json!([]));
    fake.content("plan.md", "main", &text);
    let (code, v) = plan(&fake, tmp.path(), &text, "amend");
    assert_eq!(code, 0, "{v}");
    assert_eq!(allow_of(&fake), json!(["delete-test"]));
}

#[test]
fn a_widened_local_touch_does_not_borrow_mains_allow_at_import() {
    let (tmp, fake) = (tempfile::tempdir().unwrap(), FakeGithub::start());
    fake.content("plan.md", "main", &plan_text("allow: delete-test"));
    let local = plan_text("allow: delete-test").replace("touch: crates/seed/**", "touch: **");
    let (code, v) = plan(&fake, tmp.path(), &local, "import");
    assert_eq!(code, 0, "{v}");
    assert_eq!(allow_of(&fake), json!([]));
    assert!(warned(&v, "with this touch"), "{v}");
}

#[test]
fn a_widened_local_touch_does_not_borrow_mains_allow_at_amend() {
    let (tmp, fake) = (tempfile::tempdir().unwrap(), FakeGithub::start());
    let text = plan_text("allow: delete-test");
    fake.content("plan.md", "main", &text);
    let (code, _) = plan(&fake, tmp.path(), &text, "import");
    assert_eq!(code, 0);
    assert_eq!(allow_of(&fake), json!(["delete-test"]));
    // The local plan widens the touch; main still has the narrow one.
    let wide = text.replace("touch: crates/seed/**", "touch: **");
    let (code, v) = plan(&fake, tmp.path(), &wide, "amend");
    assert_eq!(code, 0, "{v}");
    assert_eq!(
        allow_of(&fake),
        json!([]),
        "the grant is dropped, not widened"
    );
}

#[test]
fn a_different_kind_or_risk_does_not_borrow_mains_allow() {
    let (tmp, fake) = (tempfile::tempdir().unwrap(), FakeGithub::start());
    fake.content("plan.md", "main", &plan_text("allow: delete-test"));
    let local = plan_text("allow: delete-test\n  risk: high");
    let (code, v) = plan(&fake, tmp.path(), &local, "import");
    assert_eq!(code, 0, "{v}");
    assert_eq!(allow_of(&fake), json!([]));
}
