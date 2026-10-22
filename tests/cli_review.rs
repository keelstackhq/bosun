mod common;

use common::fake::FakeGithub;
use common::{bosun, one_object, run};
use serde_json::{Value, json};

const T0: &str = "2026-10-22T10:00:00Z";

/// PR 42 (head `sha42`) closing task issue 7 in `review`.
fn setup() -> FakeGithub {
    let fake = FakeGithub::start();
    fake.issue(7, "task", &["bosun:state/review"], "Do it.");
    fake.pull_as_issue(42, "the pr");
    fake.pull(42, "the pr", "claude/x", "Closes #7");
    fake
}

fn verdict(sha: &str, v: &str, conf: f64, sev: &[&str], escalate: bool) -> String {
    let findings: Vec<Value> = sev
        .iter()
        .map(|s| json!({"severity": s, "file": "a.rs", "line": 3, "claim": "c", "evidence": "e"}))
        .collect();
    json!({"pr": 42, "head_sha": sha, "verdict": v, "confidence": conf,
        "findings": findings, "lessons_hit": [], "escalate_risk": escalate})
    .to_string()
}

fn post(fake: &FakeGithub, input: &str) -> (i32, Value) {
    let out = bosun()
        .env("BOSUN_GITHUB_API", fake.url())
        .args(["verdict", "post"])
        .write_stdin(input.to_string())
        .output()
        .unwrap();
    (out.status.code().unwrap(), one_object(&out.stdout))
}

#[test]
fn approve_at_threshold_sets_success() {
    let fake = setup();
    let (code, v) = post(&fake, &verdict("sha42", "approve", 0.85, &["major"], false));
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["approved"], true);
    let st = fake.statuses_of("sha42");
    assert_eq!(st[0]["context"], "review-verdict");
    assert_eq!(st[0]["state"], "success");
    assert_eq!(st[0]["description"], "approve 0.85, 0 blocker, 1 major");
    // Task 15 queues the task, not the verdict
    assert!(fake.labels_of(7).contains(&"bosun:state/review".into()));
    let rec = fake.comments_of(42);
    assert!(rec[0].body.contains(r#""bosun":"verdict""#));
    assert!(rec[0].body.contains(r#""head_sha":"sha42""#));
}

#[test]
fn below_threshold_does_not_approve() {
    let fake = setup();
    let (code, v) = post(&fake, &verdict("sha42", "approve", 0.84, &[], false));
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["approved"], false);
    assert_eq!(fake.statuses_of("sha42")[0]["state"], "failure");
    assert!(fake.labels_of(7).contains(&"bosun:state/fixing".into()));
}

#[test]
fn a_blocker_blocks_and_request_changes_moves_to_fixing() {
    let fake = setup();
    let (_, v) = post(
        &fake,
        &verdict("sha42", "approve", 0.99, &["blocker"], false),
    );
    assert_eq!(v["data"]["approved"], false);
    assert!(
        v["data"]["reasons"][0]
            .as_str()
            .unwrap()
            .contains("blocker")
    );
    assert_eq!(fake.statuses_of("sha42")[0]["state"], "failure");
    assert!(fake.labels_of(7).contains(&"bosun:state/fixing".into()));
}

#[test]
fn stale_sha_is_exit_3() {
    let fake = setup();
    let (code, v) = post(&fake, &verdict("old", "approve", 0.9, &[], false));
    assert_eq!(code, 3);
    assert!(
        v["error"]["message"]
            .as_str()
            .unwrap()
            .contains("stale verdict for old")
    );
    assert!(v["error"]["fix"].as_str().unwrap().contains("review-pack"));
    assert!(fake.statuses_of("sha42").is_empty());
}

#[test]
fn escalate_risk_needs_human() {
    let fake = setup();
    let (code, _) = post(&fake, &verdict("sha42", "approve", 0.95, &[], true));
    assert_eq!(code, 0);
    assert!(
        fake.labels_of(7)
            .contains(&"bosun:state/needs-human".into())
    );
}

#[test]
fn an_escalated_approval_is_not_green() {
    let fake = setup();
    let (code, v) = post(&fake, &verdict("sha42", "approve", 0.95, &[], true));
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["approved"], false);
    let st = fake.statuses_of("sha42");
    assert_eq!(st[0]["state"], "failure");
    let d = st[0]["description"].as_str().unwrap();
    assert!(d.contains("escalated to the owner"), "{d}");
}

#[test]
fn invalid_json_is_usage_and_names_the_field() {
    let fake = setup();
    let (code, v) = post(&fake, r#"{"pr":42,"head_sha":"sha42","verdict":"approve"}"#);
    assert_eq!(code, 2);
    assert!(
        v["error"]["message"]
            .as_str()
            .unwrap()
            .contains("confidence")
    );
}

#[test]
fn halted_by_the_kill_switch() {
    let fake = setup();
    fake.variable("BOSUN_HALT", "1");
    let (code, _) = run(
        &fake,
        "s",
        T0,
        &["verdict", "post", "--from", "/nonexistent"],
    );
    assert_eq!(code, 4);
}

const LESSONS: &str = r#"
[[lesson]]
id = "L-001"
rule = "global"

[[lesson]]
id = "L-002"
rule = "core only"
scope = ["src/core/**"]

[[lesson]]
id = "L-003"
rule = "web only"
scope = ["web/**"]
"#;

fn pack_setup() -> FakeGithub {
    let fake = FakeGithub::start();
    fake.issue(
        3,
        "epic",
        &[],
        "Epic body\n- UC-M1-1: leases expire\n- UC-M1-2: x\nother",
    );
    fake.issue(
        7,
        "the task",
        &["bosun:state/review"],
        "Do it.\n<!-- bosun:task {\"item\":\"M1\",\"task\":\"T1\",\"epic\":3,\"touch\":[\"src/**\"]} -->",
    );
    fake.pull_as_issue(42, "the pr");
    fake.pull(42, "the pr", "claude/x", "Closes #7");
    fake.pull_file(42, "src/core/a.rs", "modified", 2, 1, Some("@@ a"));
    fake.pull_file(42, "src/b.rs", "added", 5, 0, Some("@@ b"));
    fake.content("docs/lessons.toml", "base0000", LESSONS);
    fake.content("REVIEW.md", "base0000", "Review rules");
    fake.content("REVIEW.md", "sha42", "head version, not used");
    fake
}

#[test]
fn review_pack_lists_matching_lessons_and_the_base_review_md() {
    let fake = pack_setup();
    let (code, v) = run(&fake, "s", T0, &["review-pack", "42"]);
    assert_eq!(code, 0, "{v}");
    let d = &v["data"];
    assert_eq!(d["pr"], 42);
    assert_eq!(d["head_sha"], "sha42");
    assert_eq!(d["base_sha"], "base0000");
    assert!(d["since_sha"].is_null());
    assert_eq!(d["files"].as_array().unwrap().len(), 2);
    assert_eq!(d["files"][0]["patch"], "@@ a");
    assert_eq!(d["task"]["issue"], 7);
    assert_eq!(d["task"]["meta"]["item"], "M1");
    assert_eq!(
        d["ucs"],
        json!(["- UC-M1-1: leases expire", "- UC-M1-2: x"])
    );
    let ids: Vec<&str> = d["lessons"]
        .as_array()
        .unwrap()
        .iter()
        .map(|l| l["id"].as_str().unwrap())
        .collect();
    assert_eq!(ids, ["L-001", "L-002"]);
    assert_eq!(d["review_md"], "Review rules");
    assert!(
        d["instructions"]
            .as_str()
            .unwrap()
            .contains("bosun verdict post")
    );
}

#[test]
fn review_pack_tolerates_missing_files() {
    let fake = FakeGithub::start();
    fake.pull(5, "p", "b", "no link");
    let (code, v) = run(&fake, "s", T0, &["review-pack", "5"]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["lessons"], json!([]));
    assert!(v["data"]["review_md"].is_null());
    assert!(v["data"]["task"].is_null());
}

#[test]
fn second_pack_after_a_verdict_is_a_delta() {
    let fake = pack_setup();
    let (code, _) = post(
        &fake,
        &verdict("sha42", "request-changes", 0.9, &["major"], false),
    );
    assert_eq!(code, 0);
    // the author pushes a new commit touching one file
    fake.pull_set(
        42,
        "head",
        json!({"sha": "sha42b", "ref": "claude/x", "label": "acme:claude/x"}),
    );
    fake.compare_file("sha42", "sha42b", "src/b.rs", "@@ fix");
    let (code, v) = run(&fake, "s", T0, &["review-pack", "42"]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["since_sha"], "sha42");
    assert_eq!(v["data"]["head_sha"], "sha42b");
    let files = v["data"]["files"].as_array().unwrap();
    assert_eq!(files.len(), 1);
    assert_eq!(files[0]["filename"], "src/b.rs");
}
