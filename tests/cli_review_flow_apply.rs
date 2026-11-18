//! M10 bot verdicts and M8 fix tasks in `bosun verdict apply`.
mod common;

use common::fake::FakeGithub;
use common::{bosun, one_object, run};
use serde_json::{Value, json};

const T0: &str = "2026-10-22T10:00:00Z";
const MAIN: &str = "refs/heads/main";
const BOT: &str = "claude[bot]";

fn setup(config: &str) -> FakeGithub {
    let fake = FakeGithub::start();
    fake.issue(7, "task", &["bosun:state/review"], "Do it.");
    fake.pull_as_issue(42, "the pr");
    fake.pull(42, "the pr", "claude/x", "Closes #7");
    fake.content(".bosun/bosun.toml", MAIN, config);
    fake.state
        .lock()
        .unwrap()
        .permissions
        .insert(BOT.into(), "none".into());
    fake
}

fn verdict(sha: &str, v: &str, sev: &[&str]) -> String {
    let findings: Vec<Value> = sev
        .iter()
        .map(|s| json!({"severity": s, "file": "src/a.rs", "line": 3, "claim": "bad thing", "evidence": "e"}))
        .collect();
    json!({"pr": 42, "head_sha": sha, "verdict": v, "confidence": 0.9, "findings": findings})
        .to_string()
}

/// Post a verdict comment and return its id; `user` becomes its author.
fn comment(fake: &FakeGithub, input: &str, user: &str) -> u64 {
    let out = bosun()
        .env("BOSUN_GITHUB_API", fake.url())
        .env("BOSUN_SESSION", "session_r")
        .args(["verdict", "comment", "--from", "-"])
        .write_stdin(input.to_string())
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(0), "{}", one_object(&out.stdout));
    fake.last_comment(|c| c.user = user.into());
    fake.comments_of(42).last().unwrap().id
}

fn apply(fake: &FakeGithub, id: u64) -> (i32, Value) {
    run(
        fake,
        "session_ci",
        T0,
        &["verdict", "apply", &id.to_string()],
    )
}

const BOTS: &str = "[approvals]\nverdict_bots = [\"claude[bot]\"]\n";

#[test]
fn a_bot_without_access_is_refused_neutrally() {
    let fake = setup("");
    let id = comment(&fake, &verdict("sha42", "approve", &[]), BOT);
    let (code, v) = apply(&fake, id);
    assert_eq!(code, 0, "a refused author is no failure: {v}");
    assert!(
        v["data"]["refused"]
            .as_str()
            .unwrap()
            .contains("none access"),
        "{v}"
    );
    assert!(fake.statuses_of("sha42").is_empty());
}

#[test]
fn a_listed_bot_verdict_with_the_reviewer_marker_counts() {
    let fake = setup(BOTS);
    let id = comment(&fake, &verdict("sha42", "approve", &[]), BOT);
    let (code, v) = apply(&fake, id);
    assert_eq!(code, 0, "{v}");
    assert_eq!(fake.statuses_of("sha42")[0]["state"], "success");
}

#[test]
fn a_listed_bot_verdict_without_the_marker_or_for_another_head_does_not_count() {
    let fake = setup(BOTS);
    let id = comment(&fake, &verdict("sha42", "approve", &[]), BOT);
    fake.last_comment(|c| c.body = c.body.replace("<!-- bosun:reviewer session_r -->", ""));
    let (code, v) = apply(&fake, id);
    assert_eq!(code, 0, "{v}");
    assert!(
        v["data"]["refused"]
            .as_str()
            .unwrap()
            .contains("reviewer marker"),
        "{v}"
    );
    let id = comment(&fake, &verdict("sha42", "approve", &[]), BOT);
    fake.pull_set(42, "head", json!({"sha": "sha43", "ref": "claude/x"}));
    let (code, v) = apply(&fake, id);
    assert_eq!(code, 0, "a bot verdict for an old head is neutral: {v}");
    assert!(
        v["data"]["refused"]
            .as_str()
            .unwrap()
            .contains("not the head"),
        "{v}"
    );
    assert!(fake.statuses_of("sha42").is_empty());
}

#[test]
fn bots_listed_only_on_the_pr_head_or_base_do_not_count() {
    let fake = setup("");
    fake.content(".bosun/bosun.toml", "sha42", BOTS);
    fake.content(".bosun/bosun.toml", "base0000", BOTS);
    let id = comment(&fake, &verdict("sha42", "approve", &[]), BOT);
    let (code, v) = apply(&fake, id);
    assert_eq!(code, 0, "{v}");
    assert!(v["data"]["refused"].is_string(), "{v}");
}

#[test]
fn autonomous_request_changes_opens_then_updates_one_fix_task() {
    let fake = setup("[mode]\nname = \"autonomous\"\n");
    let id = comment(
        &fake,
        &verdict("sha42", "request-changes", &["major"]),
        "octocat",
    );
    let (code, v) = apply(&fake, id);
    assert_eq!(code, 0, "{v}");
    let n = v["data"]["fix_task"].as_u64().expect("a fix task");
    let i = fake.issue_state(n);
    assert!(
        i.body.contains("#42") && i.body.contains("bad thing"),
        "{}",
        i.body
    );
    assert!(i.labels.contains(&"bosun:fix-task".to_string()));
    let id = comment(
        &fake,
        &verdict("sha42", "request-changes", &["blocker"]),
        "octocat",
    );
    let (code, v) = apply(&fake, id);
    assert_eq!(code, 0, "{v}");
    assert_eq!(
        v["data"]["fix_task"].as_u64(),
        Some(n),
        "the same issue, updated"
    );
    assert!(fake.issue_state(n).body.contains("[blocker]"));
}

#[test]
fn no_fix_task_outside_autonomous_mode_or_on_approval() {
    let fake = setup("");
    let id = comment(
        &fake,
        &verdict("sha42", "request-changes", &["major"]),
        "octocat",
    );
    let (_, v) = apply(&fake, id);
    assert!(v["data"].get("fix_task").is_none(), "{v}");
    let fake = setup("[mode]\nname = \"autonomous\"\n");
    let id = comment(&fake, &verdict("sha42", "approve", &[]), "octocat");
    let (_, v) = apply(&fake, id);
    assert!(v["data"].get("fix_task").is_none(), "{v}");
}

#[test]
fn all_verdicts_of_one_bot_are_one_session() {
    let fake = setup(BOTS);
    let id = comment(&fake, &verdict("sha42", "approve", &[]), BOT);
    assert_eq!(apply(&fake, id).0, 0);
    let rec = fake.comments_of(42).last().unwrap().body.clone();
    assert!(rec.contains(r#""session":"bot:claude[bot]""#), "{rec}");
}

#[test]
fn a_bot_verdict_on_a_pr_into_another_branch_is_refused() {
    // A decoy PR into a branch: its verdict must not be green on a head later merged to main.
    let fake = setup(BOTS);
    fake.pull_set(42, "base", json!({"sha": "base0000", "ref": "decoy"}));
    let id = comment(&fake, &verdict("sha42", "approve", &[]), BOT);
    let (code, v) = apply(&fake, id);
    assert_eq!(code, 0, "{v}");
    assert!(
        v["data"]["refused"]
            .as_str()
            .unwrap()
            .contains("default branch"),
        "{v}"
    );
    assert!(fake.statuses_of("sha42").is_empty());
}

#[test]
fn a_listed_login_must_be_a_bot_account() {
    let fake = setup(BOTS);
    let id = comment(&fake, &verdict("sha42", "approve", &[]), BOT);
    fake.last_comment(|c| c.user_type = Some("User".into()));
    let (code, v) = apply(&fake, id);
    assert_eq!(code, 0, "{v}");
    assert!(v["data"]["refused"].is_string(), "{v}");
    assert!(fake.statuses_of("sha42").is_empty());
}

#[test]
fn a_verdict_bots_entry_that_is_no_bot_login_is_a_config_error() {
    let fake = setup("[approvals]\nverdict_bots = [\"claude\"]\n");
    let id = comment(&fake, &verdict("sha42", "approve", &[]), BOT);
    let (code, v) = apply(&fake, id);
    // An unreadable config lists no bots: refused, fail closed.
    assert_eq!(code, 0, "{v}");
    assert!(v["data"]["refused"].is_string(), "{v}");
    let d = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(d.path().join(".bosun")).unwrap();
    std::fs::write(
        d.path().join(".bosun/bosun.toml"),
        "[approvals]\nverdict_bots = [\"claude\"]\n",
    )
    .unwrap();
    let (code, v) = run(
        &fake,
        "s",
        T0,
        &["-C", d.path().to_str().unwrap(), "verdict", "apply", "1"],
    );
    assert_ne!(code, 0);
    assert!(
        v["error"]["message"]
            .as_str()
            .unwrap()
            .contains("not a bot login"),
        "{v}"
    );
}

#[test]
fn a_newer_comment_of_the_same_bot_replaces_an_older_one_whatever_the_marker() {
    let fake = setup(BOTS);
    let a = comment(&fake, &verdict("sha42", "approve", &[]), BOT);
    let b = comment(&fake, &verdict("sha42", "request-changes", &["major"]), BOT);
    fake.last_comment(|c| c.body = c.body.replace("session_r", "session_other"));
    let (code, v) = apply(&fake, a);
    assert_eq!(code, 1, "{v}");
    assert!(
        v["error"]["message"].as_str().unwrap().contains("newer"),
        "{v}"
    );
    assert_eq!(apply(&fake, b).0, 0);
    assert_eq!(fake.statuses_of("sha42")[0]["state"], "failure");
}
