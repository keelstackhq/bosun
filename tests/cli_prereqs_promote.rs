//! Front-loaded prerequisites (design F5(1)): `plan import` refreshes the checklist issue and
//! keeps tasks that an open owner item blocks `planned`, listed under `data.held`.

mod common;

use common::fake::FakeGithub;
use serde_json::{Value, json};
use std::path::Path;

const T0: &str = "2026-10-27T10:00:00Z";
const BASE: &str = "# Plan: widget\n\nproject: widget\n\n## M1: Lifecycle\n\narea: lifecycle\n\nWords.\n\n\
### Tasks\n\n- T1: One\n  touch: src/a.rs\n- T2: Two\n  touch: src/b.rs\n\n\
### Done when\n\n- UC-M1-1: works.\n";

/// The plan with one owner item H1 that blocks `blocks`.
fn with_h1(blocks: &str) -> String {
    format!(
        "{BASE}\n## Prerequisites\n\n- H1: token\n  on: phone\n  steps: add it\n  blocks: {blocks}\n"
    )
}

fn import(fake: &FakeGithub, dir: &Path, extra: &[&str]) -> (i32, Value) {
    let d = dir.to_str().unwrap();
    let mut a = vec!["-C", d, "plan", "import", "plan.md"];
    a.extend(extra);
    common::run(fake, "session_test", T0, &a)
}

fn setup(text: &str) -> (tempfile::TempDir, FakeGithub) {
    let (tmp, fake) = (tempfile::tempdir().unwrap(), FakeGithub::start());
    std::fs::write(tmp.path().join("plan.md"), text).unwrap();
    (tmp, fake)
}

fn number(fake: &FakeGithub, marker: &str) -> u64 {
    let s = fake.state.lock().unwrap();
    let found = s.issues.iter().find(|i| i.body.contains(marker));
    found
        .unwrap_or_else(|| panic!("no issue with {marker}"))
        .number
}

fn state(fake: &FakeGithub, id: &str) -> Vec<String> {
    let n = number(fake, &format!("<!-- bosun:plan {id} -->"));
    let labels = fake.issue_state(n).labels;
    labels
        .into_iter()
        .filter(|l| l.starts_with("bosun:state/"))
        .collect()
}

fn checklists(fake: &FakeGithub) -> usize {
    let s = fake.state.lock().unwrap();
    s.issues
        .iter()
        .filter(|i| i.labels.iter().any(|l| l == "bosun:prereqs"))
        .count()
}

/// Import once, then mark the plan task of M1 as merged.
fn merged(tmp: &Path, fake: &FakeGithub) -> Value {
    let (code, v) = import(fake, tmp, &[]);
    assert_eq!(code, 0, "{v}");
    fake.close_issue_as(number(fake, "<!-- bosun:plan M1/P -->"), T0, "completed");
    v
}

fn tick_h1(fake: &FakeGithub) {
    let n = number(fake, "<!-- bosun:prereqs ");
    let mut s = fake.state.lock().unwrap();
    let i = s.issues.iter_mut().find(|i| i.number == n).unwrap();
    i.body = i.body.replace("- [ ] **H1**", "- [x] **H1**");
}

#[test]
fn an_open_item_holds_its_task_and_a_tick_releases_it() {
    let (tmp, fake) = setup(&with_h1("M1/T2"));
    let first = merged(tmp.path(), &fake);
    assert_eq!(first["data"]["held"], json!([]), "plan task not merged yet");
    assert_eq!(checklists(&fake), 1, "the import made the checklist issue");

    let (code, v) = import(&fake, tmp.path(), &[]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["promoted"], json!(["M1", "M1/T1"]));
    assert_eq!(
        v["data"]["held"],
        json!([{"task": "M1/T2", "needs": ["H1"]}])
    );
    assert_eq!(state(&fake, "M1/T1"), ["bosun:state/ready"]);
    assert_eq!(state(&fake, "M1/T2"), ["bosun:state/planned"]);
    assert_eq!(checklists(&fake), 1, "refreshed, not duplicated");

    tick_h1(&fake);
    let (code, v) = import(&fake, tmp.path(), &[]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["promoted"], json!(["M1/T2"]));
    assert_eq!(v["data"]["held"], json!([]));
    assert_eq!(state(&fake, "M1/T2"), ["bosun:state/ready"]);
}

#[test]
fn an_item_blocking_the_whole_item_holds_the_epic_and_all_tasks() {
    let (tmp, fake) = setup(&with_h1("M1"));
    merged(tmp.path(), &fake);
    let (code, v) = import(&fake, tmp.path(), &[]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["promoted"], json!([]));
    let held = json!([
        {"task": "M1", "needs": ["H1"]},
        {"task": "M1/T1", "needs": ["H1"]},
        {"task": "M1/T2", "needs": ["H1"]},
    ]);
    assert_eq!(v["data"]["held"], held);
    for id in ["M1", "M1/T1", "M1/T2"] {
        assert_eq!(state(&fake, id), ["bosun:state/planned"], "{id}");
    }
}

#[test]
fn dry_run_reads_only_and_still_lists_held() {
    let (tmp, fake) = setup(&with_h1("M1/T2"));
    merged(tmp.path(), &fake);
    // Drop the issue the first import made: a dry run must not make it again.
    let n = number(&fake, "<!-- bosun:prereqs ");
    fake.state.lock().unwrap().issues.retain(|i| i.number != n);
    let before = fake.requests().iter().filter(|r| r.method != "GET").count();
    let (code, v) = import(&fake, tmp.path(), &["--dry-run"]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["would_promote"], json!(["M1", "M1/T1"]));
    assert_eq!(
        v["data"]["held"],
        json!([{"task": "M1/T2", "needs": ["H1"]}])
    );
    assert_eq!(checklists(&fake), 0);
    let after = fake.requests().iter().filter(|r| r.method != "GET").count();
    assert_eq!(after, before, "a dry run writes nothing");
}

#[test]
fn a_plan_without_the_section_is_unchanged() {
    let (tmp, fake) = setup(BASE);
    merged(tmp.path(), &fake);
    let (code, v) = import(&fake, tmp.path(), &[]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["promoted"], json!(["M1", "M1/T1", "M1/T2"]));
    assert_eq!(v["data"]["held"], json!([]));
    assert_eq!(checklists(&fake), 0);
}

const SECRET: &str = "environments/release/secrets/NPM_TOKEN";

/// The plan with H1 verified through the API (a release secret) that blocks M1/T2.
fn with_secret() -> String {
    with_h1("M1/T2").replace(
        "  steps: add it\n",
        "  steps: add it\n  verify: secret release/NPM_TOKEN\n",
    )
}

fn sum() -> String {
    bosun::core::prereqs::digest("token", "add it", "secret release/NPM_TOKEN", "owner")
}

fn writes(fake: &FakeGithub) -> usize {
    fake.requests().iter().filter(|r| r.method != "GET").count()
}

#[test]
fn a_malformed_section_fails_and_writes_nothing() {
    let (tmp, fake) = setup(&format!("{BASE}\n## Pre-requisites\n\n- H1: token\n"));
    let (code, v) = import(&fake, tmp.path(), &[]);
    assert_eq!(code, 1, "{v}");
    assert_eq!(writes(&fake), 0, "no issue is made or changed");
    assert_eq!(fake.state.lock().unwrap().issues.len(), 0);
}

#[test]
fn an_item_the_api_finds_does_not_hold() {
    let (tmp, fake) = setup(&with_secret());
    fake.present(SECRET);
    merged(tmp.path(), &fake);
    let (code, v) = import(&fake, tmp.path(), &[]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["held"], json!([]));
    assert_eq!(v["data"]["promoted"], json!(["M1", "M1/T1", "M1/T2"]));
}

#[test]
fn a_403_item_holds_until_its_box_with_the_right_sum_is_ticked() {
    let (tmp, fake) = setup(&with_secret());
    fake.fail_path("secrets/NPM_TOKEN", 403);
    merged(tmp.path(), &fake);
    let held = json!([{"task": "M1/T2", "needs": ["H1"]}]);
    // A box with a wrong sum does not count.
    tick_h1(&fake);
    let n = number(&fake, "<!-- bosun:prereqs ");
    let mut s = fake.state.lock().unwrap();
    let i = s.issues.iter_mut().find(|i| i.number == n).unwrap();
    assert!(i.body.contains(&sum()), "{}", i.body);
    i.body = i.body.replace(&sum(), "0000");
    drop(s);
    let (code, v) = import(&fake, tmp.path(), &[]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["held"], held);
    tick_h1(&fake);
    let (_, v) = import(&fake, tmp.path(), &[]);
    assert_eq!(v["data"]["held"], json!([]), "{v}");
    assert_eq!(v["data"]["promoted"], json!(["M1/T2"]));
}

#[test]
fn plan_amend_lists_held_too() {
    let (tmp, fake) = setup(&with_h1("M1/T2"));
    merged(tmp.path(), &fake);
    let d = tmp.path().to_str().unwrap();
    let (code, v) = common::run(
        &fake,
        "session_test",
        T0,
        &["-C", d, "plan", "amend", "plan.md"],
    );
    assert_eq!(code, 0, "{v}");
    assert_eq!(
        v["data"]["held"],
        json!([{"task": "M1/T2", "needs": ["H1"]}])
    );
}

#[test]
fn two_checklist_issues_need_a_human() {
    let (tmp, fake) = setup(&with_h1("M1/T2"));
    merged(tmp.path(), &fake);
    let body = fake.issue_state(number(&fake, "<!-- bosun:prereqs ")).body;
    fake.issue(900, "Owner checklist: again", &["bosun:prereqs"], &body);
    let (code, v) = import(&fake, tmp.path(), &[]);
    assert_eq!(code, 5, "{v}");
    assert!(
        v["error"]["message"]
            .as_str()
            .unwrap()
            .contains("two checklist issues")
    );
}
