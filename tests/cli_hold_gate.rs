//! `bosun hold` and the gate (design F6): a held task stays out of claim until the owner ticks
//! its box; the checklist keeps the box when it is rewritten.

mod common;

use common::fake::FakeGithub;
use common::run;
use serde_json::Value;
use std::path::Path;

const NOW: &str = "2026-10-27T10:00:00Z";
const BASE: &str = "# Plan: Ship it\n\nproject: x\n\n## M1: One\n\narea: core\ndepends: seed\n\nWords.\n\n\
### Tasks\n- T1: a\n  touch: docs/a.md\n- T2: b\n  touch: src/x.rs\n### Done when\n- UC-M1-1: works\n";
const REPLACED: &str = "\n## Prerequisites\n\n- H1: token\n  on: phone\n  steps: add it\n  verify: owner\n  blocks: M1/T1\n  replaces: X1\n";
const STEP: &str = "Create the npm token";

fn setup(dir: &Path) -> FakeGithub {
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
    fake.content("plans/x.md", "main", BASE);
    std::fs::create_dir_all(dir.join("plans")).unwrap();
    std::fs::write(dir.join("plans/x.md"), BASE).unwrap();
    fake
}

fn bosun(fake: &FakeGithub, dir: &Path, args: &[&str]) -> (i32, Value) {
    let mut a = vec!["-C", dir.to_str().unwrap()];
    a.extend(args);
    run(fake, "session_a", NOW, &a)
}

/// Claim #11 and hold it for X1.
fn held(fake: &FakeGithub, dir: &Path) {
    assert_eq!(bosun(fake, dir, &["claim", "11"]).0, 0);
    let (code, v) = bosun(fake, dir, &["hold", "11", STEP]);
    assert_eq!((code, &v["data"]["id"]), (0, &"X1".into()), "{v}");
}

fn checklist_body(fake: &FakeGithub) -> String {
    let s = fake.state.lock().unwrap();
    let i = s
        .issues
        .iter()
        .find(|i| i.labels.contains(&"bosun:prereqs".to_string()));
    i.expect("the checklist issue").body.clone()
}

fn edit_checklist(fake: &FakeGithub, f: impl Fn(&str) -> String) {
    let mut s = fake.state.lock().unwrap();
    let i = s
        .issues
        .iter_mut()
        .find(|i| i.labels.contains(&"bosun:prereqs".to_string()));
    let i = i.expect("the checklist issue");
    i.body = f(&i.body);
}

fn tick(fake: &FakeGithub, id: &str) {
    edit_checklist(fake, |b| {
        b.replace(&format!("- [ ] **{id}**"), &format!("- [x] **{id}**"))
    });
}

#[test]
fn a_held_task_is_refused_until_the_owner_ticks_the_box() {
    let tmp = tempfile::tempdir().unwrap();
    let fake = setup(tmp.path());
    held(&fake, tmp.path());
    let (code, v) = bosun(&fake, tmp.path(), &["claim", "11"]);
    assert_eq!(code, 5, "{v}");
    let msg = v["error"]["message"].as_str().unwrap();
    assert_eq!(msg, "#11 (M1/T1) waits for owner steps X1 of plans/x.md");
    assert_eq!(
        bosun(&fake, tmp.path(), &["claim", "12"]).0,
        0,
        "another task is free"
    );
    tick(&fake, "X1");
    let (code, v) = bosun(&fake, tmp.path(), &["claim", "11"]);
    assert_eq!(code, 0, "{v}");
}

#[test]
fn deleting_the_record_keeps_an_unticked_box_blocking_and_deleting_the_box_keeps_the_record() {
    let tmp = tempfile::tempdir().unwrap();
    let fake = setup(tmp.path());
    held(&fake, tmp.path());
    fake.state
        .lock()
        .unwrap()
        .comments
        .retain(|c| !c.body.contains(r#""bosun":"hold""#));
    let (code, v) = bosun(&fake, tmp.path(), &["claim", "11"]);
    assert_eq!(code, 5, "the box alone holds the task: {v}");
    assert!(v["error"]["message"].as_str().unwrap().contains("X1"));

    let tmp = tempfile::tempdir().unwrap();
    let fake = setup(tmp.path());
    held(&fake, tmp.path());
    edit_checklist(&fake, |b| {
        let keep: Vec<&str> = b
            .lines()
            .filter(|l| !l.contains("bosun:prereq X1"))
            .collect();
        keep.join("\n")
    });
    let (code, v) = bosun(&fake, tmp.path(), &["claim", "11"]);
    assert_eq!(code, 5, "the record alone holds the task: {v}");
    assert!(v["error"]["message"].as_str().unwrap().contains("X1"));
}

#[test]
fn a_ticked_box_with_a_changed_sum_does_not_free_the_task() {
    let tmp = tempfile::tempdir().unwrap();
    let fake = setup(tmp.path());
    held(&fake, tmp.path());
    tick(&fake, "X1");
    let sum = bosun::core::prereqs::hold_sum(STEP);
    edit_checklist(&fake, |b| b.replace(&sum, "00000000"));
    assert_eq!(
        bosun(&fake, tmp.path(), &["claim", "11"]).0,
        5,
        "a forged box is not the record's"
    );
}

#[test]
fn prereqs_post_keeps_the_box_and_its_tick() {
    let tmp = tempfile::tempdir().unwrap();
    let fake = setup(tmp.path());
    held(&fake, tmp.path());
    let post = ["prereqs", "plans/x.md", "--post", "--no-setup"];
    let (code, v) = bosun(&fake, tmp.path(), &post);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["open"], 1, "the open hold counts: {v}");
    let body = checklist_body(&fake);
    assert!(
        body.contains("### Found during the work\n- [ ] **X1**"),
        "{body}"
    );
    tick(&fake, "X1");
    let (code, v) = bosun(&fake, tmp.path(), &post);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["open"], 0, "{v}");
    let body = checklist_body(&fake);
    assert!(
        body.contains("- [x] **X1**") && body.contains("task=11 -->"),
        "{body}"
    );
    let ids: Vec<&str> = v["data"]["plans"][0]["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| i["id"].as_str().unwrap())
        .collect();
    assert_eq!(ids, ["X1"]);
    assert_eq!(bosun(&fake, tmp.path(), &["claim", "11"]).0, 0);
}

#[test]
fn a_plan_item_that_replaces_the_hold_governs_the_task() {
    let tmp = tempfile::tempdir().unwrap();
    let fake = setup(tmp.path());
    held(&fake, tmp.path());
    let plan = format!("{BASE}{REPLACED}");
    fake.content("plans/x.md", "main", &plan);
    std::fs::write(tmp.path().join("plans/x.md"), &plan).unwrap();
    let post = ["prereqs", "plans/x.md", "--post", "--no-setup"];
    let (code, v) = bosun(&fake, tmp.path(), &post);
    assert_eq!(code, 0, "{v}");
    let body = checklist_body(&fake);
    assert!(
        body.contains("**H1**") && !body.contains("**X1**"),
        "the hold is replaced: {body}"
    );
    let (code, v) = bosun(&fake, tmp.path(), &["claim", "11"]);
    assert_eq!(code, 5, "H1 is open: {v}");
    assert_eq!(
        v["error"]["message"],
        "#11 (M1/T1) waits for owner steps H1 of plans/x.md"
    );
    tick(&fake, "H1");
    let (code, v) = bosun(&fake, tmp.path(), &["claim", "11"]);
    assert_eq!(
        code, 0,
        "H1 done frees the task although X1 never was ticked: {v}"
    );
}

/// The plan replaces X1 (on #11), and `--post` drops its box.
fn replaced(fake: &FakeGithub, dir: &Path, blocks: &str) {
    held(fake, dir);
    let plan = format!("{BASE}{}", REPLACED.replace("M1/T1", blocks));
    fake.content("plans/x.md", "main", &plan);
    std::fs::write(dir.join("plans/x.md"), &plan).unwrap();
    let post = ["prereqs", "plans/x.md", "--post", "--no-setup"];
    assert_eq!(bosun(fake, dir, &post).0, 0);
}

#[test]
fn a_hold_after_a_replaced_one_gets_a_new_id_and_keeps_its_task_held() {
    let tmp = tempfile::tempdir().unwrap();
    let fake = setup(tmp.path());
    replaced(&fake, tmp.path(), "M1/T1");
    assert_eq!(bosun(&fake, tmp.path(), &["claim", "12"]).0, 0);
    let (code, v) = bosun(&fake, tmp.path(), &["hold", "12", "Enroll the key"]);
    assert_eq!((code, &v["data"]["id"]), (0, &"X2".into()), "{v}");
    let (code, v) = bosun(&fake, tmp.path(), &["claim", "12"]);
    assert_eq!(code, 5, "{v}");
}

#[test]
fn replaces_only_releases_a_hold_of_a_task_the_item_blocks() {
    let tmp = tempfile::tempdir().unwrap();
    let fake = setup(tmp.path());
    // H1 names X1 (a hold of #11, M1/T1) but blocks only M1/T2: X1 still holds #11.
    replaced(&fake, tmp.path(), "M1/T2");
    assert!(checklist_body(&fake).contains("**X1**"), "the box stays");
    tick(&fake, "H1");
    let (code, v) = bosun(&fake, tmp.path(), &["claim", "11"]);
    assert_eq!(code, 5, "{v}");
    assert!(v["error"]["message"].as_str().unwrap().contains("X1"));
}
