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

const M3: &str = "\n## M3: New\n\narea: new\n\n### Tasks\n\n- T1: Fresh\n  touch: src/n.rs\n\n### Done when\n\n- UC-M3-1: it works.\n";

/// Open the intake with the real command, then let its PR land as `mode` says: `open` (no PR yet),
/// `merged` (adds M3), `not-added` (merged, M3 not in the file), `no-base-file` (merged, the plan
/// file is missing at the PR's base), `bad-base` (merged, the file at the base does not parse) or `unmerged` (closed without merging). The plan file on disk
/// has M3 in every case. Returns the temp dir, the fake and the plan file.
fn intake_env(mode: &str) -> (tempfile::TempDir, FakeGithub, String) {
    let (tmp, fake) = (tempfile::tempdir().unwrap(), FakeGithub::start());
    let file = imported(&fake, tmp.path());
    let dir = tmp.path().to_str().unwrap();
    let args = [
        "-C", dir, "intake", "add M3", "--source", "human", "--plan", "plan.md",
    ];
    let (code, v) = common::run(&fake, "session_test", T0, &args);
    assert_eq!(code, 0, "{v}");
    let intake = v["data"]["issue"].as_u64().unwrap();
    let with_m3 = format!("{PLAN}{M3}");
    if mode != "open" {
        let body = format!("Closes #{intake}\n<!-- bosun:task-pr 60 -->");
        fake.pull(60, "plan", "plan-m3", &body);
        if mode == "unmerged" {
            fake.pull_set(60, "state", serde_json::json!("closed"));
        } else {
            fake.pull_merged(60, "merge60");
            if mode != "no-base-file" {
                let base = if mode == "bad-base" {
                    "not a plan\n"
                } else {
                    PLAN
                };
                fake.content("plan.md", "base0000", base);
            }
            let after = if mode == "not-added" { PLAN } else { &with_m3 };
            fake.content("plan.md", "merge60", after);
            fake.content("plan.md", "main", &with_m3);
            fake.first_parent("merge60", "base0000");
        }
        fake.close_issue(intake, T0);
    }
    write_plan(tmp.path(), &with_m3);
    (tmp, fake, file)
}

fn amend_after_intake(merged: bool, plan_file_in_pr: bool) -> (FakeGithub, serde_json::Value) {
    let mode = match (merged, plan_file_in_pr) {
        (false, _) => "open",
        (true, true) => "merged",
        (true, false) => "not-added",
    };
    let (tmp, fake, file) = intake_env(mode);
    let (code, v) = plan(&fake, tmp.path(), &["amend", &file]);
    assert_eq!(code, 0, "{v}");
    (fake, v)
}

#[test]
fn amend_dry_run_reports_the_intake_promotion_and_writes_nothing() {
    let (tmp, fake, file) = intake_env("merged");
    let before = writes(&fake);
    let (code, v) = plan(&fake, tmp.path(), &["amend", &file, "--dry-run"]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(writes(&fake), before);
    assert_eq!(
        v["data"]["would_promote"],
        serde_json::json!(["M3", "M3/T1"])
    );
}

#[test]
fn a_second_amend_after_the_intake_promotion_changes_nothing() {
    let (tmp, fake, file) = intake_env("merged");
    let (_, v) = plan(&fake, tmp.path(), &["amend", &file]);
    assert_eq!(v["data"]["promoted"], serde_json::json!(["M3", "M3/T1"]));
    let before = writes(&fake);
    let (code, v) = plan(&fake, tmp.path(), &["amend", &file]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(writes(&fake), before);
    assert_eq!(v["data"]["promoted"], serde_json::json!([]));
}

#[test]
fn the_born_closed_plan_task_is_reported_closed() {
    let (_, v) = amend_after_intake(true, true);
    let p = v["data"]["tasks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["id"] == "M3/P")
        .unwrap();
    assert_eq!(p["closed"], true, "{p}");
}

#[test]
fn a_plan_file_missing_at_the_base_counts_as_all_added() {
    let (tmp, fake, file) = intake_env("no-base-file");
    let (code, v) = plan(&fake, tmp.path(), &["amend", &file]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["promoted"], serde_json::json!(["M3", "M3/T1"]));
}

#[test]
fn a_plan_file_that_does_not_parse_at_the_base_is_not_trusted() {
    let (tmp, fake, file) = intake_env("bad-base");
    let (code, v) = plan(&fake, tmp.path(), &["amend", &file]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["promoted"], serde_json::json!([]));
}

/// Open an intake `text` with the real command and let PR `pr` merge it: the plan file is `before`
/// at `base` and `after` at `merge`, the intake closes as completed.
fn landed_intake(
    fake: &FakeGithub,
    dir: &Path,
    text: &str,
    pr: u64,
    shas: (&str, &str),
    files: (&str, &str),
) {
    let dir = dir.to_str().unwrap();
    let args = [
        "-C", dir, "intake", text, "--source", "human", "--plan", "plan.md",
    ];
    let (code, v) = common::run(fake, "session_test", T0, &args);
    assert_eq!(code, 0, "{v}");
    let intake = v["data"]["issue"].as_u64().unwrap();
    fake.pull(
        pr,
        "plan",
        &format!("plan-{pr}"),
        &format!("Closes #{intake}"),
    );
    fake.pull_merged(pr, shas.1);
    fake.pull_set(
        pr,
        "base",
        serde_json::json!({"sha": shas.0, "ref": "main"}),
    );
    fake.content("plan.md", shas.0, files.0);
    fake.content("plan.md", shas.1, files.1);
    fake.content("plan.md", "main", files.1);
    fake.first_parent(shas.1, shas.0);
    fake.close_issue(intake, T0);
}

fn plan_with(items: &[&str]) -> String {
    items.iter().fold(PLAN.to_string(), |p, m| {
        format!("{p}{}", M3.replace("M3", m))
    })
}

#[test]
fn two_merged_intakes_each_promote_their_own_item() {
    let (tmp, fake) = (tempfile::tempdir().unwrap(), FakeGithub::start());
    let file = imported(&fake, tmp.path());
    let (p0, p3, p34) = (
        PLAN.to_string(),
        plan_with(&["M3"]),
        plan_with(&["M3", "M4"]),
    );
    landed_intake(&fake, tmp.path(), "add M3", 60, ("b0", "m60"), (&p0, &p3));
    landed_intake(&fake, tmp.path(), "add M4", 61, ("b1", "m61"), (&p3, &p34));
    write_plan(tmp.path(), &p34);
    let (code, v) = plan(&fake, tmp.path(), &["amend", &file]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(
        v["data"]["promoted"],
        serde_json::json!(["M3", "M3/T1", "M4", "M4/T1"]),
        "{v}"
    );
    for (item, pr) in [("M3", "PR #60"), ("M4", "PR #61")] {
        let p = fake.issue_state(find(&fake, &format!("<!-- bosun:plan {item}/P -->")));
        let first = p
            .body
            .split("-->\n\n")
            .nth(1)
            .unwrap()
            .lines()
            .next()
            .unwrap();
        assert!(
            first.starts_with("Done by intake #") && first.contains(pr),
            "{}",
            p.body
        );
    }
}

#[test]
fn the_scan_stops_once_every_new_item_has_an_owner() {
    let (tmp, fake) = (tempfile::tempdir().unwrap(), FakeGithub::start());
    let file = imported(&fake, tmp.path());
    let (p0, p3, p34) = (
        PLAN.to_string(),
        plan_with(&["M3"]),
        plan_with(&["M3", "M4"]),
    );
    landed_intake(&fake, tmp.path(), "add M3", 60, ("b0", "m60"), (&p0, &p3));
    write_plan(tmp.path(), &p3);
    let (code, v) = plan(&fake, tmp.path(), &["amend", &file]);
    assert_eq!(code, 0, "{v}");
    landed_intake(&fake, tmp.path(), "add M4", 61, ("b1", "m61"), (&p3, &p34));
    write_plan(tmp.path(), &p34);
    let seen = fake.requests().len();
    let (code, v) = plan(&fake, tmp.path(), &["amend", &file]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(
        v["data"]["promoted"],
        serde_json::json!(["M4", "M4/T1"]),
        "{v}"
    );
    let later: Vec<_> = fake
        .requests()
        .into_iter()
        .skip(seen)
        .map(|r| r.path)
        .collect();
    assert!(later.iter().any(|p| p.ends_with("/pulls/61")), "{later:?}");
    assert!(!later.iter().any(|p| p.ends_with("/pulls/60")), "{later:?}");
}

#[test]
fn a_linked_pr_closed_without_merging_promotes_nothing() {
    let (tmp, fake, file) = intake_env("unmerged");
    let (code, v) = plan(&fake, tmp.path(), &["amend", &file]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["promoted"], serde_json::json!([]));
}

#[test]
fn a_linked_pr_into_another_branch_promotes_nothing() {
    let (tmp, fake, file) = intake_env("merged");
    fake.pull_set(
        60,
        "base",
        serde_json::json!({"sha": "base0000", "ref": "release"}),
    );
    let (code, v) = plan(&fake, tmp.path(), &["amend", &file]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["promoted"], serde_json::json!([]));
}

#[test]
fn amend_trusts_a_merged_intake_and_promotes_the_new_tasks_at_once() {
    let (fake, v) = amend_after_intake(true, true);
    assert_eq!(v["data"]["promoted"], serde_json::json!(["M3", "M3/T1"]));
    let p = fake.issue_state(find(&fake, "<!-- bosun:plan M3/P -->"));
    assert!(p.labels.contains(&"bosun:state/merged".to_string()));
    assert_eq!(p.state, "closed");
    assert_eq!(p.state_reason.as_deref(), Some("completed"));
    assert!(p.body.contains("Done by intake #"), "{}", p.body);
    assert!(p.body.contains("PR #60"), "{}", p.body);
    let t = fake.issue_state(find(&fake, "<!-- bosun:plan M3/T1 -->"));
    assert!(t.labels.contains(&"bosun:state/ready".to_string()));
}

#[test]
fn an_item_another_pr_added_between_the_base_and_the_merge_is_not_credited() {
    let (tmp, fake, file) = intake_env("merged");
    // The PR's base predates M3, but the merge commit's first parent already has it.
    fake.content("plan.md", "parent60", &format!("{PLAN}{M3}"));
    fake.first_parent("merge60", "parent60");
    let (code, v) = plan(&fake, tmp.path(), &["amend", &file]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["promoted"], serde_json::json!([]), "{v}");
}

#[test]
fn amend_trusts_only_what_the_default_branch_has() {
    let (tmp, fake, file) = intake_env("merged");
    let extra = M3.replace(
        "### Done when",
        "- T2: Unreviewed\n  touch: .github/workflows/ci.yml\n\n### Done when",
    );
    write_plan(tmp.path(), &format!("{PLAN}{extra}"));
    let (code, v) = plan(&fake, tmp.path(), &["amend", &file]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["promoted"], serde_json::json!([]), "{v}");
    let p = fake.issue_state(find(&fake, "<!-- bosun:plan M3/P -->"));
    assert!(p.labels.contains(&"bosun:state/ready".to_string()));
    assert_eq!(p.state, "open");
    let t = fake.issue_state(find(&fake, "<!-- bosun:plan M3/T2 -->"));
    assert!(t.labels.contains(&"bosun:state/planned".to_string()));
}

#[test]
fn amend_trusts_nothing_when_the_default_branch_has_no_parsable_plan() {
    for text in [None, Some("not a plan\n")] {
        let (tmp, fake, file) = intake_env("merged");
        {
            let mut s = fake.state.lock().unwrap();
            s.contents.remove("main:plan.md");
            if let Some(t) = text {
                s.contents.insert("main:plan.md".into(), t.into());
            }
        }
        let (code, v) = plan(&fake, tmp.path(), &["amend", &file]);
        assert_eq!(code, 0, "{v}");
        assert_eq!(
            v["data"]["promoted"],
            serde_json::json!([]),
            "{text:?}: {v}"
        );
    }
}

#[test]
fn amend_ignores_a_merged_intake_whose_pr_did_not_add_the_item() {
    let (fake, v) = amend_after_intake(true, false);
    assert_eq!(v["data"]["promoted"], serde_json::json!([]));
    let p = fake.issue_state(find(&fake, "<!-- bosun:plan M3/P -->"));
    assert_eq!(p.state, "open");
}

#[test]
fn amend_keeps_the_plan_task_when_the_intake_is_still_open() {
    let (fake, v) = amend_after_intake(false, true);
    assert_eq!(v["data"]["promoted"], serde_json::json!([]));
    let p = fake.issue_state(find(&fake, "<!-- bosun:plan M3/P -->"));
    assert!(p.labels.contains(&"bosun:state/ready".to_string()));
    assert_eq!(p.state, "open");
    let t = fake.issue_state(find(&fake, "<!-- bosun:plan M3/T1 -->"));
    assert!(t.labels.contains(&"bosun:state/planned".to_string()));
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
