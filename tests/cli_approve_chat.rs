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
    let v = json!({id: {"answer": answer, "repo": "acme/widget", "pr": pr, "head": head, "at": NOW,
        "session": "session_orch", "cli_session_id": "3f2c9a4e-1b7d-4c55-9a0e-5d6f7a8b9c0d", "tool_use_id": "tu"}});
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
fn chat_answer_from_other_session_refused() {
    let (fake, tmp) = (setup(), repo());
    for sid in ["session_other", ""] {
        let file = tmp
            .path()
            .join(git(tmp.path(), &["rev-parse", "--git-common-dir"]))
            .join("bosun/answers.json");
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        let v = json!({Q: {"answer": "approve", "pr": 1, "head": HEAD, "at": NOW,
            "session_id": sid, "tool_use_id": "tu"}});
        std::fs::write(file, v.to_string()).unwrap();
        let r = chat(&fake, tmp.path(), "session_orch", &["--question", Q], &[]);
        refused(&fake, r, 5);
    }
}

#[test]
fn chat_spawned_origin_refused() {
    let (fake, tmp) = (setup(), repo());
    answer(tmp.path(), Q, "approve", 1, HEAD);
    let env = [("CLAUDE_CODE_CHILD_SESSION", "1")];
    let r = chat(&fake, tmp.path(), "session_orch", &["--question", Q], &env);
    refused(&fake, r, 5);
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
    // An id `approve --list` never issued as a Claude Tag ask is refused.
    let r = chat(
        &fake,
        tmp.path(),
        "session_tag",
        &["--question", Q, "--slack-user", "U0ADA1"],
        &env,
    );
    refused(&fake, r, 5);
    answer_at(tmp.path(), Q, "asked", NOW);
    let (code, v) = chat(
        &fake,
        tmp.path(),
        "session_tag",
        &["--question", Q, "--slack-user", "U0ADA1"],
        &env,
    );
    assert_eq!(code, 0, "{v}");
    let body = bosun::core::approve::chat_comment(HEAD, "session_tag", NOW, Some(Q));
    assert_eq!(chat_comments(&fake), vec![body]);
}

#[test]
fn chat_slack_head_moved_refused() {
    let (fake, tmp) = (setup(), repo());
    join(&fake, "session_tag", "slack", None, 2);
    fake.pull_set(1, "head", json!({"sha": OTHER, "ref": "feature"}));
    let env = [("BOSUN_ORIGIN", "slack")];
    let r = chat(
        &fake,
        tmp.path(),
        "session_tag",
        &["--question", Q, "--slack-user", "U0ADA1"],
        &env,
    );
    refused(&fake, r, 3);
}

#[test]
fn chat_short_head_is_usage_error() {
    let (fake, tmp) = (setup(), repo());
    let out = bosun()
        .current_dir(tmp.path())
        .env("BOSUN_GITHUB_API", fake.url())
        .env("BOSUN_SESSION", "session_orch")
        .env("BOSUN_NOW", NOW)
        .args([
            "approve",
            &format!("1@{}", &HEAD[..7]),
            "--chat",
            "--question",
            Q,
        ])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(2));
    assert!(chat_comments(&fake).is_empty());
}

#[test]
fn chat_slack_hold_refused() {
    let (fake, tmp) = (setup(), repo());
    join(&fake, "session_tag", "slack", None, 2);
    answer(tmp.path(), "1-0123456-1762768801", "hold", 1, HEAD);
    let env = [("BOSUN_ORIGIN", "slack")];
    let r = chat(
        &fake,
        tmp.path(),
        "session_tag",
        &["--question", Q, "--slack-user", "U0ADA1"],
        &env,
    );
    refused(&fake, r, 5);
}

#[test]
fn chat_slack_wrong_question_prefix_refused() {
    let (fake, tmp) = (setup(), repo());
    join(&fake, "session_tag", "slack", None, 2);
    let env = [("BOSUN_ORIGIN", "slack")];
    for q in ["2-0123456-1762768800", "1-0000000-1762768800", "x"] {
        let r = chat(
            &fake,
            tmp.path(),
            "session_tag",
            &["--question", q, "--slack-user", "U0ADA1"],
            &env,
        );
        refused(&fake, r, 5);
    }
}

#[test]
fn chat_slack_user_needs_question() {
    let (fake, tmp) = (setup(), repo());
    join(&fake, "session_tag", "slack", None, 2);
    let env = [("BOSUN_ORIGIN", "slack")];
    let r = chat(
        &fake,
        tmp.path(),
        "session_tag",
        &["--slack-user", "U0ADA1"],
        &env,
    );
    assert_eq!(r.0, 2, "{}", r.1);
}

#[test]
fn chat_wave_agent_id_refused() {
    let (fake, tmp) = (setup(), repo());
    join(&fake, "S1/task", "local", None, 1);
    answer(tmp.path(), Q, "approve", 1, HEAD);
    let r = chat(&fake, tmp.path(), "S1/task", &["--question", Q], &[]);
    refused(&fake, r, 5);
}

fn usage_args(fake: &FakeGithub, tmp: &Path, args: &[&str]) -> i32 {
    let out = bosun()
        .current_dir(tmp)
        .env("BOSUN_GITHUB_API", fake.url())
        .env("BOSUN_SESSION", "session_orch")
        .env("BOSUN_NOW", NOW)
        .arg("approve")
        .args(args)
        .output()
        .unwrap();
    assert!(chat_comments(fake).is_empty());
    out.status.code().unwrap()
}

#[test]
fn chat_conflicting_flags_are_usage_errors() {
    let (fake, tmp) = (setup(), repo());
    let (a, b) = (format!("1@{HEAD}"), format!("2@{HEAD}"));
    let two = [a.as_str(), b.as_str(), "--chat", "--question", Q];
    assert_eq!(usage_args(&fake, tmp.path(), &two), 2);
    let list = [a.as_str(), "--chat", "--question", Q, "--list"];
    assert_eq!(usage_args(&fake, tmp.path(), &list), 2);
    let via = [a.as_str(), "--chat", "--question", Q, "--via-signer"];
    assert_eq!(usage_args(&fake, tmp.path(), &via), 2);
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
        &["--question", Q, "--slack-user", "U0EVE9"],
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
        &["--question", Q, "--slack-user", "U0ADA1"],
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
        &["--question", Q, "--slack-user", "U0ADA1"],
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

/// Add an answer to the stored file at time `at`.
fn answer_at(dir: &Path, id: &str, answer: &str, at: &str) {
    let file = dir
        .join(git(dir, &["rev-parse", "--git-common-dir"]))
        .join("bosun/answers.json");
    std::fs::create_dir_all(file.parent().unwrap()).unwrap();
    let mut map: Value = std::fs::read_to_string(&file)
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_else(|| json!({}));
    map[id] = json!({"answer": answer, "repo": "acme/widget", "pr": 1, "head": HEAD, "at": at,
        "session": "session_orch", "cli_session_id": "3f2c9a4e-1b7d-4c55-9a0e-5d6f7a8b9c0d", "tool_use_id": "tu"});
    std::fs::write(file, map.to_string()).unwrap();
}

#[test]
fn chat_needs_the_newest_answer_for_the_head_to_be_an_approve() {
    let (fake, tmp) = (setup(), repo());
    answer_at(tmp.path(), Q, "approve", "2026-11-10T09:00:00Z");
    answer_at(
        tmp.path(),
        "1-0123456-1762768999",
        "hold",
        "2026-11-10T09:30:00Z",
    );
    let r = chat(&fake, tmp.path(), "session_orch", &["--question", Q], &[]);
    refused(&fake, r, 5);
    // The newer answer approves: now it records.
    answer_at(
        tmp.path(),
        "1-0123456-1762769000",
        "approve",
        "2026-11-10T09:45:00Z",
    );
    let (code, v) = chat(&fake, tmp.path(), "session_orch", &["--question", Q], &[]);
    assert_eq!(code, 0, "{v}");
}

#[test]
fn chat_refused_when_the_child_flag_is_set_whatever_the_origin_says() {
    let (fake, tmp) = (setup(), repo());
    answer(tmp.path(), Q, "approve", 1, HEAD);
    let env = [
        ("CLAUDE_CODE_CHILD_SESSION", "1"),
        ("BOSUN_ORIGIN", "local"),
    ];
    let r = chat(&fake, tmp.path(), "session_orch", &["--question", Q], &env);
    refused(&fake, r, 5);
}

/// PR `n` at `head` in the ready list: its risk-check waits for an owner, a clean verdict, green CI.
fn ready(fake: &FakeGithub, n: u64, head: &str) {
    fake.pull(n, "change", &format!("b{n}"), "");
    fake.pull_as_issue(n, "change");
    fake.pull_set(n, "head", json!({"sha": head, "ref": format!("b{n}")}));
    fake.status(
        head,
        "risk-check",
        "failure",
        "high risk, owner approval needed: R-gov",
    );
    fake.status(head, "review-verdict", "success", "approved by session_rev");
    fake.check_run(head, "ci-ok", "completed", Some("success"));
    fake.compare_file("base0000", head, &format!("governance/f{n}.toml"), "@@");
}

fn slack_short(fake: &FakeGithub, dir: &Path, short: &str, q: &str) -> (i32, Value) {
    let out = bosun()
        .current_dir(dir)
        .env("BOSUN_GITHUB_API", fake.url())
        .env("BOSUN_SESSION", "session_tag")
        .env("BOSUN_NOW", NOW)
        .env("BOSUN_ORIGIN", "slack")
        .args([
            "approve",
            short,
            "--chat",
            "--question",
            q,
            "--slack-user",
            "U0ADA1",
        ])
        .output()
        .unwrap();
    (out.status.code().unwrap(), one_object(&out.stdout))
}

#[test]
fn chat_slack_short_head_must_name_exactly_one_ready_pr() {
    let (fake, tmp) = (setup(), repo());
    join(&fake, "session_tag", "slack", None, 2);
    answer_at(tmp.path(), Q, "asked", NOW);
    // Nothing is ready: zero matches.
    let (code, v) = slack_short(&fake, tmp.path(), &HEAD[..7], Q);
    assert_eq!(code, 3, "{v}");
    // Two ready PRs share the short head.
    ready(&fake, 1, HEAD);
    let twin = format!("{}{}", &HEAD[..7], &OTHER[7..]);
    ready(&fake, 2, &twin);
    let (code, v) = slack_short(&fake, tmp.path(), &HEAD[..7], Q);
    assert_eq!(code, 3, "{v}");
    assert!(chat_comments(&fake).is_empty());
}

#[test]
fn chat_slack_short_head_with_one_match_records() {
    let (fake, tmp) = (setup(), repo());
    join(&fake, "session_tag", "slack", None, 2);
    ready(&fake, 1, HEAD);
    answer_at(tmp.path(), Q, "asked", NOW);
    // The question names another PR than the one the short head maps to.
    let (code, v) = slack_short(&fake, tmp.path(), &HEAD[..7], "2-0123456-1762768800");
    assert_eq!(code, 3, "{v}");
    let (code, v) = slack_short(&fake, tmp.path(), &HEAD[..7], Q);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["head_sha"], HEAD);
    assert_eq!(chat_comments(&fake).len(), 1);
}

/// The hook stores the answer under the bosun identity (the same env as the CLI), not the CLI's
/// UUID: the whole path from a real-shaped hook input to `--chat`.
#[test]
fn chat_end_to_end_from_the_hook_input() {
    let (fake, tmp) = (setup(), repo());
    git(
        tmp.path(),
        &[
            "remote",
            "add",
            "origin",
            "https://github.com/acme/widget.git",
        ],
    );
    let text = format!(
        "Approve acme/widget#1 `change` (risk R-gov; verdict: ok; head 0123456 = {HEAD})? [bosun:{Q}]"
    );
    let q = json!({"question": text, "header": "Approve", "multiSelect": false,
        "options": [{"label": "Approve #1 0123456", "description": "x"},
                    {"label": "Hold", "description": "x"}]});
    let input = json!({"session_id": "3f2c9a4e-1b7d-4c55-9a0e-5d6f7a8b9c0d",
        "tool_use_id": "toolu_1", "hook_event_name": "PostToolUse",
        "tool_name": "AskUserQuestion", "tool_input": {"questions": [q.clone()]},
        "tool_response": {"questions": [q], "answers": {text: "Approve #1 0123456"}}});
    let hook = |session: &str| {
        let out = bosun()
            .current_dir(tmp.path())
            .env("BOSUN_NOW", NOW)
            .env("BOSUN_SESSION", session)
            .args(["hook", "post-tool-use"])
            .write_stdin(input.to_string())
            .output()
            .unwrap();
        assert_eq!(out.status.code(), Some(0));
    };
    hook("session_orch");
    // Another bosun session cannot use the orchestrator's answer.
    join(&fake, "session_other", "local", None, 1);
    let r = chat(&fake, tmp.path(), "session_other", &["--question", Q], &[]);
    refused(&fake, r, 5);
    let (code, v) = chat(&fake, tmp.path(), "session_orch", &["--question", Q], &[]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(chat_comments(&fake).len(), 1);
}
