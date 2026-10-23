mod common;

use common::fake::FakeGithub;
use common::run;

const T0: &str = "2026-10-22T10:00:00Z";

fn building(fake: &FakeGithub, n: u64) {
    fake.issue(n, "task", &["bosun:state/building"], "Do it.");
}

fn fail(fake: &FakeGithub, n: u64, sig: &str, extra: &[&str]) -> (i32, serde_json::Value) {
    let mut args = vec!["fail", "--signature", sig];
    let n = n.to_string();
    args.insert(1, &n);
    args.extend_from_slice(extra);
    run(fake, "session_a", T0, &args)
}

fn escalations(fake: &FakeGithub, n: u64) -> Vec<String> {
    fake.comments_of(n)
        .into_iter()
        .filter(|c| c.body.contains(r#""bosun":"escalate""#))
        .map(|c| c.body)
        .collect()
}

#[test]
fn uc_m1_2_three_equal_failures_block_and_escalate() {
    let fake = FakeGithub::start();
    building(&fake, 5);
    for i in 1..=2 {
        let (code, v) = fail(&fake, 5, "E0308 in src/x.rs", &[]);
        assert_eq!(code, 0, "{v}");
        assert_eq!(v["data"]["stopped"], false);
        assert_eq!(v["data"]["streak"], i);
        assert!(fake.labels_of(5).contains(&"bosun:state/building".into()));
    }
    let (code, v) = fail(&fake, 5, "E0308 in src/x.rs", &[]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["stopped"], true);
    assert_eq!(v["data"]["rule"], "same-failure");
    assert_eq!(v["data"]["signature"], "E0308 in src/x.rs");
    assert_eq!(v["data"]["state"], "blocked");
    assert_eq!(v["data"]["escalate"]["from"], "sonnet");
    assert_eq!(v["data"]["escalate"]["to"], "opus");
    let labels = fake.labels_of(5);
    assert!(labels.contains(&"bosun:state/blocked".into()));
    assert!(!labels.contains(&"bosun:state/building".into()));
    let esc = escalations(&fake, 5);
    assert_eq!(esc.len(), 1);
    assert!(esc[0].contains(r#""from":"sonnet","to":"opus","reason":"same-failure""#));
}

#[test]
fn a_different_signature_resets_the_streak() {
    let fake = FakeGithub::start();
    building(&fake, 5);
    fail(&fake, 5, "a", &[]);
    fail(&fake, 5, "a", &[]);
    let (_, v) = fail(&fake, 5, "b", &[]);
    assert_eq!(v["data"]["stopped"], false);
    assert_eq!(v["data"]["streak"], 1);
    let (_, v) = fail(&fake, 5, "a", &[]);
    assert_eq!(v["data"]["stopped"], false);
    assert!(escalations(&fake, 5).is_empty());
}

#[test]
fn zero_changed_lines_stops_at_once() {
    let fake = FakeGithub::start();
    building(&fake, 5);
    let (code, v) = fail(&fake, 5, "a", &["--changed-lines", "0"]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["rule"], "empty-fix");
    assert!(fake.labels_of(5).contains(&"bosun:state/blocked".into()));
    assert_eq!(escalations(&fake, 5).len(), 1);
}

#[test]
fn opus_used_up_gives_needs_human() {
    let fake = FakeGithub::start();
    building(&fake, 5);
    fake.comment(
        5,
        r#"{"bosun":"escalate","session":"s","from":"sonnet","to":"opus","reason":"same-failure"}"#,
    );
    let (code, v) = fail(&fake, 5, "a", &["--changed-lines", "0"]);
    assert_eq!(code, 5, "{v}");
    assert_eq!(v["ok"], false);
    assert_eq!(v["data"]["state"], "needs-human");
    assert_eq!(v["data"]["rule"], "empty-fix");
    assert!(v["error"]["fix"].as_str().unwrap().contains("#5"));
    assert!(
        fake.labels_of(5)
            .contains(&"bosun:state/needs-human".into())
    );
    assert_eq!(escalations(&fake, 5).len(), 1);
}

#[test]
fn an_escalation_restarts_the_count() {
    let fake = FakeGithub::start();
    building(&fake, 5);
    fail(&fake, 5, "a", &[]);
    fail(&fake, 5, "a", &[]);
    fail(&fake, 5, "a", &[]);
    assert_eq!(escalations(&fake, 5).len(), 1);
    let (code, v) = fail(&fake, 5, "a", &[]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["stopped"], false);
    assert_eq!(v["data"]["streak"], 1);
}

#[test]
fn empty_signature_is_usage_and_missing_issue_is_error() {
    let fake = FakeGithub::start();
    building(&fake, 5);
    let (code, _) = fail(&fake, 5, "  ", &[]);
    assert_eq!(code, 2);
    let (code, _) = fail(&fake, 99, "a", &[]);
    assert_eq!(code, 1);
}

#[test]
fn a_merged_task_is_refused_and_gets_no_comment() {
    let fake = FakeGithub::start();
    fake.issue(5, "task", &["bosun:state/merged"], "Do it.");
    let (code, v) = fail(&fake, 5, "a", &[]);
    assert_eq!(code, 3, "{v}");
    assert!(fake.comments_of(5).is_empty());
}

#[test]
fn model_never_goes_below_the_newest_escalation() {
    let fake = FakeGithub::start();
    building(&fake, 5);
    fake.comment(
        5,
        r#"{"bosun":"escalate","session":"s","from":"haiku","to":"sonnet","reason":"same-failure"}"#,
    );
    let (code, v) = fail(&fake, 5, "a", &["--changed-lines", "0", "--model", "haiku"]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["escalate"]["from"], "sonnet");
    assert_eq!(v["data"]["escalate"]["to"], "opus");
}

fn created_issues(fake: &FakeGithub) -> usize {
    fake.requests()
        .iter()
        .filter(|r| r.method == "POST" && r.path.ends_with("/issues"))
        .count()
}

#[test]
fn a_stop_opens_a_lesson_and_the_same_signature_on_another_task_hits_it() {
    let fake = FakeGithub::start();
    building(&fake, 5);
    building(&fake, 6);
    for _ in 0..2 {
        fail(&fake, 5, "E0308 in src/x.rs", &[]);
    }
    let (_, v) = fail(&fake, 5, "E0308 in src/x.rs", &[]);
    assert_eq!(v["data"]["lesson"]["created"], true, "{v}");
    let lesson = v["data"]["lesson"]["issue"].as_u64().unwrap();
    assert!(fake.labels_of(lesson).contains(&"bosun:lesson".to_string()));
    assert_eq!(created_issues(&fake), 1);
    for _ in 0..2 {
        fail(&fake, 6, "E0308 in src/x.rs", &[]);
    }
    let (code, v) = fail(&fake, 6, "E0308 in src/x.rs", &[]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["lesson"]["created"], false);
    assert_eq!(v["data"]["lesson"]["issue"], lesson);
    assert_eq!(v["data"]["lesson"]["hits"], 1);
    assert_eq!(created_issues(&fake), 1);
}

#[test]
fn the_needs_human_path_also_captures_a_lesson() {
    let fake = FakeGithub::start();
    building(&fake, 5);
    fake.comment(
        5,
        r#"{"bosun":"escalate","session":"s","from":"sonnet","to":"opus","reason":"same-failure"}"#,
    );
    let (code, v) = fail(&fake, 5, "a", &["--changed-lines", "0"]);
    assert_eq!(code, 5, "{v}");
    assert_eq!(v["data"]["lesson"]["created"], true, "{v}");
    assert_eq!(created_issues(&fake), 1);
}

#[test]
fn a_failed_capture_does_not_hide_the_stop() {
    let fake = FakeGithub::start();
    building(&fake, 5);
    // The kill switch lists `/issues` once; every later list (the lesson lookup) fails.
    fake.after_requests("/issues", 1, |s| {
        s.fail_paths.push(("/issues".into(), 500));
    });
    let (code, v) = fail(&fake, 5, "a", &["--changed-lines", "0"]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["state"], "blocked");
    assert!(v["data"]["lesson"].is_null());
    assert!(!v["data"]["lesson_error"].as_str().unwrap().is_empty());
}
