//! `bosun review-loop <pr> [--max N]` and `--report`: rounds from verdict comments, one next step.

mod common;

use bosun::core::verdict::{Finding, Verdict, comment_body};
use common::fake::FakeGithub;
use common::run;
use serde_json::Value;

const NOW: &str = "2026-11-18T10:00:00Z";

fn verdict(head: &str, kind: &str, conf: f64, escalate: bool, pr: u64) -> Verdict {
    Verdict {
        pr,
        head_sha: head.into(),
        verdict: kind.into(),
        confidence: conf,
        findings: if kind == "approve" {
            vec![]
        } else {
            vec![Finding {
                severity: "major".into(),
                file: "src/a.rs".into(),
                line: Some(3),
                claim: "wrong".into(),
                evidence: "see line".into(),
            }]
        },
        lessons_hit: vec![],
        escalate_risk: escalate,
        chunks_reviewed: vec![],
    }
}

fn post(fake: &FakeGithub, user: &str, v: &Verdict) {
    fake.comment_by(v.pr, user, &comment_body(v, "session_rev", None));
}

fn fake() -> FakeGithub {
    let fake = FakeGithub::start();
    fake.pull_as_issue(1, "change");
    fake.pull(1, "change", "feature", "Closes #7");
    fake
}

fn lp(fake: &FakeGithub, args: &[&str]) -> (i32, Value) {
    let mut a = vec!["review-loop"];
    a.extend_from_slice(args);
    run(fake, "session_test", NOW, &a)
}

#[test]
fn no_verdict_spawns_a_reviewer_with_the_posting_text() {
    let f = fake();
    let (code, v) = lp(&f, &["1"]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["step"], "spawn-reviewer");
    assert_eq!(v["data"]["rounds"], 0);
    assert_eq!(v["data"]["max"], 5);
    let prompt = v["next"][0]["args"]["prompt"].as_str().unwrap();
    assert!(
        prompt.contains("bosun verdict comment --from -"),
        "{prompt}"
    );
    assert!(prompt.contains("scratchpad"), "{prompt}");
}

#[test]
fn clean_verdict_at_head_is_ready() {
    let f = fake();
    post(&f, "ada", &verdict("sha1", "approve", 0.95, false, 1));
    let (code, v) = lp(&f, &["1"]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["step"], "ready");
    assert_eq!(v["data"]["rounds"], 1);
}

#[test]
fn request_changes_gives_findings_to_fix() {
    let f = fake();
    post(
        &f,
        "ada",
        &verdict("sha1", "request-changes", 0.9, false, 1),
    );
    let (code, v) = lp(&f, &["1"]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["step"], "fix");
    assert_eq!(v["data"]["findings"][0]["file"], "src/a.rs");
    assert!(
        v["next"][0]["args"]["prompt"]
            .as_str()
            .unwrap()
            .contains("src/a.rs")
    );
}

#[test]
fn escalated_verdict_asks_the_owner() {
    let f = fake();
    post(&f, "ada", &verdict("sha1", "approve", 0.95, true, 1));
    let (code, v) = lp(&f, &["1"]);
    assert_eq!(code, 5, "{v}");
    assert_eq!(v["data"]["step"], "ask-owner");
}

#[test]
fn stale_verdict_spawns_a_new_round_until_the_limit() {
    let f = fake();
    post(
        &f,
        "ada",
        &verdict("old1", "request-changes", 0.9, false, 1),
    );
    post(
        &f,
        "ada",
        &verdict("old2", "request-changes", 0.9, false, 1),
    );
    let (code, v) = lp(&f, &["1", "--max", "3"]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["step"], "spawn-reviewer");
    assert_eq!(v["data"]["rounds"], 2);
    let (code, v) = lp(&f, &["1", "--max", "2"]);
    assert_eq!(code, 5, "{v}");
    assert!(v["error"]["message"].as_str().unwrap().contains("round 3"));
}

#[test]
fn bot_verdict_is_never_ready() {
    let f = fake();
    post(
        &f,
        "github-actions[bot]",
        &verdict("sha1", "approve", 0.95, false, 1),
    );
    let (code, v) = lp(&f, &["1"]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["step"], "spawn-reviewer");
    assert_eq!(v["data"]["rounds"], 0);
}

#[test]
fn report_lists_open_prs_that_need_attention() {
    let f = fake();
    f.pull_as_issue(2, "other");
    f.pull(2, "other", "feature2", "");
    f.pull_as_issue(3, "done");
    f.pull(3, "done", "feature3", "");
    post(&f, "ada", &verdict("sha3", "approve", 0.95, false, 3));
    post(&f, "ada", &verdict("old", "request-changes", 0.9, false, 2));
    let (code, v) = lp(&f, &["--report"]);
    assert_eq!(code, 0, "{v}");
    let rows = v["data"]["prs"].as_array().unwrap();
    let step = |n: u64| {
        rows.iter()
            .find(|r| r["pr"] == n)
            .map(|r| r["step"].clone())
    };
    assert_eq!(step(1), Some("spawn-reviewer".into()));
    assert_eq!(step(2), Some("spawn-reviewer".into()));
    assert_eq!(step(3), None, "ready PRs are not listed");
    assert_eq!(rows.iter().find(|r| r["pr"] == 2).unwrap()["rounds"], 1);
}

#[test]
fn newer_comment_of_the_same_session_replaces_and_outsiders_do_not_count() {
    let f = fake();
    post(
        &f,
        "ada",
        &verdict("sha1", "request-changes", 0.9, false, 1),
    );
    post(&f, "ada", &verdict("sha1", "approve", 0.95, false, 1));
    let (_, v) = lp(&f, &["1"]);
    assert_eq!(v["data"]["step"], "ready", "{v}");
    let g = fake();
    g.permission("eve", "read");
    post(&g, "eve", &verdict("sha1", "approve", 0.95, false, 1));
    let (_, v) = lp(&g, &["1"]);
    assert_eq!(v["data"]["step"], "spawn-reviewer", "{v}");
}

#[test]
fn low_confidence_approve_is_fix_with_reasons() {
    let f = fake();
    post(&f, "ada", &verdict("sha1", "approve", 0.6, false, 1));
    let (_, v) = lp(&f, &["1"]);
    assert_eq!(v["data"]["step"], "fix");
    assert!(
        v["data"]["reasons"][0]
            .as_str()
            .unwrap()
            .contains("confidence")
    );
}
