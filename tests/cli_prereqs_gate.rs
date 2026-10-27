//! Front-loaded prerequisites (design F5): claim, `wave plan` and spawn honour the owner checklist.
//! Ids, blocks and verify kinds come from the plan file; the checklist issue only gives ticks.

mod common;

use bosun::core::prereqs::digest;
use common::fake::FakeGithub;
use common::run;
use serde_json::{Value, json};

const NOW: &str = "2026-10-27T10:00:00Z";
const BASE: &str = "# Plan: X\n\nproject: x\n\n## M1: One\n\narea: core\ndepends: seed\n\nWords.\n\n\
### Tasks\n- T1: a\n  touch: docs/a.md\n- T2: b\n  touch: src/x.rs\n### Done when\n- UC-M1-1: works\n";
const SECRET: &str = "environments/release/secrets/NPM_TOKEN";

/// The plan with one prerequisite H1 that blocks `blocks`, verified by `verify`.
fn plan(blocks: &str, verify: &str) -> String {
    format!(
        "{BASE}\n## Prerequisites\n\n- H1: token\n  on: phone\n  steps: add it\n  verify: {verify}\n  blocks: {blocks}\n"
    )
}

/// A checklist line for `id`, with the sum of the owner-verified H1 of [`plan`].
fn line(id: &str, tick: bool, api_ok: bool) -> String {
    let b = if tick { "x" } else { " " };
    let v = if api_ok { "api ok" } else { "owner" };
    let sum = digest("token", "add it", "owner", "owner");
    format!("- [{b}] **{id}** thing <!-- bosun:prereq {id} sum={sum} state={v} -->")
}

/// Registry #1, epic #10 (plan `plans/x.md`, item M1), tasks #11 (M1/T1) and #12 (M1/T2), ready.
fn setup(text: &str) -> FakeGithub {
    let fake = FakeGithub::start();
    fake.issue(1, "bosun registry", &["bosun:registry"], "");
    let epic = r#"<!-- bosun:epic {"item":"M1","plan":"plans/x.md"} -->"#;
    fake.issue(10, "Epic", &["bosun:state/ready"], epic);
    for (n, t) in [(11, "T1"), (12, "T2")] {
        let body = format!(
            r#"<!-- bosun:task {{"item":"M1","task":"{t}","epic":10,"touch":["src/{t}/**"]}} -->"#
        );
        fake.issue(n, &format!("Task {t}"), &["bosun:state/ready"], &body);
    }
    fake.content("plans/x.md", "main", text);
    fake
}

fn checklist(fake: &FakeGithub, n: u64, path: &str, lines: &[String]) {
    let body = format!("<!-- bosun:prereqs {path} -->\n{}", lines.join("\n"));
    fake.issue(n, "Owner checklist: X", &["bosun:prereqs"], &body);
}

fn claim(fake: &FakeGithub, n: &str) -> (i32, Value) {
    run(fake, "session_a", NOW, &["claim", n])
}

fn untouched(fake: &FakeGithub, n: u64) {
    assert!(fake.comments_of(n).is_empty(), "no lease comment");
    let s = fake.state.lock().unwrap();
    let issue = s.issues.iter().find(|i| i.number == n).unwrap();
    assert_eq!(issue.labels, ["bosun:state/ready"], "no label change");
}

#[test]
fn claim_of_a_blocked_task_is_exit_5_and_writes_nothing() {
    let fake = setup(&plan("M1/T1", "owner"));
    checklist(&fake, 20, "plans/x.md", &[line("H1", false, false)]);
    let (code, v) = claim(&fake, "11");
    assert_eq!(code, 5, "{v}");
    let msg = v["error"]["message"].as_str().unwrap();
    assert_eq!(msg, "#11 (M1/T1) waits for owner steps H1 of plans/x.md");
    assert!(v["error"]["fix"].as_str().unwrap().contains("#20"));
    untouched(&fake, 11);
    let (code, v) = claim(&fake, "12");
    assert_eq!(code, 0, "a task H1 does not block: {v}");
}

#[test]
fn a_ticked_owner_item_releases_the_task() {
    let fake = setup(&plan("M1", "owner"));
    checklist(&fake, 20, "plans/x.md", &[line("H1", true, false)]);
    assert_eq!(claim(&fake, "11").0, 0);
}

#[test]
fn the_issue_cannot_forge_an_api_item_and_the_api_decides() {
    let fake = setup(&plan("M1", "secret release/NPM_TOKEN"));
    checklist(&fake, 20, "plans/x.md", &[line("H1", true, true)]);
    let (code, v) = claim(&fake, "11");
    assert_eq!(code, 5, "ticked and marked ok, the secret missing: {v}");
    untouched(&fake, 11);
    fake.present(SECRET);
    assert_eq!(claim(&fake, "11").0, 0, "the API found it, no tick needed");
}

#[test]
fn two_checklist_issues_for_a_plan_are_refused() {
    let fake = setup(&plan("M1", "owner"));
    checklist(&fake, 20, "plans/x.md", &[line("H1", true, false)]);
    checklist(&fake, 21, "plans/x.md", &[line("H1", true, false)]);
    let (code, v) = claim(&fake, "11");
    assert_eq!(code, 5, "{v}");
    assert_eq!(v["error"]["message"], "two checklist issues for plans/x.md");
    untouched(&fake, 11);
}

#[test]
fn a_missing_box_or_a_repeated_box_is_open() {
    let fake = setup(&plan("M1", "owner"));
    checklist(&fake, 20, "plans/x.md", &[line("H2", true, false)]);
    assert_eq!(claim(&fake, "11").0, 5, "no box for H1");
    let fake = setup(&plan("M1", "owner"));
    let twice = [line("H1", true, false), line("H1", true, false)];
    checklist(&fake, 20, "plans/x.md", &twice);
    assert_eq!(claim(&fake, "11").0, 5, "H1 appears twice");
}

#[test]
fn a_plan_with_prerequisites_and_no_checklist_issue_fails_closed() {
    let fake = setup(&plan("M1", "owner"));
    let (code, v) = claim(&fake, "11");
    assert_eq!(code, 5, "{v}");
    let (msg, fix) = (&v["error"]["message"], v["error"]["fix"].as_str().unwrap());
    assert_eq!(msg, "plans/x.md has prerequisites but no checklist issue");
    assert!(fix.contains("bosun prereqs plans/x.md --post"));
    untouched(&fake, 11);
}

#[test]
fn a_plan_file_missing_on_the_default_branch_fails_closed() {
    let fake = setup("");
    fake.state.lock().unwrap().contents.clear();
    let (code, v) = claim(&fake, "11");
    assert_eq!(code, 5, "{v}");
    assert!(
        v["error"]["fix"]
            .as_str()
            .unwrap()
            .contains("bosun prereqs plans/x.md --post")
    );
    untouched(&fake, 11);
}

#[test]
fn a_plan_without_the_section_has_no_gate() {
    let fake = setup(BASE);
    assert_eq!(claim(&fake, "11").0, 0);
}

#[test]
fn the_same_item_id_in_another_plan_does_not_count() {
    let fake = setup(&plan("M1", "owner"));
    checklist(&fake, 20, "plans/other.md", &[line("H1", true, false)]);
    assert_eq!(
        claim(&fake, "11").0,
        5,
        "plans/x.md has no issue of its own"
    );
    checklist(&fake, 21, "plans/x.md", &[line("H1", true, false)]);
    assert_eq!(claim(&fake, "11").0, 0, "its own box is ticked");
}

#[test]
fn spawn_of_an_epic_blocked_as_a_whole_is_exit_5() {
    let fake = setup(&plan("M1", "owner"));
    let join = r#"{"bosun":"join","session":"session_a","origin":"spawned","depth":1,"expires":"2026-10-27T11:00:00Z"}"#;
    fake.comment(1, join);
    checklist(&fake, 20, "plans/x.md", &[line("H1", false, false)]);
    let (code, v) = run(&fake, "session_a", NOW, &["spawn", "10"]);
    assert_eq!(code, 5, "{v}");
    assert!(v["error"]["message"].as_str().unwrap().contains("H1"));
    assert!(fake.comments_of(10).is_empty(), "nothing written");
}

#[test]
fn a_403_tick_a_stale_sum_and_a_misspelt_heading() {
    let api = plan("M1", "secret release/NPM_TOKEN");
    let sum = digest("token", "add it", "secret release/NPM_TOKEN", "owner");
    for (ticked, want) in [(false, 5), (true, 0)] {
        let fake = setup(&api);
        fake.fail_path("secrets/NPM_TOKEN", 403);
        let b = if ticked { "x" } else { " " };
        let l = format!("- [{b}] **H1** t <!-- bosun:prereq H1 sum={sum} state=api 403 -->");
        checklist(&fake, 20, "plans/x.md", &[l]);
        assert_eq!(claim(&fake, "11").0, want, "a 403 falls back to the tick");
    }
    let fake = setup(&plan("M1", "owner"));
    let stale =
        line("H1", true, false).replace(&digest("token", "add it", "owner", "owner"), "00000000");
    checklist(&fake, 20, "plans/x.md", &[stale]);
    assert_eq!(claim(&fake, "11").0, 5, "a tick on a stale sum");
    let fake = setup(&plan("M1", "owner").replace("## Prerequisites", "## Pre-requisites"));
    assert_eq!(claim(&fake, "11").0, 5, "a misspelt heading fails closed");
}

#[test]
fn only_items_that_block_the_target_count() {
    let fake = setup(&plan("M1/T2", "owner"));
    assert_eq!(
        claim(&fake, "11").0,
        0,
        "H1 blocks only T2: no issue needed for T1"
    );
    let join = r#"{"bosun":"join","session":"session_a","origin":"spawned","depth":1,"expires":"2026-10-27T11:00:00Z"}"#;
    fake.comment(1, join);
    let (code, v) = run(&fake, "session_a", NOW, &["spawn", "10"]);
    assert_ne!(code, 5, "a task-level block does not hold the epic: {v}");
}

#[test]
fn wave_plan_leaves_out_the_held_task_and_lists_it() {
    let fake = setup(&plan("M1/T1", "owner"));
    checklist(&fake, 20, "plans/x.md", &[line("H1", false, false)]);
    let (code, v) = run(&fake, "session_a", NOW, &["wave", "plan", "10"]);
    assert_eq!(code, 0, "{v}");
    let ids = v["data"]["run"].as_array().unwrap().iter();
    let ids: Vec<u64> = ids.map(|t| t["issue"].as_u64().unwrap()).collect();
    assert_eq!(ids, [12]);
    assert_eq!(
        v["data"]["held"],
        json!([{"issue": 11, "task": "M1/T1", "needs": ["H1"]}])
    );
    assert_eq!(v["data"]["warnings"], json!([]));
}

#[test]
fn wave_plan_without_a_checklist_issue_leaves_out_the_plan_with_a_warning() {
    let fake = setup(&plan("M1/T1", "owner"));
    let (code, v) = run(&fake, "session_a", NOW, &["wave", "plan", "10"]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["run"], json!([]));
    let w = v["data"]["warnings"][0].as_str().unwrap();
    assert!(
        w.contains("has prerequisites but no checklist issue"),
        "{w}"
    );
}

#[test]
fn a_plan_path_that_could_break_a_marker_fails_closed() {
    let fake = setup(&plan("M1", "owner"));
    let epic = r#"<!-- bosun:epic {"item":"M1","plan":"plans/x y.md"} -->"#;
    fake.state
        .lock()
        .unwrap()
        .issues
        .iter_mut()
        .find(|i| i.number == 10)
        .unwrap()
        .body = epic.into();
    assert_eq!(claim(&fake, "11").0, 5);
}
