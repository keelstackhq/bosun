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

## M2: Intake

area: intake

### Tasks

- T1: Import
  touch: src/cli/plan.rs

### Done when

- UC-M2-1: importing twice creates no duplicates.
";

fn write_plan(dir: &Path, text: &str) -> String {
    let p = dir.join("plan.md");
    std::fs::write(&p, text).unwrap();
    p.to_str().unwrap().to_string()
}

fn plan(fake: &FakeGithub, dir: &Path, args: &[&str]) -> (i32, Value) {
    let mut a = vec!["-C", dir.to_str().unwrap(), "plan"];
    a.extend_from_slice(args);
    common::run(fake, "session_test", T0, &a)
}

fn writes(fake: &FakeGithub) -> usize {
    fake.requests().iter().filter(|r| r.method != "GET").count()
}

fn find(fake: &FakeGithub, marker: &str) -> u64 {
    let s = fake.state.lock().unwrap();
    s.issues
        .iter()
        .find(|i| i.body.starts_with(marker))
        .unwrap_or_else(|| panic!("no issue with {marker}"))
        .number
}

fn ids(v: &Value) -> Vec<&str> {
    v.as_array()
        .unwrap()
        .iter()
        .map(|x| x.as_str().unwrap())
        .collect()
}

/// A fake with PLAN imported; returns the plan file path.
fn imported(fake: &FakeGithub, dir: &Path) -> String {
    let file = write_plan(dir, PLAN);
    let (code, v) = plan(fake, dir, &["import", &file]);
    assert_eq!(code, 0, "{v}");
    file
}

fn plan_without_m2() -> String {
    PLAN.split("## M2").next().unwrap().to_string()
}

#[test]
fn removing_an_item_closes_its_epic_tasks_and_plan_task() {
    let (tmp, fake) = (tempfile::tempdir().unwrap(), FakeGithub::start());
    let file = imported(&fake, tmp.path());
    write_plan(tmp.path(), &plan_without_m2());
    let (code, v) = plan(&fake, tmp.path(), &["amend", &file]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["command"], "plan amend");
    assert_eq!(ids(&v["data"]["closed"]), ["M2", "M2/P", "M2/T1"]);
    assert_eq!(v["data"]["updated"], serde_json::json!([]));
    for m in ["M2", "M2/P", "M2/T1"] {
        let n = find(&fake, &format!("<!-- bosun:plan {m} -->"));
        let i = fake.issue_state(n);
        assert_eq!(i.state, "closed", "{m}");
        assert_eq!(i.state_reason.as_deref(), Some("not_planned"), "{m}");
        let c = fake.comments_of(n);
        assert_eq!(c.len(), 1, "{m}");
        assert_eq!(c[0].body, "Removed from plan.md by an amendment.");
    }
    for m in ["M1", "M1/P", "M1/T1", "M1/T2"] {
        let n = find(&fake, &format!("<!-- bosun:plan {m} -->"));
        assert_eq!(fake.issue_state(n).state, "open", "{m}");
    }
}

#[test]
fn removing_a_task_closes_only_that_task() {
    let (tmp, fake) = (tempfile::tempdir().unwrap(), FakeGithub::start());
    let file = imported(&fake, tmp.path());
    write_plan(
        tmp.path(),
        &PLAN.replace("- T2: Stop rules\n  touch: src/core/stop.rs\n", ""),
    );
    let (code, v) = plan(&fake, tmp.path(), &["amend", &file]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(ids(&v["data"]["closed"]), ["M1/T2"]);
    let t2 = fake.issue_state(find(&fake, "<!-- bosun:plan M1/T2 -->"));
    assert_eq!(t2.state_reason.as_deref(), Some("not_planned"));
    for m in ["M1", "M1/P", "M1/T1", "M2", "M2/P", "M2/T1"] {
        let n = find(&fake, &format!("<!-- bosun:plan {m} -->"));
        assert_eq!(fake.issue_state(n).state, "open", "{m}");
    }
}

#[test]
fn a_changed_task_is_updated_in_place() {
    let (tmp, fake) = (tempfile::tempdir().unwrap(), FakeGithub::start());
    let file = imported(&fake, tmp.path());
    let t1 = find(&fake, "<!-- bosun:plan M1/T1 -->");
    let epic = find(&fake, "<!-- bosun:plan M1 -->");
    {
        let mut s = fake.state.lock().unwrap();
        let i = s.issues.iter_mut().find(|i| i.number == t1).unwrap();
        i.body.push_str("\n\nHuman note.");
        i.labels.retain(|l| l != "bosun:state/planned");
        i.labels.push("bosun:state/claimed".into());
    }
    write_plan(
        tmp.path(),
        &PLAN.replace(
            "- T1: State machine\n  touch: src/core/state.rs\n",
            "- T1: State machine v2\n  touch: src/core/state.rs, src/core/lease.rs\n  kind: bookkeeping\n  risk: high\n",
        ),
    );
    let (code, v) = plan(&fake, tmp.path(), &["amend", &file]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(ids(&v["data"]["updated"]), ["M1/T1"]);
    assert_eq!(v["data"]["closed"], serde_json::json!([]));
    let i = fake.issue_state(t1);
    assert_eq!(i.title, "M1/T1: State machine v2");
    assert_eq!(i.state, "open");
    assert!(i.body.contains("Human note."));
    assert!(i.body.starts_with("<!-- bosun:plan M1/T1 -->"));
    let m = bosun::core::meta::parse_task_meta(&i.body).unwrap();
    assert_eq!(m.touch, ["src/core/state.rs", "src/core/lease.rs"]);
    assert_eq!((m.kind.as_str(), m.risk.as_str()), ("bookkeeping", "high"));
    assert_eq!(m.epic, Some(epic));
    assert!(i.labels.contains(&"bosun:kind/bookkeeping".to_string()));
    assert!(!i.labels.contains(&"bosun:kind/feature".to_string()));
    assert!(i.labels.contains(&"bosun:state/claimed".to_string()));
    assert!(!i.labels.contains(&"bosun:state/planned".to_string()));
    // the untouched task keeps its title
    let t2 = fake.issue_state(find(&fake, "<!-- bosun:plan M1/T2 -->"));
    assert_eq!(t2.title, "M1/T2: Stop rules");
}

#[test]
fn other_plans_and_hand_made_issues_are_never_touched() {
    let (tmp, fake) = (tempfile::tempdir().unwrap(), FakeGithub::start());
    let file = imported(&fake, tmp.path());
    let planned = ["bosun:state/planned"];
    // Another plan file's epic and task (marker, but a different `plan`).
    fake.issue(
        900,
        "X1: other",
        &planned,
        r#"<!-- bosun:plan X1 -->
<!-- bosun:epic {"item":"X1","area":"a","plan":"other.md","depends":[],"ucs":[]} -->"#,
    );
    fake.issue(
        901,
        "X1/T1: other task",
        &planned,
        r#"<!-- bosun:plan X1/T1 -->

<!-- bosun:task {"item":"X1","task":"T1","epic":900,"touch":["a"]} -->"#,
    );
    // Hand-made: no marker, even with this plan's file in the epic metadata.
    fake.issue(
        902,
        "M7: by hand",
        &planned,
        &format!(
            r#"<!-- bosun:epic {{"item":"M7","area":"a","plan":{},"depends":[],"ucs":[]}} -->"#,
            serde_json::to_string(&file).unwrap()
        ),
    );
    fake.issue(
        903,
        "M7/T1: by hand",
        &planned,
        r#"<!-- bosun:task {"item":"M7","task":"T1","epic":902,"touch":["a"]} -->"#,
    );
    let before: Vec<_> = [900, 901, 902, 903]
        .iter()
        .map(|n| fake.issue_state(*n))
        .collect();
    let (code, v) = plan(&fake, tmp.path(), &["amend", &file]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["closed"], serde_json::json!([]));
    for (n, b) in [900, 901, 902, 903].iter().zip(before) {
        let a = fake.issue_state(*n);
        assert_eq!(
            (a.state, a.title, a.body, a.labels),
            (b.state, b.title, b.body, b.labels)
        );
    }
}

#[test]
fn dry_run_reports_and_writes_nothing() {
    let (tmp, fake) = (tempfile::tempdir().unwrap(), FakeGithub::start());
    let file = imported(&fake, tmp.path());
    write_plan(
        tmp.path(),
        &plan_without_m2().replace("touch: src/core/state.rs", "touch: src/core/other.rs"),
    );
    let before = writes(&fake);
    let (code, v) = plan(&fake, tmp.path(), &["amend", &file, "--dry-run"]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(writes(&fake), before);
    assert_eq!(v["data"]["dry_run"], true);
    assert_eq!(ids(&v["data"]["would_close"]), ["M2", "M2/P", "M2/T1"]);
    assert_eq!(ids(&v["data"]["would_update"]), ["M1/T1"]);
    assert!(v["data"].get("closed").is_none());
}

#[test]
fn a_second_amend_changes_nothing() {
    let (tmp, fake) = (tempfile::tempdir().unwrap(), FakeGithub::start());
    let file = imported(&fake, tmp.path());
    write_plan(
        tmp.path(),
        &plan_without_m2().replace("State machine", "State machine v2"),
    );
    let (code, v) = plan(&fake, tmp.path(), &["amend", &file]);
    assert_eq!(code, 0, "{v}");
    let before = writes(&fake);
    let (code, v) = plan(&fake, tmp.path(), &["amend", &file]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(writes(&fake), before);
    assert_eq!(v["data"]["closed"], serde_json::json!([]));
    assert_eq!(v["data"]["updated"], serde_json::json!([]));
}

#[test]
fn amend_also_imports_new_items() {
    let (tmp, fake) = (tempfile::tempdir().unwrap(), FakeGithub::start());
    let file = imported(&fake, tmp.path());
    write_plan(
        tmp.path(),
        &format!(
            "{PLAN}\n## M3: New\n\narea: new\n\n### Tasks\n\n- T1: Fresh\n  touch: src/n.rs\n\n### Done when\n\n- UC-M3-1: it works.\n"
        ),
    );
    let (code, v) = plan(&fake, tmp.path(), &["amend", &file]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["epics"][2]["action"], "created");
    assert_eq!(v["data"]["epics"][0]["action"], "existing");
}

#[test]
fn every_spelling_of_the_plan_path_reconciles_the_same() {
    for spelling in ["plans/x.md", "./plans/x.md", "ABS"] {
        let (tmp, fake) = (tempfile::tempdir().unwrap(), FakeGithub::start());
        std::fs::create_dir(tmp.path().join("plans")).unwrap();
        let abs = tmp.path().join("plans/x.md");
        std::fs::write(&abs, PLAN).unwrap();
        let abs = std::fs::canonicalize(&abs).unwrap();
        let spelling = if spelling == "ABS" {
            abs.to_str().unwrap()
        } else {
            spelling
        };
        // Import with the spelling under test: the epic stores the relative form.
        let (code, v) = plan(&fake, tmp.path(), &["import", spelling]);
        assert_eq!(code, 0, "{spelling}: {v}");
        let epic = fake.issue_state(find(&fake, "<!-- bosun:plan M2 -->"));
        assert!(
            epic.body.contains(r#""plan":"plans/x.md""#),
            "{}",
            epic.body
        );
        std::fs::write(&abs, plan_without_m2()).unwrap();
        let (code, v) = plan(&fake, tmp.path(), &["amend", spelling]);
        assert_eq!(code, 0, "{spelling}: {v}");
        assert_eq!(
            ids(&v["data"]["closed"]),
            ["M2", "M2/P", "M2/T1"],
            "{spelling}"
        );
        assert_eq!(v["data"]["warnings"], serde_json::json!([]), "{spelling}");
    }
}

#[test]
fn an_unknown_path_warns_and_reconciles_nothing() {
    let (tmp, fake) = (tempfile::tempdir().unwrap(), FakeGithub::start());
    imported(&fake, tmp.path());
    std::fs::write(tmp.path().join("other.md"), plan_without_m2()).unwrap();
    let (code, v) = plan(&fake, tmp.path(), &["amend", "other.md"]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["closed"], serde_json::json!([]));
    let w = v["data"]["warnings"].to_string();
    assert!(
        w.contains("no epic names other.md; nothing reconciled"),
        "{w}"
    );
}

#[test]
fn an_active_task_is_not_closed_and_keeps_its_epic_open() {
    let (tmp, fake) = (tempfile::tempdir().unwrap(), FakeGithub::start());
    let file = imported(&fake, tmp.path());
    let t = find(&fake, "<!-- bosun:plan M2/T1 -->");
    let epic = find(&fake, "<!-- bosun:plan M2 -->");
    {
        let mut s = fake.state.lock().unwrap();
        let i = s.issues.iter_mut().find(|i| i.number == t).unwrap();
        i.labels.retain(|l| l != "bosun:state/planned");
        i.labels.push("bosun:state/building".into());
    }
    write_plan(tmp.path(), &plan_without_m2());
    let (code, v) = plan(&fake, tmp.path(), &["amend", &file]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(ids(&v["data"]["closed"]), ["M2/P"]);
    assert_eq!(ids(&v["data"]["skipped"]), ["M2", "M2/T1"]);
    assert_eq!(fake.issue_state(t).state, "open");
    assert_eq!(fake.issue_state(epic).state, "open");
    let w = v["data"]["warnings"].to_string();
    assert!(
        w.contains(&format!("M2/T1 #{t} is building; not closed")),
        "{w}"
    );
    assert!(
        w.contains(&format!("M2 #{epic} has active tasks; left open")),
        "{w}"
    );
}

#[test]
fn a_task_with_a_live_lease_is_not_closed() {
    let (tmp, fake) = (tempfile::tempdir().unwrap(), FakeGithub::start());
    let file = imported(&fake, tmp.path());
    let t = find(&fake, "<!-- bosun:plan M1/T2 -->");
    // Planned by label, but a live lease holds it.
    fake.comment(
        t,
        &common::lease_note("session_other", "2026-10-23T11:00:00Z", "src/core/stop.rs"),
    );
    write_plan(
        tmp.path(),
        &PLAN.replace("- T2: Stop rules\n  touch: src/core/stop.rs\n", ""),
    );
    let (code, v) = plan(&fake, tmp.path(), &["amend", &file]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(ids(&v["data"]["skipped"]), ["M1/T2"]);
    assert_eq!(fake.issue_state(t).state, "open");
}

#[test]
fn a_re_added_item_reopens_its_epic_and_tasks() {
    let (tmp, fake) = (tempfile::tempdir().unwrap(), FakeGithub::start());
    let file = imported(&fake, tmp.path());
    write_plan(tmp.path(), &plan_without_m2());
    let (code, _) = plan(&fake, tmp.path(), &["amend", &file]);
    assert_eq!(code, 0);
    let epic = find(&fake, "<!-- bosun:plan M2 -->");
    assert_eq!(fake.issue_state(epic).state, "closed");
    let posts = |f: &FakeGithub| {
        f.requests()
            .iter()
            .filter(|r| r.method == "POST" && r.path.ends_with("/issues"))
            .count()
    };
    let before = posts(&fake);
    write_plan(tmp.path(), PLAN);
    let (code, v) = plan(&fake, tmp.path(), &["amend", &file]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(ids(&v["data"]["reopened"]), ["M2", "M2/P", "M2/T1"]);
    assert_eq!(posts(&fake), before);
    assert_eq!(fake.issue_state(epic).state, "open");
    let c = fake.comments_of(epic);
    assert_eq!(c.last().unwrap().body, "Re-added by an amendment.");
    let t = find(&fake, "<!-- bosun:plan M2/T1 -->");
    assert_eq!(fake.issue_state(t).state, "open");
}

#[test]
fn wave_plan_does_not_count_not_planned_as_merged() {
    let fake = FakeGithub::start();
    fake.issue(10, "Epic", &["bosun:state/claimed"], "");
    let body = |t: &str, dep: &str| {
        format!(
            r#"<!-- bosun:task {{"item":"M1","task":"{t}","epic":10,"depends":[{dep}],"touch":["src/{t}/**"]}} -->"#
        )
    };
    fake.issue(11, "M1/T1", &["bosun:state/planned"], &body("T1", ""));
    fake.issue(
        12,
        "M1/T2",
        &["bosun:state/ready"],
        &body("T2", r#""M1/T1""#),
    );
    fake.close_issue_as(11, T0, "not_planned");
    let (code, v) = common::run(&fake, "session_test", T0, &["wave", "plan", "10"]);
    assert_eq!(code, 0, "{v}");
    // #11 is out of the plan and does not satisfy #12's dependency.
    assert_eq!(v["data"]["run"].as_array().unwrap().len(), 0, "{v}");
    assert_eq!(v["data"]["waiting"][0]["issue"], 12);
    // Completed does count.
    fake.close_issue_as(11, T0, "completed");
    let (_, v) = common::run(&fake, "session_test", T0, &["wave", "plan", "10"]);
    assert_eq!(v["data"]["run"][0]["issue"], 12, "{v}");
}
