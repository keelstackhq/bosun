mod common;

use common::fake::FakeGithub;
use common::run;
use serde_json::Value;

const NOW: &str = "2026-11-06T10:00:00Z";
const URL: &str = "https://example.test/runs/9";
const LABEL: &str = "bosun:process-fix";

fn gm(fake: &FakeGithub, args: &[&str]) -> (i32, Value) {
    let mut a = vec!["gate-missed"];
    a.extend(args);
    run(fake, "session_a", NOW, &a)
}

fn tasks(fake: &FakeGithub) -> Vec<(String, String)> {
    let s = fake.state.lock().unwrap();
    s.issues
        .iter()
        .filter(|i| !i.is_pr && i.labels.iter().any(|l| l == LABEL))
        .map(|i| (i.title.clone(), i.body.clone()))
        .collect()
}

#[test]
fn run_opens_one_task_with_the_checklist_and_dedupes() {
    let fake = FakeGithub::start();
    let (code, v) = gm(&fake, &["--run", URL, "--range", "abc..def"]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["command"], "gate-missed");
    assert_eq!(v["data"]["created"], true);
    let t = tasks(&fake);
    assert_eq!(t.len(), 1);
    assert!(t[0].0.starts_with("Which gate missed this?"), "{}", t[0].0);
    for want in [
        URL,
        "abc..def",
        "Root cause",
        "tightest loop",
        "local",
        "nightly",
        "gate or test",
    ] {
        assert!(t[0].1.contains(want), "missing {want}: {}", t[0].1);
    }
    let (code, v) = gm(&fake, &["--run", URL]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["created"], false);
    assert_eq!(tasks(&fake).len(), 1);
}

#[test]
fn closed_task_still_dedupes_and_other_incident_gets_its_own() {
    let fake = FakeGithub::start();
    fake.pull(7, "Add broken", "claude/seven", "");
    fake.pull_merged(7, "deadbeef");
    let (_, a) = gm(&fake, &["--pr", "7"]);
    let n = a["data"]["issue"].as_u64().unwrap();
    fake.state
        .lock()
        .unwrap()
        .issues
        .iter_mut()
        .find(|i| i.number == n)
        .unwrap()
        .state = "closed".into();
    let (_, b) = gm(&fake, &["--pr", "7"]);
    assert_eq!(b["data"]["issue"], n);
    assert_eq!(b["data"]["created"], false);
    let (_, c) = gm(&fake, &["--run", URL]);
    assert_eq!(c["data"]["created"], true);
    assert_eq!(tasks(&fake).len(), 2);
    assert!(tasks(&fake)[0].1.contains("#7"));
}

#[test]
fn needs_an_incident_and_a_missing_pr_is_an_error() {
    let fake = FakeGithub::start();
    let (code, v) = gm(&fake, &[]);
    assert_eq!(code, 2, "{v}");
    let (code, v) = gm(&fake, &["--pr", "99"]);
    assert_eq!(code, 1, "{v}");
    assert!(tasks(&fake).is_empty());
}

#[test]
fn main_health_red_opens_the_task_once() {
    let fake = FakeGithub::start();
    let h = |r: &str| {
        run(
            &fake,
            "session_a",
            NOW,
            &["main-health", "--result", r, "--run-url", URL],
        )
    };
    let (code, v) = h("failure");
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["gate_missed"]["created"], true);
    let (_, v) = h("failure");
    assert_eq!(v["data"]["gate_missed"], Value::Null);
    assert_eq!(tasks(&fake).len(), 1);
    assert!(tasks(&fake)[0].1.contains(URL));
}

#[test]
fn both_flags_and_forged_markers_are_handled() {
    let fake = FakeGithub::start();
    let (code, _) = gm(&fake, &["--pr", "1", "--run", URL]);
    assert_eq!(code, 2);
    let (_, a) = gm(&fake, &["--run", "x <!-- bosun:gate-missed run:other -->"]);
    assert_eq!(a["data"]["created"], true);
    let (_, b) = gm(&fake, &["--run", "other"]);
    assert_eq!(b["data"]["created"], true);
}
