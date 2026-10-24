mod common;

use common::fake::FakeGithub;
use serde_json::{Value, json};
use std::path::Path;

const T0: &str = "2026-10-23T10:00:00Z";

const PLAN: &str = "# Plan: widget

project: widget

## M1: Lifecycle

area: lifecycle

The state machine.

### Tasks

- T1: State machine
  touch: src/core/state.rs
- T2: Stop rules
  touch: src/core/stop.rs
  depends: T1
  risk: high

### Done when

- UC-M1-1: a task moves planned -> merged.

## M2: Intake

area: intake
depends: M1

### Tasks

- T1: Import
  touch: src/cli/plan.rs
  depends: M1/T2
  kind: bookkeeping

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

fn posts(fake: &FakeGithub) -> usize {
    fake.requests()
        .iter()
        .filter(|r| r.method == "POST" && r.path.ends_with("/issues"))
        .count()
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

#[test]
fn import_creates_epics_plan_tasks_and_tasks() {
    let (tmp, fake) = (tempfile::tempdir().unwrap(), FakeGithub::start());
    let file = write_plan(tmp.path(), PLAN);
    let (code, v) = plan(&fake, tmp.path(), &["import", &file]);
    assert_eq!(code, 0, "{v}");
    // 2 epics + 2 plan tasks + 3 tasks
    assert_eq!(posts(&fake), 7);
    assert_eq!(v["data"]["dry_run"], false);
    let epics = v["data"]["epics"].as_array().unwrap();
    assert_eq!(epics.len(), 2);
    assert!(epics.iter().all(|e| e["action"] == "created"));
    let tasks = v["data"]["tasks"].as_array().unwrap();
    let ids: Vec<&str> = tasks.iter().map(|t| t["id"].as_str().unwrap()).collect();
    assert_eq!(ids, ["M1/P", "M1/T1", "M1/T2", "M2/P", "M2/T1"]);
    assert_eq!(tasks[0]["state"], "ready");
    assert_eq!(tasks[1]["state"], "planned");

    let epic = find(&fake, "<!-- bosun:plan M1 -->");
    assert_eq!(epics[0]["issue"], epic);
    let e = fake.issue_state(epic);
    assert_eq!(e.title, "M1: Lifecycle");
    for l in [
        "bosun:kind/feature",
        "bosun:area/lifecycle",
        "bosun:source/plan",
        "bosun:state/planned",
    ] {
        assert!(e.labels.contains(&l.to_string()), "{l} in {:?}", e.labels);
    }
    assert!(e.body.contains("The state machine."));
    assert!(e.body.contains("- [ ] M1/T1: State machine"));
    assert!(e.body.contains("UC-M1-1"));
    let epic_json = e
        .body
        .split("<!-- bosun:epic ")
        .nth(1)
        .unwrap()
        .split(" -->")
        .next()
        .unwrap();
    let em: Value = serde_json::from_str(epic_json).unwrap();
    assert_eq!(em["item"], "M1");
    assert_eq!(em["area"], "lifecycle");
    assert_eq!(em["plan"], "plan.md");
    assert_eq!(em["ucs"], json!(["UC-M1-1"]));
    let m2 = fake.issue_state(find(&fake, "<!-- bosun:plan M2 -->"));
    assert!(m2.body.contains(r#""depends":["M1"]"#));

    let p = fake.issue_state(find(&fake, "<!-- bosun:plan M1/P -->"));
    assert_eq!(p.title, "M1/P: plan");
    assert!(p.labels.contains(&"bosun:kind/plan".to_string()));
    assert!(p.labels.contains(&"bosun:state/ready".to_string()));
    assert!(!p.body.contains("#[ignore"));
    assert!(p.body.contains("UC-M1-1"));
    let meta = bosun::core::meta::parse_task_meta(&p.body).unwrap();
    assert_eq!(meta.epic, Some(epic));
    assert_eq!(meta.touch, ["plan.md"]);
    assert_eq!(meta.kind, "plan");
    assert!(meta.allow.is_empty());
    assert!(
        !tmp.path().join("tests").exists(),
        "import writes no test files"
    );
    assert!(v["data"].get("stubs").is_none());

    let t = fake.issue_state(find(&fake, "<!-- bosun:plan M1/T2 -->"));
    assert_eq!(t.title, "M1/T2: Stop rules");
    assert!(t.labels.contains(&"bosun:kind/feature".to_string()));
    assert!(t.labels.contains(&"bosun:state/planned".to_string()));
    let meta = bosun::core::meta::parse_task_meta(&t.body).unwrap();
    assert_eq!(meta.epic, Some(epic));
    assert_eq!(meta.depends, ["M1/T1"]);
    assert_eq!((meta.risk.as_str(), meta.allow.len()), ("high", 0));
    let t = fake.issue_state(find(&fake, "<!-- bosun:plan M2/T1 -->"));
    assert!(t.labels.contains(&"bosun:kind/bookkeeping".to_string()));
    assert_eq!(
        bosun::core::meta::parse_task_meta(&t.body).unwrap().depends,
        ["M1/T2"]
    );
    assert!(
        fake.label_names()
            .contains(&"bosun:state/ready".to_string())
    );

    let before = posts(&fake);
    let (code, v) = plan(&fake, tmp.path(), &["import", &file]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(posts(&fake), before);
    assert!(
        v["data"]["tasks"]
            .as_array()
            .unwrap()
            .iter()
            .all(|t| t["action"] == "existing")
    );
    assert!(v["data"]["epics"][0]["action"] == "existing");
    assert_eq!(v["data"]["tasks"][0]["state"], "ready");
}

#[test]
fn a_hand_made_task_issue_is_adopted_unchanged() {
    let (tmp, fake) = (tempfile::tempdir().unwrap(), FakeGithub::start());
    let body = r#"Hand made.

<!-- bosun:task {"item":"M1","task":"T1","touch":["src/core/state.rs"]} -->"#;
    fake.issue(5, "M1/T1: by hand", &["bosun:state/claimed"], body);
    let file = write_plan(tmp.path(), PLAN);
    let (code, v) = plan(&fake, tmp.path(), &["import", &file]);
    assert_eq!(code, 0, "{v}");
    let t = &v["data"]["tasks"][1];
    assert_eq!(
        (t["id"].as_str(), t["issue"].as_u64(), t["action"].as_str()),
        (Some("M1/T1"), Some(5), Some("adopted"))
    );
    assert_eq!(t["state"], "claimed");
    let i = fake.issue_state(5);
    assert_eq!(i.body, body);
    assert_eq!(i.labels, ["bosun:state/claimed"]);
    // 2 epics + 2 plan tasks + 2 tasks; T1 of M1 is not duplicated
    assert_eq!(posts(&fake), 6);
    assert!(
        fake.requests()
            .iter()
            .all(|r| !r.path.ends_with("/issues/5") || r.method == "GET")
    );
}

#[test]
fn an_epic_is_adopted_by_its_epic_marker() {
    let (tmp, fake) = (tempfile::tempdir().unwrap(), FakeGithub::start());
    fake.issue(
        9,
        "M1: old epic",
        &["bosun:state/planned"],
        r#"<!-- bosun:epic {"item":"M1","area":"lifecycle"} -->"#,
    );
    let file = write_plan(tmp.path(), PLAN);
    let (_, v) = plan(&fake, tmp.path(), &["import", &file]);
    assert_eq!(v["data"]["epics"][0]["issue"], 9);
    assert_eq!(v["data"]["epics"][0]["action"], "adopted");
    let meta = bosun::core::meta::parse_task_meta(
        &fake
            .issue_state(find(&fake, "<!-- bosun:plan M1/T1 -->"))
            .body,
    )
    .unwrap();
    assert_eq!(meta.epic, Some(9));
}

#[test]
fn dry_run_writes_nothing() {
    let (tmp, fake) = (tempfile::tempdir().unwrap(), FakeGithub::start());
    let file = write_plan(tmp.path(), PLAN);
    let (code, v) = plan(&fake, tmp.path(), &["import", &file, "--dry-run"]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(writes(&fake), 0);
    assert_eq!(v["data"]["dry_run"], true);
    assert_eq!(v["data"]["epics"][0]["action"], "would-create");
    assert_eq!(v["data"]["epics"][0]["issue"], Value::Null);
    assert_eq!(v["data"]["tasks"].as_array().unwrap().len(), 5);
    assert_eq!(v["data"]["tasks"][0]["state"], "ready");
}

#[test]
fn item_filters_and_unknown_item_is_usage() {
    let (tmp, fake) = (tempfile::tempdir().unwrap(), FakeGithub::start());
    let file = write_plan(tmp.path(), PLAN);
    let (code, v) = plan(&fake, tmp.path(), &["import", &file, "--item", "M2"]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(posts(&fake), 3);
    assert_eq!(v["data"]["epics"].as_array().unwrap().len(), 1);
    let (code, v) = plan(&fake, tmp.path(), &["import", &file, "--item", "M9"]);
    assert_eq!(code, 2, "{v}");
    assert_eq!(posts(&fake), 3);
}

#[test]
fn parse_errors_exit_1_with_all_errors_and_write_nothing() {
    let (tmp, fake) = (tempfile::tempdir().unwrap(), FakeGithub::start());
    let bad = "# Plan\n\n## m1: Bad\n\n## M2: Also bad\n";
    let file = write_plan(tmp.path(), bad);
    for cmd in ["import", "check"] {
        let (code, v) = plan(&fake, tmp.path(), &[cmd, &file]);
        assert_eq!(code, 1, "{v}");
        assert!(
            v["error"]["message"]
                .as_str()
                .unwrap()
                .contains("errors in")
        );
        assert!(v["data"]["errors"].as_array().unwrap().len() >= 2, "{v}");
    }
    assert!(fake.requests().is_empty());
}

#[test]
fn project_mismatch_exits_1() {
    let (tmp, fake) = (tempfile::tempdir().unwrap(), FakeGithub::start());
    let file = write_plan(
        tmp.path(),
        &PLAN.replace("project: widget", "project: other"),
    );
    let (code, v) = plan(&fake, tmp.path(), &["import", &file]);
    assert_eq!(code, 1, "{v}");
    assert_eq!(posts(&fake), 0);
}

#[test]
fn halted_import_exits_4_but_dry_run_does_not() {
    let (tmp, fake) = (tempfile::tempdir().unwrap(), FakeGithub::start());
    fake.variable("BOSUN_HALT", "1");
    let file = write_plan(tmp.path(), PLAN);
    let (code, _) = plan(&fake, tmp.path(), &["import", &file]);
    assert_eq!(code, 4);
    let (code, _) = plan(&fake, tmp.path(), &["import", &file, "--dry-run"]);
    assert_eq!(code, 0);
    assert_eq!(posts(&fake), 0);
}

#[test]
fn check_summarises_a_good_plan_without_github() {
    let (tmp, fake) = (tempfile::tempdir().unwrap(), FakeGithub::start());
    let file = write_plan(tmp.path(), PLAN);
    let (code, v) = plan(&fake, tmp.path(), &["check", &file]);
    assert_eq!(code, 0, "{v}");
    assert!(fake.requests().is_empty());
    assert_eq!(v["data"]["title"], "Plan: widget");
    assert_eq!(v["data"]["project"], "widget");
    assert_eq!(
        v["data"]["items"][0],
        json!({"id":"M1","title":"Lifecycle","area":"lifecycle","tasks":2,"ucs":1})
    );
    assert_eq!(v["data"]["warnings"], json!([]));
}

fn close(fake: &FakeGithub, number: u64) {
    let mut s = fake.state.lock().unwrap();
    s.issues
        .iter_mut()
        .find(|i| i.number == number)
        .unwrap()
        .state = "closed".into();
}

#[test]
fn closed_issues_are_reported_not_recreated() {
    let (tmp, fake) = (tempfile::tempdir().unwrap(), FakeGithub::start());
    fake.issue(1, "M1: x", &[], "<!-- bosun:plan M1 -->");
    fake.issue(
        2,
        "by hand",
        &["bosun:state/merged"],
        r#"<!-- bosun:task {"item":"M1","task":"T1"} -->"#,
    );
    close(&fake, 1);
    close(&fake, 2);
    let file = write_plan(tmp.path(), PLAN);
    let (code, v) = plan(&fake, tmp.path(), &["import", &file]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(posts(&fake), 5);
    let e = &v["data"]["epics"][0];
    assert_eq!(
        (e["action"].as_str(), e["closed"].as_bool()),
        (Some("existing"), Some(true))
    );
    let t = &v["data"]["tasks"][1];
    assert_eq!(
        (
            t["issue"].as_u64(),
            t["action"].as_str(),
            t["closed"].as_bool()
        ),
        (Some(2), Some("adopted"), Some(true))
    );
    assert!(v["data"]["tasks"][0].get("closed").is_none());
}

#[test]
fn a_failure_part_way_reports_progress_and_the_rerun_finishes() {
    let (tmp, fake) = (tempfile::tempdir().unwrap(), FakeGithub::start());
    let file = write_plan(tmp.path(), PLAN);
    // Creates fail from the request after the epic and the plan task were made.
    fake.after_requests("/issues", 3, |s| {
        s.fail_paths.push(("/issues".into(), 400));
    });
    let (code, v) = plan(&fake, tmp.path(), &["import", &file]);
    assert_eq!(code, 1, "{v}");
    assert_eq!(v["data"]["epics"].as_array().unwrap().len(), 1);
    assert_eq!(v["data"]["epics"][0]["action"], "created");
    assert_eq!(v["data"]["tasks"].as_array().unwrap().len(), 1);
    assert_eq!(v["data"]["tasks"][0]["id"], "M1/P");
    fake.state.lock().unwrap().fail_paths.clear();
    let (code, v) = plan(&fake, tmp.path(), &["import", &file]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["epics"][0]["action"], "existing");
    assert_eq!(v["data"]["tasks"][0]["action"], "existing");
    assert_eq!(v["data"]["tasks"][1]["action"], "created");
    assert_eq!(fake.state.lock().unwrap().issues.len(), 7);
}

#[test]
fn several_matches_use_the_lowest_number_and_warn() {
    let (tmp, fake) = (tempfile::tempdir().unwrap(), FakeGithub::start());
    fake.issue(9, "dup", &[], "<!-- bosun:plan M1/T1 -->");
    fake.issue(5, "first", &[], "<!-- bosun:plan M1/T1 -->");
    let file = write_plan(tmp.path(), PLAN);
    let (_, v) = plan(&fake, tmp.path(), &["import", &file]);
    assert_eq!(v["data"]["tasks"][1]["issue"], 5);
    assert_eq!(
        v["data"]["warnings"],
        json!(["M1/T1 matches #5 and #9; using #5"])
    );
}

fn set_labels(fake: &FakeGithub, number: u64, labels: &[&str]) {
    let mut s = fake.state.lock().unwrap();
    let i = s.issues.iter_mut().find(|i| i.number == number).unwrap();
    i.labels = labels.iter().map(|l| l.to_string()).collect();
}

fn close_as(fake: &FakeGithub, number: u64, reason: &str) {
    let mut s = fake.state.lock().unwrap();
    let i = s.issues.iter_mut().find(|i| i.number == number).unwrap();
    i.state = "closed".into();
    i.state_reason = Some(reason.into());
}

fn state_labels(fake: &FakeGithub, marker: &str) -> Vec<String> {
    fake.issue_state(find(fake, marker))
        .labels
        .into_iter()
        .filter(|l| l.starts_with("bosun:state/"))
        .collect()
}

#[test]
fn tasks_become_ready_after_the_plan_task_merged() {
    let (tmp, fake) = (tempfile::tempdir().unwrap(), FakeGithub::start());
    let file = write_plan(tmp.path(), PLAN);
    let (code, v) = plan(&fake, tmp.path(), &["import", &file]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["promoted"], json!([]));
    // A hand-made planned task of M2 is adopted and never touched.
    fake.issue(
        900,
        "hand made",
        &["bosun:state/planned"],
        r#"<!-- bosun:task {"item":"M2","task":"T9","touch":["src/x.rs"]} -->"#,
    );
    // Not merged yet: a re-import promotes nothing.
    let (_, v) = plan(&fake, tmp.path(), &["import", &file]);
    assert_eq!(v["data"]["promoted"], json!([]));
    assert_eq!(
        state_labels(&fake, "<!-- bosun:plan M1/T1 -->"),
        ["bosun:state/planned"]
    );

    let p1 = find(&fake, "<!-- bosun:plan M1/P -->");
    set_labels(&fake, p1, &["bosun:state/merged"]);
    let before = writes(&fake);
    let (code, v) = plan(&fake, tmp.path(), &["import", &file, "--dry-run"]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["would_promote"], json!(["M1", "M1/T1", "M1/T2"]));
    assert_eq!(writes(&fake), before);

    let (code, v) = plan(&fake, tmp.path(), &["import", &file]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["promoted"], json!(["M1", "M1/T1", "M1/T2"]));
    for m in ["M1", "M1/T1", "M1/T2"] {
        let marker = format!("<!-- bosun:plan {m} -->");
        assert_eq!(state_labels(&fake, &marker), ["bosun:state/ready"], "{m}");
    }
    // M2's plan task is not merged: M2 stays planned.
    assert_eq!(
        state_labels(&fake, "<!-- bosun:plan M2/T1 -->"),
        ["bosun:state/planned"]
    );
    assert_eq!(
        state_labels(&fake, "<!-- bosun:plan M2 -->"),
        ["bosun:state/planned"]
    );

    // A plan task closed as completed counts too; the adopted issue stays planned.
    close_as(&fake, find(&fake, "<!-- bosun:plan M2/P -->"), "completed");
    let (_, v) = plan(&fake, tmp.path(), &["import", &file]);
    assert_eq!(v["data"]["promoted"], json!(["M2", "M2/T1"]));
    assert_eq!(fake.issue_state(900).labels, ["bosun:state/planned"]);
    let (_, v) = plan(&fake, tmp.path(), &["import", &file]);
    assert_eq!(v["data"]["promoted"], json!([]));
}

#[test]
fn a_plan_task_closed_as_not_planned_promotes_nothing() {
    let (tmp, fake) = (tempfile::tempdir().unwrap(), FakeGithub::start());
    let file = write_plan(tmp.path(), PLAN);
    plan(&fake, tmp.path(), &["import", &file]);
    close_as(
        &fake,
        find(&fake, "<!-- bosun:plan M1/P -->"),
        "not_planned",
    );
    let (_, v) = plan(&fake, tmp.path(), &["import", &file]);
    assert_eq!(v["data"]["promoted"], json!([]));
}

#[test]
fn issues_missing_from_the_lagging_list_are_still_found() {
    let (tmp, fake) = (tempfile::tempdir().unwrap(), FakeGithub::start());
    let file = write_plan(tmp.path(), PLAN);
    plan(&fake, tmp.path(), &["import", &file]);
    let made = posts(&fake);
    let top = fake
        .state
        .lock()
        .unwrap()
        .issues
        .iter()
        .map(|i| i.number)
        .max()
        .unwrap();
    for n in top - 2..=top {
        fake.hide_from_list(n);
    }
    let (code, v) = plan(&fake, tmp.path(), &["import", &file]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(posts(&fake), made);
    assert!(v["data"]["warnings"].as_array().unwrap().is_empty());
}

#[test]
fn issues_closed_as_duplicate_never_match_or_warn() {
    let (tmp, fake) = (tempfile::tempdir().unwrap(), FakeGithub::start());
    let file = write_plan(tmp.path(), PLAN);
    // The lowest number would win, but it is a closed duplicate.
    fake.issue(1, "old dup", &[], "<!-- bosun:plan M1/T1 -->");
    close_as(&fake, 1, "duplicate");
    let (code, v) = plan(&fake, tmp.path(), &["import", &file]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(posts(&fake), 7, "{v}");
    let (_, v) = plan(&fake, tmp.path(), &["import", &file]);
    assert_eq!(posts(&fake), 7);
    assert!(v["data"]["warnings"].as_array().unwrap().is_empty(), "{v}");
    let row = v["data"]["tasks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["id"] == "M1/T1")
        .unwrap();
    assert_ne!(row["issue"], 1);
}

#[test]
fn the_probe_skips_deleted_numbers_and_pulls_and_stops_at_a_missing_one() {
    let (tmp, fake) = (tempfile::tempdir().unwrap(), FakeGithub::start());
    let file = write_plan(tmp.path(), PLAN);
    fake.issue(1, "unrelated", &[], "");
    fake.pull_as_issue(2, "a pull");
    fake.issue(3, "deleted", &[], "");
    fake.gone(3);
    plan(&fake, tmp.path(), &["import", &file]);
    let made = posts(&fake);
    let top = fake
        .state
        .lock()
        .unwrap()
        .issues
        .iter()
        .map(|i| i.number)
        .max()
        .unwrap();
    for n in 4..=top {
        fake.hide_from_list(n);
    }
    let (code, v) = plan(&fake, tmp.path(), &["import", &file]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(posts(&fake), made, "{v}");
    assert!(v["data"]["warnings"].as_array().unwrap().is_empty(), "{v}");
}

fn with_profile(profile: &str) -> tempfile::TempDir {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(tmp.path().join(".bosun")).unwrap();
    std::fs::write(
        tmp.path().join(".bosun/bosun.toml"),
        format!("project = \"widget\"\n\n[[stack]]\npath = \".\"\nprofile = \"{profile}\"\n"),
    )
    .unwrap();
    tmp
}

fn tree(dir: &Path) -> Vec<String> {
    let mut out: Vec<String> = walk(dir, dir);
    out.sort();
    out
}

fn walk(root: &Path, dir: &Path) -> Vec<String> {
    let mut out = vec![];
    for e in std::fs::read_dir(dir).unwrap().flatten() {
        let p = e.path();
        if p.is_dir() {
            out.extend(walk(root, &p));
        } else {
            out.push(p.strip_prefix(root).unwrap().to_str().unwrap().to_string());
        }
    }
    out
}

#[test]
fn a_dry_run_writes_no_files_and_no_issues() {
    let (tmp, fake) = (with_profile("rust"), FakeGithub::start());
    let file = write_plan(tmp.path(), PLAN);
    let files = tree(tmp.path());
    let before = writes(&fake);
    let (code, v) = plan(&fake, tmp.path(), &["import", &file, "--dry-run"]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["dry_run"], true);
    assert!(v["data"].get("would_write").is_none(), "{v}");
    assert!(v["data"].get("stubs").is_none(), "{v}");
    assert_eq!(v["data"]["would_promote"], json!([]));
    assert!(!tmp.path().join("tests").exists());
    assert_eq!(tree(tmp.path()), files);
    assert_eq!(writes(&fake), before);
}

#[test]
fn existing_uc_files_stay_byte_identical_across_imports() {
    let (tmp, fake) = (with_profile("rust"), FakeGithub::start());
    let file = write_plan(tmp.path(), PLAN);
    std::fs::create_dir_all(tmp.path().join("tests")).unwrap();
    let with_id = "// UC-M1-1 done\n#[test]\nfn it() {}\n";
    let without_id = "#[test]\nfn other() {}";
    std::fs::write(tmp.path().join("tests/uc_m1.rs"), with_id).unwrap();
    std::fs::write(tmp.path().join("tests/uc_m2.rs"), without_id).unwrap();
    for _ in 0..2 {
        let (code, v) = plan(&fake, tmp.path(), &["import", &file]);
        assert_eq!(code, 0, "{v}");
        assert_eq!(
            std::fs::read_to_string(tmp.path().join("tests/uc_m1.rs")).unwrap(),
            with_id
        );
        assert_eq!(
            std::fs::read_to_string(tmp.path().join("tests/uc_m2.rs")).unwrap(),
            without_id
        );
        assert_eq!(tree(&tmp.path().join("tests")), ["uc_m1.rs", "uc_m2.rs"]);
    }
}

#[test]
fn other_profiles_import_without_files_or_a_stub_warning() {
    let (tmp, fake) = (with_profile("pnpm"), FakeGithub::start());
    let file = write_plan(tmp.path(), PLAN);
    let (code, v) = plan(&fake, tmp.path(), &["import", &file]);
    assert_eq!(code, 0, "{v}");
    assert!(!tmp.path().join("tests").exists());
    assert_eq!(v["data"]["warnings"], json!([]));
    assert_eq!(posts(&fake), 7);
}


#[test]
fn each_plan_task_touches_the_plan_and_lists_its_ucs() {
    let (tmp, fake) = (tempfile::tempdir().unwrap(), FakeGithub::start());
    let file = write_plan(tmp.path(), PLAN);
    let (code, v) = plan(&fake, tmp.path(), &["import", &file]);
    assert_eq!(code, 0, "{v}");
    for (item, uc, text) in [
        ("M1", "UC-M1-1", "a task moves planned -> merged."),
        ("M2", "UC-M2-1", "importing twice creates no duplicates."),
    ] {
        let p = fake.issue_state(find(&fake, &format!("<!-- bosun:plan {item}/P -->")));
        assert_eq!(p.title, format!("{item}/P: plan"));
        assert!(p.body.contains(&format!("- {uc}: {text}")), "{}", p.body);
        assert!(p.body.contains("passing tests"), "{}", p.body);
        assert!(p.body.contains("plan.md"), "{}", p.body);
        let meta = bosun::core::meta::parse_task_meta(&p.body).unwrap();
        assert_eq!(meta.task, "P");
        assert_eq!(meta.touch, ["plan.md"]);
        assert!(meta.allow.is_empty());
    }
}
