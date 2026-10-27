//! `bosun hold` (design F6): a human step found mid-run becomes a record on the task and a box
//! in the plan's owner checklist; the task goes back to ready.

mod common;

use common::fake::FakeGithub;
use common::run;
use serde_json::Value;

const NOW: &str = "2026-10-27T10:00:00Z";
const PLAN: &str = "# Plan: Ship it\n\nproject: x\n\n## M1: One\n\narea: core\ndepends: seed\n\nWords.\n\n\
### Tasks\n- T1: a\n  touch: docs/a.md\n- T2: b\n  touch: src/x.rs\n### Done when\n- UC-M1-1: works\n";
const STEP: &str = "Create the npm token and add it as NPM_TOKEN";

/// Registry #1, epic #10 (plan `plans/x.md`, item M1), tasks #11 and #12, ready.
fn setup() -> FakeGithub {
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
    fake.content("plans/x.md", "main", PLAN);
    fake
}

fn bosun(fake: &FakeGithub, dir: &std::path::Path, args: &[&str]) -> (i32, Value) {
    let mut a = vec!["-C", dir.to_str().unwrap()];
    a.extend(args);
    run(fake, "session_a", NOW, &a)
}

fn labels(fake: &FakeGithub, n: u64) -> Vec<String> {
    let s = fake.state.lock().unwrap();
    s.issues
        .iter()
        .find(|i| i.number == n)
        .unwrap()
        .labels
        .clone()
}

fn checklists(fake: &FakeGithub) -> Vec<(u64, String, String)> {
    let s = fake.state.lock().unwrap();
    let is = s
        .issues
        .iter()
        .filter(|i| i.labels.contains(&"bosun:prereqs".to_string()));
    is.map(|i| (i.number, i.state.clone(), i.body.clone()))
        .collect()
}

fn claimed(fake: &FakeGithub, dir: &std::path::Path, n: &str) {
    let (code, v) = bosun(fake, dir, &["claim", n]);
    assert_eq!(code, 0, "{v}");
}

#[test]
fn hold_posts_the_record_adds_the_box_and_readies_the_task() {
    let (tmp, fake) = (tempfile::tempdir().unwrap(), setup());
    claimed(&fake, tmp.path(), "11");
    let (code, v) = bosun(&fake, tmp.path(), &["hold", "11", STEP, "--phone"]);
    assert_eq!(code, 0, "{v}");
    let d = &v["data"];
    assert_eq!(
        (&d["issue"], &d["id"], &d["released"]),
        (&11.into(), &"X1".into(), &true.into())
    );
    assert!(d["lesson_error"].is_null(), "{v}");
    assert!(v["next"].as_array().is_none_or(Vec::is_empty), "{v}");
    let record = fake
        .comments_of(11)
        .into_iter()
        .find(|c| c.body.contains(r#""bosun":"hold""#));
    let body = record.expect("a hold record").body;
    assert!(
        body.contains(r#""id":"X1""#) && body.contains(r#""on":"phone""#),
        "{body}"
    );
    assert!(body.contains(r#""session":"session_a""#));

    let lists = checklists(&fake);
    assert_eq!(lists.len(), 1, "the checklist issue was created");
    let (number, state, text) = &lists[0];
    assert_eq!(
        (d["checklist"]["number"].as_u64(), state.as_str()),
        (Some(*number), "open")
    );
    assert!(
        text.starts_with("<!-- bosun:prereqs plans/x.md -->"),
        "{text}"
    );
    let sum = bosun::core::prereqs::hold_sum(STEP);
    assert!(
        text.contains("### Found during the work\n- [ ] **X1** `Create the npm token"),
        "{text}"
    );
    let marker = format!("<!-- bosun:prereq X1 sum={sum} state=owner task=11 -->");
    assert!(
        text.contains(&marker) && text.contains("_(task #11, phone)_"),
        "{text}"
    );
    assert_eq!(labels(&fake, 11), ["bosun:state/ready"]);
    let ended = fake
        .comments_of(11)
        .iter()
        .any(|c| c.body.contains("lease-end"));
    assert!(ended, "the lease ended");
    assert!(
        tmp.path().join("docs/lessons.toml").is_file(),
        "a lesson was recorded"
    );
}

#[test]
fn a_second_hold_takes_the_next_id_and_a_closed_checklist_is_reopened() {
    let (tmp, fake) = (tempfile::tempdir().unwrap(), setup());
    claimed(&fake, tmp.path(), "11");
    assert_eq!(bosun(&fake, tmp.path(), &["hold", "11", STEP]).0, 0);
    {
        let mut s = fake.state.lock().unwrap();
        let i = s
            .issues
            .iter_mut()
            .find(|i| i.labels.contains(&"bosun:prereqs".to_string()));
        i.unwrap().state = "closed".into();
    }
    claimed(&fake, tmp.path(), "12");
    let (code, v) = bosun(&fake, tmp.path(), &["hold", "12", "Enable the environment"]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["id"], "X2");
    let lists = checklists(&fake);
    assert_eq!(lists.len(), 1, "one issue per plan");
    assert_eq!(lists[0].1, "open", "reopened");
    assert!(
        lists[0].2.contains("**X1**") && lists[0].2.contains("**X2**"),
        "{}",
        lists[0].2
    );
    assert_eq!(lists[0].2.matches("### Found during the work").count(), 1);
}

#[test]
fn the_kill_switch_stops_it_and_nothing_is_written() {
    let (tmp, fake) = (tempfile::tempdir().unwrap(), setup());
    claimed(&fake, tmp.path(), "11");
    fake.variable("BOSUN_HALT", "1");
    let before = fake.comments_of(11).len();
    let (code, v) = bosun(&fake, tmp.path(), &["hold", "11", STEP]);
    assert_eq!(code, 4, "{v}");
    assert_eq!(fake.comments_of(11).len(), before);
    assert!(checklists(&fake).is_empty());
    assert_eq!(labels(&fake, 11), ["bosun:state/claimed"]);
}

#[test]
fn bad_text_is_a_usage_error() {
    let (tmp, fake) = (tempfile::tempdir().unwrap(), setup());
    claimed(&fake, tmp.path(), "11");
    let long = "x".repeat(301);
    let bad = [
        "",
        "  ",
        "see <!-- bosun:prereq H1 -->",
        "a --> b",
        "zero\u{200b}width",
        "two\nlines",
        long.as_str(),
    ];
    for text in bad {
        let (code, v) = bosun(&fake, tmp.path(), &["hold", "11", text]);
        assert_eq!(code, 2, "{text:?}: {v}");
    }
    assert!(checklists(&fake).is_empty());
    assert_eq!(labels(&fake, 11), ["bosun:state/claimed"]);
}

#[test]
fn a_task_without_an_epic_cannot_be_held() {
    let (tmp, fake) = (tempfile::tempdir().unwrap(), setup());
    fake.issue(20, "Hand made", &["bosun:state/ready"], "no meta");
    let (code, v) = bosun(&fake, tmp.path(), &["hold", "20", STEP]);
    assert_eq!(code, 1, "{v}");
    assert!(
        v["error"]["message"]
            .as_str()
            .unwrap()
            .contains("only plan tasks can be held")
    );
    let body = r#"<!-- bosun:task {"item":"M1","task":"T1","touch":["a/**"]} -->"#;
    fake.issue(21, "No epic", &["bosun:state/ready"], body);
    assert_eq!(bosun(&fake, tmp.path(), &["hold", "21", STEP]).0, 1);
    assert!(checklists(&fake).is_empty());
}
