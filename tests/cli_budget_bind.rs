//! Modes PR 7b: trusted writer bindings close the body-edit misses of the feature budget.
mod common;

use common::fake::FakeGithub;
use serde_json::Value;
use std::path::Path;

const NOW: &str = "2026-10-22T10:00:00Z";
const LATER: &str = "2026-10-22T10:20:00Z";
const EPIC: &str = r#"Epic<!-- bosun:epic {"item":"M1","plan":"plans/x.md"} -->"#;

fn plan_text(budget: &str) -> String {
    format!(
        "# Plan: widget\n\nproject: widget\n\n## M1: One\n\narea: core\nbudget_usd: {budget}\n\nWords.\n\n\
### Tasks\n- T1: a\n  touch: src/a/**\n- T2: b\n  touch: src/b/**\n\n### Done when\n- UC-M1-1: works\n"
    )
}

fn cfg_dir() -> tempfile::TempDir {
    let d = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(d.path().join(".bosun")).unwrap();
    d
}

fn run_in(fake: &FakeGithub, dir: &Path, args: &[&str]) -> (i32, Value) {
    let mut a = vec!["-C", dir.to_str().unwrap()];
    a.extend_from_slice(args);
    common::run(fake, "session_test", NOW, &a)
}

fn mode(fake: &FakeGithub, name: &str) {
    let t = format!("[mode]\nname = \"{name}\"\n");
    fake.content(".bosun/bosun.toml", "main", &t);
}

fn bind(plan: &str, item: &str, rest: &str) -> String {
    format!(r#"{{"bosun":"bind","session":"s","plan":"{plan}","item":"{item}"{rest}}}"#)
}

fn task_body(epic: &str) -> String {
    let e = if epic.is_empty() {
        String::new()
    } else {
        format!(r#""epic":{epic},"#)
    };
    format!(r#"<!-- bosun:task {{"item":"M1",{e}"task":"T1","touch":["src/a/**"]}} -->"#)
}

/// Epic #10 (budget 100), ready task #11 and merged task #12 that spent `usd`. With `bound`
/// the writer's binds are on all three; the bodies of #11 and #12 have lost their `epic`.
fn feature(bound: bool, usd: f64) -> FakeGithub {
    let fake = FakeGithub::start();
    mode(&fake, "booster");
    fake.issue(1, "bosun registry", &["bosun:registry"], "");
    let join = format!(
        r#"{{"bosun":"join","session":"session_test","origin":"spawned","depth":1,"expires":"{LATER}"}}"#
    );
    fake.comment(1, &join);
    fake.comment(
        1,
        r#"{"bosun":"epic-index","session":"s","plan":"plans/x.md","item":"M1","epic":10}"#,
    );
    // T2 is a mainfix task in the plan.
    let mf = |t: String| t.replace("touch: src/b/**", "touch: src/b/**\n  kind: mainfix");
    fake.content("plans/x.md", "main", &mf(plan_text("100")));
    fake.content("plans/y.md", "main", &plan_text("9999"));
    fake.issue(10, "Epic", &["bosun:state/ready", "bosun:area/core"], EPIC);
    let body = |e: &str| if bound { task_body("") } else { task_body(e) };
    fake.issue(11, "T1", &["bosun:state/ready"], &body("10"));
    fake.issue(
        12,
        "T3",
        &["bosun:state/merged"],
        &body("10").replace("T1", "T3"),
    );
    fake.comment(
        12,
        &format!(r#"{{"bosun":"spend","session":"s","usd":{usd},"at":"2026-10-22T03:00:00Z"}}"#),
    );
    if bound {
        let t = |n: u64, t: &str| {
            let rest = format!(r#","task":"{t}","epic":10"#);
            fake.comment(n, &bind("plans/x.md", "M1", &rest));
        };
        t(11, "T1");
        t(12, "T3");
        fake.comment(10, &bind("plans/x.md", "M1", r#","tasks":[11,12]"#));
    }
    fake
}

#[test]
fn a_bound_task_without_its_body_epic_is_still_guarded_and_its_spend_counts() {
    let (dir, fake) = (cfg_dir(), feature(true, 100.0));
    refused(&fake, dir.path(), "claim", "11");
    // Unbound, the body epic still guards.
    let (dir, fake) = (cfg_dir(), feature(false, 100.0));
    refused(&fake, dir.path(), "claim", "11");
}

#[test]
fn a_repointed_marker_and_a_later_forged_bind_change_nothing() {
    let (dir, fake) = (cfg_dir(), feature(true, 100.0));
    let big = EPIC.replace("plans/x.md", "plans/y.md");
    set_body(&fake, 10, &big);
    // The bind and the marker disagree now: refused by that, not by the spend.
    guarded(&fake, dir.path(), "11");
    let (dir, fake) = (cfg_dir(), feature(true, 100.0));
    fake.comment(10, &bind("plans/y.md", "M1", r#","tasks":[]"#));
    refused(&fake, dir.path(), "claim", "11");
}

#[test]
fn a_bound_epic_has_no_mainfix_label_exemption_and_an_unbound_one_keeps_it() {
    let (dir, fake) = (cfg_dir(), feature(true, 100.0));
    fake.label(10, "bosun:kind/mainfix");
    refused(&fake, dir.path(), "spawn", "10");
    let (dir, fake) = (cfg_dir(), feature(false, 100.0));
    fake.label(10, "bosun:kind/mainfix");
    let (code, v) = run_in(&fake, dir.path(), &["spawn", "10"]);
    assert_eq!(code, 0, "{v}");
}

/// Exit 5 from the budget guard with the spend of the feature.
fn refused(fake: &FakeGithub, dir: &Path, cmd: &str, n: &str) {
    let (code, v) = run_in(fake, dir, &[cmd, n]);
    assert_eq!(code, 5, "{v}");
    assert_eq!(v["data"]["check"], "budget_feature", "{v}");
    assert_eq!(v["data"]["spent_usd"], 100.0, "{v}");
}

fn set_body(fake: &FakeGithub, n: u64, body: &str) {
    let mut s = fake.state.lock().unwrap();
    s.issues.iter_mut().find(|i| i.number == n).unwrap().body = body.into();
}

fn binds(fake: &FakeGithub) -> usize {
    (1..=12)
        .flat_map(|n| fake.comments_of(n))
        .filter(|c| c.body.contains(r#""bosun":"bind""#))
        .count()
}

fn import(fake: &FakeGithub, dir: &Path) {
    let file = dir.join("plan.md");
    std::fs::write(&file, plan_text("100")).unwrap();
    let (code, v) = run_in(fake, dir, &["plan", "import", file.to_str().unwrap()]);
    assert_eq!(code, 0, "{v}");
}

#[test]
fn import_writes_binds_only_when_the_knob_is_on_and_never_twice() {
    let (dir, fake) = (cfg_dir(), FakeGithub::start());
    import(&fake, dir.path());
    assert_eq!(binds(&fake), 0, "gatekeeper writes no bind");
    let (dir, fake) = (cfg_dir(), FakeGithub::start());
    mode(&fake, "steady");
    import(&fake, dir.path());
    // The plan task, two tasks and the epic.
    assert_eq!(binds(&fake), 4);
    import(&fake, dir.path());
    assert_eq!(binds(&fake), 4, "a second import adds nothing");
}

fn amend(fake: &FakeGithub, dir: &Path) -> (i32, Value) {
    let file = dir.join("plan.md");
    run_in(fake, dir, &["plan", "amend", file.to_str().unwrap()])
}

fn number(fake: &FakeGithub, marker: &str) -> u64 {
    let s = fake.state.lock().unwrap();
    s.issues
        .iter()
        .find(|i| i.body.starts_with(marker))
        .unwrap()
        .number
}

#[test]
fn a_bound_epic_whose_body_has_task_meta_is_still_an_epic() {
    let (dir, fake) = (cfg_dir(), feature(true, 100.0));
    let body = format!("{EPIC}\n{}", task_body(""));
    set_body(&fake, 10, &body);
    refused(&fake, dir.path(), "spawn", "10");
}

#[test]
fn a_task_named_in_the_body_and_the_bind_counts_once() {
    let (dir, fake) = (cfg_dir(), feature(false, 100.0));
    let t = r#","task":"T3","epic":10"#;
    fake.comment(12, &bind("plans/x.md", "M1", t));
    fake.comment(11, &bind("plans/x.md", "M1", r#","task":"T1","epic":10"#));
    fake.comment(10, &bind("plans/x.md", "M1", r#","tasks":[11,12]"#));
    refused(&fake, dir.path(), "claim", "11");
}

#[test]
fn a_task_bound_to_an_epic_that_does_not_list_it_is_refused() {
    let (dir, fake) = (cfg_dir(), feature(false, 100.0));
    fake.issue(99, "Dummy", &[], "Epic");
    fake.comment(11, &bind("plans/x.md", "M1", r#","task":"T1","epic":99"#));
    let (code, v) = run_in(&fake, dir.path(), &["claim", "11"]);
    assert_eq!(code, 5, "{v}");
    assert_eq!(v["data"]["check"], "budget_feature");
}

#[test]
fn import_refuses_a_forged_first_bind_and_amend_backfills() {
    let (dir, fake) = (cfg_dir(), FakeGithub::start());
    import(&fake, dir.path());
    assert_eq!(binds(&fake), 0);
    let t1 = number(&fake, "<!-- bosun:plan M1/T1 -->");
    fake.comment(t1, &bind("plan.md", "M1", r#","task":"T1","epic":99"#));
    mode(&fake, "steady");
    let (code, v) = amend(&fake, dir.path());
    assert_eq!(code, 5, "{v}");
    assert!(
        v["error"]["message"]
            .as_str()
            .unwrap()
            .contains(&format!("on #{t1}")),
        "{v}"
    );
    assert!(
        v["error"]["fix"]
            .as_str()
            .unwrap()
            .contains("deletes the forged bind comment")
    );
    // A forged epic bind naming another plan.
    let (dir, fake) = (cfg_dir(), FakeGithub::start());
    import(&fake, dir.path());
    let e = number(&fake, "<!-- bosun:plan M1 -->");
    fake.comment(e, &bind("other.md", "M1", r#","tasks":[]"#));
    mode(&fake, "steady");
    assert_eq!(amend(&fake, dir.path()).0, 5);
    // Without a forgery, amend backfills an epic imported before the knob was on.
    let (dir, fake) = (cfg_dir(), FakeGithub::start());
    import(&fake, dir.path());
    mode(&fake, "steady");
    let (code, v) = amend(&fake, dir.path());
    assert_eq!(code, 0, "{v}");
    assert_eq!(binds(&fake), 4);
}

fn guarded(fake: &FakeGithub, dir: &Path, n: &str) {
    let (code, v) = run_in(fake, dir, &["claim", n]);
    assert_eq!(code, 5, "{v}");
    assert_eq!(v["data"]["check"], "budget_feature", "{v}");
}

fn task_issue(fake: &FakeGithub, n: u64, task: &str, epic: &str) {
    let e = if epic.is_empty() {
        String::new()
    } else {
        format!(r#""epic":{epic},"#)
    };
    let body = format!(
        r#"<!-- bosun:task {{"item":"M1",{e}"task":"{task}","touch":["src/n{n}/**"]}} -->"#
    );
    fake.issue(n, task, &["bosun:state/ready"], &body);
}

#[test]
fn a_forged_bind_to_a_mainfix_id_and_an_appended_epic_list_is_refused() {
    // T2 is a mainfix task in the plan; #13 is its real issue. #11 is bound to T2 by a forger.
    for body_task in ["T1", "T2"] {
        let (dir, fake) = (cfg_dir(), feature(false, 100.0));
        task_issue(&fake, 13, "T2", "10");
        set_body(&fake, 11, &task_body("10").replace("T1", body_task));
        fake.comment(11, &bind("plans/x.md", "M1", r#","task":"T2","epic":10"#));
        fake.comment(10, &bind("plans/x.md", "M1", r#","tasks":[11,13]"#));
        guarded(&fake, dir.path(), "11");
    }
}

#[test]
fn a_task_bound_to_a_later_epic_that_copies_the_real_item_is_refused() {
    let (dir, fake) = (cfg_dir(), feature(true, 100.0));
    let dummy = EPIC.replace("Epic", "Dummy");
    fake.issue(20, "Dummy", &["bosun:state/ready"], &dummy);
    task_issue(&fake, 13, "T2", "");
    fake.comment(13, &bind("plans/x.md", "M1", r#","task":"T2","epic":20"#));
    fake.comment(20, &bind("plans/x.md", "M1", r#","tasks":[13]"#));
    guarded(&fake, dir.path(), "13");
}

#[test]
fn a_bound_task_with_its_meta_block_deleted_is_still_guarded() {
    let (dir, fake) = (cfg_dir(), feature(true, 100.0));
    set_body(&fake, 11, "no meta any more");
    guarded(&fake, dir.path(), "11");
    let (code, v) = run_in(&fake, dir.path(), &["spawn", "11"]);
    assert_eq!(code, 5, "{v}");
    assert_eq!(v["data"]["check"], "budget_feature", "{v}");
}

#[test]
fn a_hand_made_task_cannot_borrow_the_id_of_a_closed_mainfix_task() {
    let (dir, fake) = (cfg_dir(), feature(true, 100.0));
    task_issue(&fake, 13, "T2", "");
    fake.comment(13, &bind("plans/x.md", "M1", r#","task":"T2","epic":10"#));
    fake.comment(10, &bind("plans/x.md", "M1", r#","tasks":[13]"#));
    fake.close_issue_as(13, NOW, "not_planned");
    task_issue(&fake, 14, "T2", "10");
    let (code, v) = run_in(&fake, dir.path(), &["claim", "14"]);
    assert_eq!(code, 5, "{v}");
    assert_eq!(v["data"]["check"], "budget_feature", "{v}");
    let msg = v["error"]["message"].as_str().unwrap();
    assert!(
        msg.contains("spent"),
        "refused by the budget, not the exemption: {msg}"
    );
}

#[test]
fn a_lower_epic_closed_as_duplicate_still_wins_over_the_new_one() {
    let (dir, fake) = (cfg_dir(), feature(true, 100.0));
    fake.close_issue_as(10, NOW, "duplicate");
    fake.issue(20, "Epic", &["bosun:state/ready"], EPIC);
    task_issue(&fake, 13, "T2", "20");
    fake.comment(13, &bind("plans/x.md", "M1", r#","task":"T2","epic":20"#));
    fake.comment(20, &bind("plans/x.md", "M1", r#","tasks":[13]"#));
    guarded(&fake, dir.path(), "13");
}

#[test]
fn an_unbound_rival_epic_copying_the_marker_is_refused() {
    let (dir, fake) = (cfg_dir(), feature(true, 100.0));
    fake.issue(20, "Rival", &["bosun:state/ready"], EPIC);
    task_issue(&fake, 21, "T1", "20");
    guarded(&fake, dir.path(), "21");
}

#[test]
fn binds_must_agree_with_the_epic_marker_and_the_task_body() {
    let (dir, fake) = (cfg_dir(), feature(true, 100.0));
    set_body(&fake, 10, &EPIC.replace("M1", "M2"));
    guarded(&fake, dir.path(), "11");
    let (dir, fake) = (cfg_dir(), feature(true, 100.0));
    set_body(&fake, 11, &task_body("99"));
    let (code, v) = run_in(&fake, dir.path(), &["claim", "11"]);
    assert_eq!(code, 5, "{v}");
    assert!(
        v["error"]["fix"]
            .as_str()
            .unwrap()
            .contains("deletes the bind comment")
    );
}

#[test]
fn an_open_bound_mainfix_task_claims() {
    let (dir, fake) = (cfg_dir(), feature(true, 100.0));
    task_issue(&fake, 13, "T2", "");
    fake.comment(13, &bind("plans/x.md", "M1", r#","task":"T2","epic":10"#));
    fake.comment(10, &bind("plans/x.md", "M1", r#","tasks":[13]"#));
    let (code, v) = run_in(&fake, dir.path(), &["claim", "13"]);
    assert_eq!(code, 0, "{v}");
}

#[test]
fn a_rival_is_found_by_the_real_epics_bind_whatever_its_body_says() {
    for body in [
        "Epic with no marker".to_string(),
        EPIC.replace("plans/x.md", "plans/y.md"),
        // A fake task block appended to the real epic does not hide it.
        format!(r#"{EPIC}<!-- bosun:task {{"item":"M9","task":"T1"}} -->"#),
    ] {
        let (dir, fake) = (cfg_dir(), feature(true, 100.0));
        set_body(&fake, 10, &body);
        fake.issue(20, "Rival", &["bosun:state/ready"], EPIC);
        task_issue(&fake, 21, "T1", "20");
        guarded(&fake, dir.path(), "21");
    }
}

#[test]
fn a_closed_canonical_epic_is_named_as_the_cause() {
    let (dir, fake) = (cfg_dir(), feature(true, 100.0));
    fake.close_issue_as(10, NOW, "duplicate");
    fake.issue(20, "Epic", &["bosun:state/ready"], EPIC);
    task_issue(&fake, 21, "T1", "20");
    let (code, v) = run_in(&fake, dir.path(), &["claim", "21"]);
    assert_eq!(code, 5, "{v}");
    let (msg, fix) = (
        v["error"]["message"].as_str().unwrap(),
        v["error"]["fix"].as_str().unwrap(),
    );
    assert!(
        msg.contains("#10 is the epic of") && fix.contains("work under #10"),
        "{v}"
    );
}
