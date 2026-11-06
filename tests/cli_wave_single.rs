mod common;

use common::fake::FakeGithub;
use common::{lease_note, run};
use serde_json::Value;

const NOW: &str = "2026-10-22T10:00:00Z";
const LATER: &str = "2026-10-22T10:20:00Z";

fn task(fake: &FakeGithub, n: u64, id: &str, state: &str, depends: &str, touch: &str) {
    let (item, t) = id.split_once('/').unwrap();
    let body = format!(
        r#"<!-- bosun:task {{"item":"{item}","task":"{t}","epic":10,"depends":[{depends}],"touch":[{touch}]}} -->"#
    );
    fake.issue(
        n,
        &format!("Task {id}"),
        &[&format!("bosun:state/{state}")],
        &body,
    );
}

fn epic(meta: &str) -> FakeGithub {
    let fake = FakeGithub::start();
    let body = format!("Epic\n<!-- bosun:epic {meta} -->");
    fake.issue(10, "Epic", &["bosun:state/claimed"], &body);
    fake
}

/// A narrow rule set on the default branch: the wildcard touches of these tests (`src/a/**`)
/// would overlap the built-in `**/...` risk globs and count as gated.
fn single() -> FakeGithub {
    let fake = epic(r#"{"item":"M1","shape":"single"}"#);
    let rule = "[[rule]]\nid = \"ci\"\nwhy = \"x\"\npaths = [\".github/workflows/**\"]\n";
    fake.content(".bosun/risk.toml", "main", rule);
    fake
}

fn plan(fake: &FakeGithub) -> Value {
    let (code, v) = run(fake, "session_test", NOW, &["wave", "plan", "10"]);
    assert_eq!(code, 0, "{v}");
    v
}

fn three(fake: &FakeGithub) {
    task(fake, 11, "M1/T1", "ready", "", r#""src/a/**""#);
    task(fake, 12, "M1/T2", "ready", r#""M1/T1""#, r#""src/b/**""#);
    task(
        fake,
        13,
        "M1/T3",
        "ready",
        r#""M1/T2""#,
        r#""src/a/**","docs/**""#,
    );
}

fn nums(v: &Value) -> Vec<u64> {
    v.as_array()
        .unwrap()
        .iter()
        .map(|n| n.as_u64().unwrap())
        .collect()
}

#[test]
fn ready_tasks_are_one_pr_led_by_the_lowest_issue() {
    let fake = single();
    three(&fake);
    let d = plan(&fake)["data"].clone();
    assert_eq!(d["shape"], "single");
    let run = d["run"].as_array().unwrap();
    assert_eq!(run.len(), 1);
    assert_eq!(run[0]["issue"], 11);
    assert_eq!(nums(&run[0]["bundle"]), vec![12, 13]);
    assert_eq!(nums(&run[0]["closes"]), vec![11, 12, 13]);
    assert_eq!(
        run[0]["touch"],
        serde_json::json!(["src/a/**", "src/b/**", "docs/**"])
    );
    assert!(d["waiting"].as_array().unwrap().is_empty());
}

#[test]
fn a_task_in_flight_holds_the_rest() {
    let fake = single();
    task(&fake, 11, "M1/T1", "building", "", r#""src/a/**""#);
    fake.comment(11, &lease_note("session_test", LATER, "src/a/**"));
    task(&fake, 12, "M1/T2", "ready", "", r#""src/b/**""#);
    let d = plan(&fake)["data"].clone();
    assert!(d["run"].as_array().unwrap().is_empty());
    let w = d["waiting"].as_array().unwrap();
    assert!(
        w.iter()
            .any(|x| x["issue"] == 12 && x["reason"] == "bundled into #11's PR")
    );
}

#[test]
fn an_open_dependency_outside_the_item_holds_every_task() {
    let fake = single();
    task(&fake, 11, "M1/T1", "ready", r#""X1/T1""#, r#""src/a/**""#);
    task(&fake, 12, "M1/T2", "ready", "", r#""src/b/**""#);
    fake.issue(
        20,
        "Other",
        &["bosun:state/building"],
        &task_meta("X1", "T1"),
    );
    let d = plan(&fake)["data"].clone();
    assert!(d["run"].as_array().unwrap().is_empty());
    let w = d["waiting"].as_array().unwrap();
    assert_eq!(w.len(), 2);
    assert!(w.iter().all(|x| x["reason"] == "waits for X1/T1"), "{w:?}");
}

fn task_meta(item: &str, t: &str) -> String {
    format!(
        r#"<!-- bosun:task {{"item":"{item}","task":"{t}","epic":99,"depends":[],"touch":["z"]}} -->"#
    )
}

#[test]
fn an_epic_without_a_shape_plans_as_before() {
    let fake = epic(r#"{"item":"M1"}"#);
    three(&fake);
    task(&fake, 14, "M1/T4", "ready", "", r#""zz/**""#);
    let d = plan(&fake)["data"].clone();
    assert_eq!(d["shape"], "split");
    let run = d["run"].as_array().unwrap();
    assert_eq!(
        run.iter()
            .map(|r| r["issue"].as_u64().unwrap())
            .collect::<Vec<_>>(),
        vec![11, 14]
    );
    assert!(run[0].get("bundle").is_none());
}

#[test]
fn a_gated_task_runs_alone_beside_the_bundle() {
    // risk: high (old metadata) and gated: true both keep the task out of the bundle.
    let fake = single();
    three(&fake);
    gated_task(&fake, 14, "M1/T4", r#""zz/**""#, r#","risk":"high""#);
    gated_task(&fake, 15, "M1/T5", r#""yy/**""#, r#","gated":true"#);
    let d = plan(&fake)["data"].clone();
    assert_eq!(d["shape"], "single");
    let run = d["run"].as_array().unwrap();
    assert_eq!(run.len(), 3, "{d}");
    assert_eq!(run[0]["issue"], 11);
    assert_eq!(nums(&run[0]["closes"]), [11, 12, 13]);
    for (k, n) in [(1, 14), (2, 15)] {
        assert_eq!(run[k]["issue"], n);
        assert!(run[k].get("bundle").is_none(), "{d}");
    }
}

#[test]
fn a_gated_task_overlapping_the_bundle_waits() {
    let fake = single();
    three(&fake);
    gated_task(&fake, 14, "M1/T4", r#""src/b/x.rs""#, r#","gated":true"#);
    let d = plan(&fake)["data"].clone();
    assert_eq!(d["run"].as_array().unwrap().len(), 1, "{d}");
    assert_eq!(d["waiting"][0]["reason"], "overlaps the bundle of #11");
}

fn gated_task(fake: &FakeGithub, n: u64, id: &str, touch: &str, extra: &str) {
    let (item, t) = id.split_once('/').unwrap();
    let body = format!(
        r#"<!-- bosun:task {{"item":"{item}","task":"{t}","epic":10,"depends":[],"touch":[{touch}]{extra}}} -->"#
    );
    fake.issue(n, &format!("Task {id}"), &["bosun:state/ready"], &body);
}

#[test]
fn wave_recomputes_gated_from_the_default_branch_rules() {
    // The task meta says nothing about gated, but the touch is a built-in risk path.
    let fake = epic(r#"{"item":"M1","shape":"single"}"#);
    task(&fake, 11, "M1/T1", "ready", "", r#""src/a.rs""#);
    task(
        &fake,
        12,
        "M1/T2",
        "ready",
        "",
        r#"".github/workflows/x.yml""#,
    );
    let d = plan(&fake)["data"].clone();
    for r in d["run"].as_array().unwrap() {
        if r["issue"] == 11 {
            assert!(!nums(&r["bundle"]).contains(&12), "{d}");
            assert!(!nums(&r["closes"]).contains(&12), "{d}");
        }
    }
}
