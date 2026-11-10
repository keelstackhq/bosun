//! `bosun approve <pr>@<head> --chat` (plan 2026-11-10 T5, spec §8): an orchestrator records the
//! owner's chat answer as one PR comment, only with the stored answer, the live head and a clean
//! verdict from another session.
mod common;

use std::path::Path;
use std::process::Command;

use common::fake::FakeGithub;
use common::{bosun, lease_note, one_object};
use serde_json::{Value, json};

const NOW: &str = "2026-11-10T10:00:00Z";
const LIVE: &str = "2026-11-10T11:00:00Z";
const HEAD: &str = "0123456789abcdef0123456789abcdef01234567";
const OTHER: &str = "89abcdef0123456789abcdef0123456789abcdef";
const Q: &str = "1-0123456-1762768800";
const REG: u64 = 50;
const BOT: &str = "github-actions[bot]";
const OWNERS: &str = "[[owner]]\ngithub = \"ada\"\nslack = \"U0ADA1\"\n";

fn git(dir: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .output()
        .unwrap();
    assert!(out.status.success(), "git {args:?}: {out:?}");
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

fn repo() -> tempfile::TempDir {
    let tmp = tempfile::tempdir().unwrap();
    git(tmp.path(), &["init", "-q", "-b", "main"]);
    tmp
}

/// Store an answer as the PostToolUse hook does.
fn answer(dir: &Path, id: &str, answer: &str, pr: u64, head: &str) {
    let file = dir
        .join(git(dir, &["rev-parse", "--git-common-dir"]))
        .join("bosun/answers.json");
    std::fs::create_dir_all(file.parent().unwrap()).unwrap();
    let v = json!({id: {"answer": answer, "pr": pr, "head": head, "at": NOW}});
    std::fs::write(file, v.to_string()).unwrap();
}

fn join(fake: &FakeGithub, session: &str, origin: &str, parent: Option<&str>, depth: u32) {
    let parent = parent.map_or(String::new(), |p| format!(",\"parent\":\"{p}\""));
    fake.comment(
        REG,
        &format!(
            "{{\"bosun\":\"join\",\"session\":\"{session}\",\"origin\":\"{origin}\"{parent},\"depth\":{depth},\"expires\":\"{LIVE}\"}}"
        ),
    );
}

fn verdict(session: &str, head: &str) -> String {
    let r = json!({"bosun": "verdict", "session": session, "head_sha": head,
        "approved": true, "confidence": 0.95, "major": 0});
    format!("Review verdict: approve 0.95, 0 blocker.\n\n{r}")
}

/// PR 1 (Closes #7, leased by `session_w`) at HEAD, root `session_orch` in the registry, a clean
/// verdict by `session_rev`.
fn setup() -> FakeGithub {
    let fake = FakeGithub::start();
    fake.pull(1, "change", "feature", "Closes #7");
    fake.pull_as_issue(1, "change");
    fake.pull_set(1, "head", json!({"sha": HEAD, "ref": "feature"}));
    fake.issue(7, "task", &[], "task\n<!-- bosun:task {} -->");
    fake.comment(7, &lease_note("session_w", LIVE, "src/**"));
    fake.content("governance/owners.toml", "main", OWNERS);
    fake.issue(REG, "bosun registry", &["bosun:registry"], "");
    join(&fake, "session_orch", "spawned", None, 1);
    fake.comment_by(1, BOT, &verdict("session_rev", HEAD));
    fake
}

fn chat(
    fake: &FakeGithub,
    dir: &Path,
    session: &str,
    args: &[&str],
    env: &[(&str, &str)],
) -> (i32, Value) {
    let mut cmd = bosun();
    cmd.current_dir(dir)
        .env("BOSUN_GITHUB_API", fake.url())
        .env("BOSUN_SESSION", session)
        .env("BOSUN_NOW", NOW)
        .args(["approve", &format!("1@{HEAD}"), "--chat"])
        .args(args);
    for (k, v) in env {
        cmd.env(k, v);
    }
    let out = cmd.output().unwrap();
    let v = one_object(&out.stdout);
    if v["ok"] == false {
        assert!(!v["error"]["fix"].as_str().unwrap().trim().is_empty());
    }
    (out.status.code().unwrap(), v)
}

fn chat_comments(fake: &FakeGithub) -> Vec<String> {
    (fake.comments_of(1).into_iter())
        .map(|c| c.body)
        .filter(|b| b.contains("bosun:chat-approval"))
        .collect()
}

fn refused(fake: &FakeGithub, (code, v): (i32, Value), want: i32) -> Value {
    assert_eq!(code, want, "{v}");
    assert!(chat_comments(fake).is_empty(), "a comment was posted");
    v
}

#[test]
fn chat_records_comment() {
    let (fake, tmp) = (setup(), repo());
    answer(tmp.path(), Q, "approve", 1, HEAD);
    let (code, v) = chat(&fake, tmp.path(), "session_orch", &["--question", Q], &[]);
    assert_eq!(code, 0, "{v}");
    let body = bosun::core::approve::chat_comment(HEAD, "session_orch", NOW, Some(Q));
    assert_eq!(chat_comments(&fake), vec![body]);
    assert_eq!(v["data"]["pr"], 1);
    assert_eq!(v["data"]["head_sha"], HEAD);
    assert_eq!(v["data"]["question_id"], Q);
    assert!(v["data"]["comment_id"].as_u64().is_some(), "{v}");
}

#[test]
fn chat_works_in_cloud() {
    let (fake, tmp) = (setup(), repo());
    answer(tmp.path(), Q, "approve", 1, HEAD);
    let env = [
        ("CLAUDE_CODE_REMOTE", "true"),
        ("CCR_AGENT_PROXY_ENABLED", "true"),
    ];
    let (code, v) = chat(&fake, tmp.path(), "session_orch", &["--question", Q], &env);
    assert_eq!(code, 0, "{v}");
    assert_eq!(chat_comments(&fake).len(), 1);
}

#[test]
fn chat_child_refused() {
    let (fake, tmp) = (setup(), repo());
    join(&fake, "session_kid", "spawned", Some("session_orch"), 2);
    answer(tmp.path(), Q, "approve", 1, HEAD);
    let env = [("BOSUN_ORIGIN", "spawned")];
    let r = chat(&fake, tmp.path(), "session_kid", &["--question", Q], &env);
    let v = refused(&fake, r, 5);
    assert!(
        v["error"]["fix"].as_str().unwrap().contains("handoff-pr"),
        "{v}"
    );
}

#[test]
fn chat_disabled_refused() {
    let (fake, tmp) = (setup(), repo());
    fake.content(
        ".bosun/bosun.toml",
        "base0000",
        "project = \"widget\"\n\n[approvals]\nchat = false\n",
    );
    answer(tmp.path(), Q, "approve", 1, HEAD);
    let r = chat(&fake, tmp.path(), "session_orch", &["--question", Q], &[]);
    refused(&fake, r, 5);
}

#[test]
fn chat_head_moved_refused() {
    let (fake, tmp) = (setup(), repo());
    fake.pull_set(1, "head", json!({"sha": OTHER, "ref": "feature"}));
    answer(tmp.path(), Q, "approve", 1, HEAD);
    let r = chat(&fake, tmp.path(), "session_orch", &["--question", Q], &[]);
    refused(&fake, r, 3);
}

#[test]
fn chat_without_clean_verdict_refused() {
    let tmp = repo();
    answer(tmp.path(), Q, "approve", 1, HEAD);
    // Only the orchestrator's own verdict, and the author's (lease holder): neither counts.
    let fake = setup();
    fake.last_comment(|c| c.body = verdict("session_orch", HEAD));
    fake.comment_by(1, BOT, &verdict("session_w", HEAD));
    let r = chat(&fake, tmp.path(), "session_orch", &["--question", Q], &[]);
    refused(&fake, r, 5);
    // A clean verdict on an older head does not count either.
    let fake = setup();
    fake.last_comment(|c| c.body = verdict("session_rev", OTHER));
    let r = chat(&fake, tmp.path(), "session_orch", &["--question", Q], &[]);
    refused(&fake, r, 5);
}

#[test]
fn chat_without_stored_answer_refused() {
    let (fake, tmp) = (setup(), repo());
    let r = chat(&fake, tmp.path(), "session_orch", &["--question", Q], &[]);
    let v = refused(&fake, r, 5);
    assert!(v["error"]["fix"].as_str().unwrap().contains("ask"), "{v}");
    // An answer for another head or another PR under this id does not count.
    answer(tmp.path(), Q, "approve", 1, OTHER);
    let r = chat(&fake, tmp.path(), "session_orch", &["--question", Q], &[]);
    refused(&fake, r, 5);
    answer(tmp.path(), Q, "approve", 2, HEAD);
    let r = chat(&fake, tmp.path(), "session_orch", &["--question", Q], &[]);
    refused(&fake, r, 5);
    // An id naming another PR, even with a matching stored answer.
    let q2 = "2-0123456-1762768800";
    answer(tmp.path(), q2, "approve", 1, HEAD);
    let r = chat(&fake, tmp.path(), "session_orch", &["--question", q2], &[]);
    refused(&fake, r, 5);
    // No --question at all.
    let (code, v) = chat(&fake, tmp.path(), "session_orch", &[], &[]);
    assert_eq!(code, 2, "{v}");
}

#[test]
fn chat_hold_answer_refused() {
    let (fake, tmp) = (setup(), repo());
    answer(tmp.path(), Q, "hold", 1, HEAD);
    let r = chat(&fake, tmp.path(), "session_orch", &["--question", Q], &[]);
    refused(&fake, r, 5);
}

#[test]
fn chat_slack_owner_ok() {
    let (fake, tmp) = (setup(), repo());
    join(&fake, "session_tag", "slack", None, 2);
    let env = [("BOSUN_ORIGIN", "slack")];
    let (code, v) = chat(
        &fake,
        tmp.path(),
        "session_tag",
        &["--slack-user", "U0ADA1"],
        &env,
    );
    assert_eq!(code, 0, "{v}");
    let body = bosun::core::approve::chat_comment(HEAD, "session_tag", NOW, None);
    assert_eq!(chat_comments(&fake), vec![body]);
}

#[test]
fn chat_slack_non_owner_refused() {
    let (fake, tmp) = (setup(), repo());
    join(&fake, "session_tag", "slack", None, 2);
    let env = [("BOSUN_ORIGIN", "slack")];
    let r = chat(
        &fake,
        tmp.path(),
        "session_tag",
        &["--slack-user", "U0EVE9"],
        &env,
    );
    refused(&fake, r, 5);
}

#[test]
fn chat_slack_user_needs_slack_origin() {
    // An owner's Slack ID is public in owners.toml: a non-Slack orchestrator cannot use it to
    // skip the stored answer.
    let (fake, tmp) = (setup(), repo());
    let local = [("BOSUN_ORIGIN", "local")];
    let r = chat(
        &fake,
        tmp.path(),
        "session_orch",
        &["--slack-user", "U0ADA1"],
        &local,
    );
    refused(&fake, r, 5);
}

#[test]
fn chat_slack_user_needs_slack_join() {
    // BOSUN_ORIGIN=slack alone does not do: session_orch joined the registry as spawned.
    let (fake, tmp) = (setup(), repo());
    let slack = [("BOSUN_ORIGIN", "slack")];
    let r = chat(
        &fake,
        tmp.path(),
        "session_orch",
        &["--slack-user", "U0ADA1"],
        &slack,
    );
    refused(&fake, r, 5);
}

#[test]
fn chat_later_hold_wins() {
    let (fake, tmp) = (setup(), repo());
    let file = tmp
        .path()
        .join(git(tmp.path(), &["rev-parse", "--git-common-dir"]))
        .join("bosun/answers.json");
    std::fs::create_dir_all(file.parent().unwrap()).unwrap();
    let v = json!({
        Q: {"answer": "approve", "pr": 1, "head": HEAD, "at": NOW},
        "1-0123456-1762769000": {"answer": "hold", "pr": 1, "head": HEAD, "at": NOW},
    });
    std::fs::write(file, v.to_string()).unwrap();
    let r = chat(&fake, tmp.path(), "session_orch", &["--question", Q], &[]);
    refused(&fake, r, 5);
}
