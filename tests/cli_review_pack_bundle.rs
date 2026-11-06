//! The review pack lists every task a bundle PR closes and no longer flags their paths.

mod common;

use common::fake::FakeGithub;
use common::{lease_note, run};
use serde_json::json;

const T0: &str = "2026-10-22T10:00:00Z";

fn meta(task: &str, touch: &str) -> String {
    format!(
        "Do it.\n<!-- bosun:task {{\"item\":\"M1\",\"task\":\"{task}\",\"touch\":[\"{touch}\"]}} -->"
    )
}

fn setup(body: &str) -> FakeGithub {
    let fake = FakeGithub::start();
    fake.issue(7, "lead", &["bosun:state/review"], &meta("T1", "src/a.rs"));
    fake.issue(8, "second", &["bosun:state/ready"], &meta("T2", "src/b.rs"));
    fake.issue(9, "plain", &[], "no metadata");
    fake.pull_as_issue(42, "the pr");
    fake.pull(42, "the pr", "claude/x", body);
    fake.pull_file(42, "src/b.rs", "added", 5, 0, Some("@@ b"));
    fake
}

#[test]
fn bundle_tasks_lists_the_other_closing_tasks() {
    let fake = setup("Closes #7\nCloses #8\nCloses #9\nCloses other/repo#10");
    let (code, v) = run(&fake, "s", T0, &["review-pack", "42"]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["task"]["issue"], 7);
    let bt = v["data"]["bundle_tasks"].as_array().unwrap();
    assert_eq!(bt.len(), 1, "{v}");
    assert_eq!(bt[0]["issue"], 8);
    assert_eq!(bt[0]["title"], "second");
    assert_eq!(bt[0]["meta"]["task"], "T2");
    assert!(bt[0]["body"].as_str().unwrap().contains("Do it."));
}

#[test]
fn touch_added_is_computed_against_the_union_of_the_tasks() {
    let fake = setup("Closes #7\nCloses #8");
    fake.comment(7, &lease_note("s/T1", "2026-10-22T10:20:00Z", "src/a.rs"));
    fake.comment(
        7,
        r#"Claimed by s/T1.

{"bosun":"lease","session":"s/T1","origin":"local","expires":"2026-10-22T10:30:00Z","touch":["src/a.rs","src/b.rs","src/c.rs"]}"#,
    );
    fake.comment(7, r#"{"bosun":"task-pr","session":"s/T1","pr":42}"#);
    let (code, v) = run(&fake, "s", T0, &["review-pack", "42"]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["task"]["touch_added"], json!(["src/c.rs"]));
    let ins = v["data"]["instructions"].as_str().unwrap();
    assert!(
        ins.contains("bundle_tasks") && !ins.contains("by design"),
        "{ins}"
    );
}

#[test]
fn a_single_task_pr_has_no_bundle_tasks() {
    let fake = setup("Closes #7");
    let (_, v) = run(&fake, "s", T0, &["review-pack", "42"]);
    assert_eq!(v["data"]["bundle_tasks"], json!([]));
}

#[test]
fn only_tasks_of_the_same_epic_and_repo_are_bundle_tasks() {
    let fake = setup("Closes #7\nCloses #8\nCloses #11\nCloses other/repo#12");
    let epic = |t: &str, e: u64| {
        format!("x\n<!-- bosun:task {{\"item\":\"M1\",\"task\":\"{t}\",\"epic\":{e}}} -->")
    };
    fake.issue(11, "foreign epic", &[], &epic("T3", 99));
    fake.issue(12, "other repo", &[], &meta("T4", "src/b.rs"));
    let (code, v) = run(&fake, "s", T0, &["review-pack", "42"]);
    assert_eq!(code, 0, "{v}");
    let got: Vec<u64> = (v["data"]["bundle_tasks"].as_array().unwrap().iter())
        .map(|t| t["issue"].as_u64().unwrap())
        .collect();
    assert_eq!(got, vec![8], "{v}");
}
