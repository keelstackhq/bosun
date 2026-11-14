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
    assert_eq!(items[1]["shape"], "single");
    assert_eq!(items[1]["size"], 250);
    assert_eq!(items[1]["shape_reasons"][0], "1 gated tasks run alone");
    assert_eq!(items[1]["gated"][0], "M2/T2");
    assert_eq!(items[0]["gated"].as_array().unwrap().len(), 0);
}

const GATED: &str = "# Plan: widget

project: widget

## M3: Mixed
area: core

### Tasks

- T1: ci
  touch: .github/workflows/ci.yml
  size: 30
- T2: a
  touch: src/a.rs, tests/a.rs
  size: 5000
- T3: b
  touch: src/b.rs, tests/b.rs
  size: 5000

### Done when

- UC-M3-1: ok.
- UC-M3-2: ok.
";

#[test]
fn a_task_touching_a_risk_path_is_gated_and_size_caps_nothing() {
    let (tmp, fake) = (tempfile::tempdir().unwrap(), FakeGithub::start());
    std::fs::write(tmp.path().join("plan.md"), GATED).unwrap();
    let (code, v) = plan(&fake, tmp.path(), &["check", "plan.md"]);
    assert_eq!(code, 0, "{v}");
    let m = &v["data"]["items"][0];
    assert_eq!(m["gated"][0], "M3/T1");
    assert_eq!(m["shape"], "split");
    assert_eq!(v["data"]["warnings"].as_array().unwrap().len(), 0, "{v}");
    // A custom rule set in the working tree decides what is gated.
    std::fs::create_dir_all(tmp.path().join(".bosun")).unwrap();
    std::fs::write(
        tmp.path().join(".bosun/risk.toml"),
        "[[rule]]\nid = \"src-a\"\nwhy = \"x\"\npaths = [\"src/a.rs\"]\n",
    )
    .unwrap();
    // src/a.rs (size 5000) is now gated: D-016 caps it at 400.
    let (code, v) = plan(&fake, tmp.path(), &["check", "plan.md"]);
    assert_eq!(code, 1, "{v}");
    assert_eq!(v["data"]["errors"][0]["line"], 15, "{v}");
    assert!(
        v["data"]["errors"][0]["message"]
            .as_str()
            .unwrap()
            .contains("size 5000")
    );
    std::fs::write(tmp.path().join(".bosun/risk.toml"), "not toml [").unwrap();
    let (code, v) = plan(&fake, tmp.path(), &["check", "plan.md"]);
    assert_eq!(code, 1, "{v}");
    assert!(!v["error"]["fix"].as_str().unwrap().is_empty());
}

#[test]
fn import_records_gated_and_size_in_the_task_meta() {
    let (tmp, fake) = (tempfile::tempdir().unwrap(), FakeGithub::start());
    std::fs::write(tmp.path().join("plan.md"), GATED).unwrap();
    let file = tmp.path().join("plan.md");
    let (code, v) = plan(
        &fake,
        tmp.path(),
        &["import", file.to_str().unwrap(), "--full"],
    );
    assert_eq!(code, 0, "{v}");
    let bodies: Vec<String> = fake
        .state
        .lock()
        .unwrap()
        .issues
        .iter()
        .map(|i| i.body.clone())
        .collect();
    let task = |id: &str| {
        bodies
            .iter()
            .find(|b| b.contains(&format!(r#""task":"{id}""#)))
            .unwrap_or_else(|| panic!("no body for {id}: {bodies:?}"))
            .clone()
    };
    assert!(task("T1").contains(r#""gated":true"#) && task("T1").contains(r#""size":30"#));
    assert!(task("T2").contains(r#""size":5000"#) && !task("T2").contains(r#""gated":true"#));
}

#[test]
fn import_writes_the_shape_into_the_epic() {
    let (tmp, fake) = (tempfile::tempdir().unwrap(), FakeGithub::start());
    std::fs::write(tmp.path().join("plan.md"), PLAN).unwrap();
    let file = tmp.path().join("plan.md");
    let (code, v) = plan(
        &fake,
        tmp.path(),
        &["import", file.to_str().unwrap(), "--full"],
    );
    assert_eq!(code, 0, "{v}");
    let epics = v["data"]["epics"].as_array().unwrap();
    let shape = |k: usize| {
        let n = epics[k]["issue"].as_u64().unwrap();
        fake.issue_state(n).body
    };
    assert!(shape(0).contains(r#""shape":"single""#), "{}", shape(0));
    assert!(shape(1).contains(r#""shape":"single""#), "{}", shape(1));
}

const BIG_GATED: &str = "# Plan: widget

project: widget

## M4: Big gate
area: core

### Tasks

- T1: ci
  touch: .github/workflows/ci.yml
  size: 401

### Done when

- UC-M4-1: ok.
";

fn assert_gated_size_error(v: &Value) {
    let e = &v["data"]["errors"][0];
    assert!(
        e["message"]
            .as_str()
            .unwrap()
            .contains("size 401 is over 400 changed lines for a gated task"),
        "{v}"
    );
    assert!(e["fix"].as_str().unwrap().contains("D-016"), "{v}");
}

#[test]
fn a_gated_task_over_400_lines_is_a_plan_error() {
    let (tmp, fake) = (tempfile::tempdir().unwrap(), FakeGithub::start());
    std::fs::write(tmp.path().join("plan.md"), BIG_GATED).unwrap();
    for cmd in ["check", "import", "amend"] {
        let (code, v) = plan(&fake, tmp.path(), &[cmd, "plan.md"]);
        assert_eq!(code, 1, "{cmd}: {v}");
        assert_gated_size_error(&v);
    }
}
