//! `plan check` reports the PR shape of each item; import writes it into the epic.

mod common;

use common::fake::FakeGithub;
use serde_json::Value;
use std::path::Path;

const T0: &str = "2026-10-23T10:00:00Z";

const PLAN: &str = "# Plan: widget

project: widget

## M1: Big
area: core
shape: single

### Tasks

- T1: a
  touch: src/a.rs
  size: 500
- T2: b
  touch: src/b.rs
  size: 500

### Done when

- UC-M1-1: ok.
- UC-M1-2: ok.

## M2: Small
area: core

### Tasks

- T1: a
  touch: src/c.rs

- T2: b
  touch: src/d.rs
  risk: high
  size: 50

### Done when

- UC-M2-1: ok.
";

fn plan(fake: &FakeGithub, dir: &Path, args: &[&str]) -> (i32, Value) {
    let mut a = vec!["-C", dir.to_str().unwrap(), "plan"];
    a.extend_from_slice(args);
    common::run(fake, "session_test", T0, &a)
}

#[test]
fn check_reports_shape_reasons_and_size() {
    let (tmp, fake) = (tempfile::tempdir().unwrap(), FakeGithub::start());
    std::fs::write(tmp.path().join("plan.md"), PLAN).unwrap();
    let (code, v) = plan(&fake, tmp.path(), &["check", "plan.md"]);
    assert_eq!(code, 0, "{v}");
    let items = &v["data"]["items"];
    assert_eq!(items[0]["shape"], "single");
    assert_eq!(items[0]["size"], 1000);
    assert_eq!(items[0]["shape_reasons"][0], "planner chose single");
    assert_eq!(items[1]["shape"], "split");
    assert_eq!(items[1]["size"], 250);
    let why = items[1]["shape_reasons"][0].as_str().unwrap();
    assert!(why.starts_with("M2/T2 is risk: high"), "{why}");
}

#[test]
fn import_writes_the_shape_into_the_epic() {
    let (tmp, fake) = (tempfile::tempdir().unwrap(), FakeGithub::start());
    std::fs::write(tmp.path().join("plan.md"), PLAN).unwrap();
    let file = tmp.path().join("plan.md");
    let (code, v) = plan(&fake, tmp.path(), &["import", file.to_str().unwrap()]);
    assert_eq!(code, 0, "{v}");
    let epics = v["data"]["epics"].as_array().unwrap();
    let shape = |k: usize| {
        let n = epics[k]["issue"].as_u64().unwrap();
        fake.issue_state(n).body
    };
    assert!(shape(0).contains(r#""shape":"single""#), "{}", shape(0));
    assert!(shape(1).contains(r#""shape":"split""#), "{}", shape(1));
}
