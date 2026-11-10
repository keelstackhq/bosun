//! `bosun pr --ready` with `[ci] provider = "none"` (plan 2026-11-10 T9, spec §4): bosun runs the
//! checks CI would, posts one summary per head and queues the task for `bosun merge`.
mod common;

use std::path::Path;
use std::process::Command;

use bosun::core::verdict::{Verdict, comment_body};
use common::fake::FakeGithub;
use common::{bosun, lease_note, one_object, stamps};
use serde_json::{Value, json};

const NOW: &str = "2026-11-10T10:00:00Z";
const LIVE: &str = "2026-11-10T11:00:00Z";
const BASE: &str = "base0000";
const NO_CI: &str = "[ci]\nprovider = \"none\"\n";
const REG: u64 = 50;

fn git(dir: &Path, args: &[&str]) {
    let st = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(["-c", "user.name=t", "-c", "user.email=t@t"])
        .args(args)
        .status()
        .unwrap();
    assert!(st.success());
}

/// A checkout on `claude/t7` whose own config says no CI (the approve board reads it).
fn repo() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    git(dir.path(), &["init", "-q", "-b", "claude/t7"]);
    std::fs::create_dir_all(dir.path().join(".bosun")).unwrap();
    std::fs::write(dir.path().join(".bosun/bosun.toml"), NO_CI).unwrap();
    git(dir.path(), &["add", "-A"]);
    git(dir.path(), &["commit", "-q", "-m", "x"]);
    dir
}

/// Task 7 in review, its PR 20 leased by `session_w`, no CI at base; `high` adds a workflow
/// change and an owner. Every gate is stamped on the local head, which is the PR head.
fn setup(high: bool, ci: Option<&str>) -> (FakeGithub, tempfile::TempDir) {
    let fake = FakeGithub::start();
    fake.issue(
        7,
        "the task",
        &["bosun:state/review"],
        "Do it.\n<!-- bosun:task {} -->",
    );
    fake.comment(7, &lease_note("session_w", LIVE, "**"));
    fake.pull(20, "t", "claude/t7", "Closes #7");
    fake.pull_as_issue(20, "t");
    if let Some(c) = ci {
        fake.content(".bosun/bosun.toml", BASE, c);
    }
    if high {
        let patch = Some("@@ -0,0 +1,1 @@\n+x");
        fake.pull_file(20, ".github/workflows/ci.yml", "modified", 1, 0, patch);
        fake.content(
            "governance/owners.toml",
            BASE,
            "[[owner]]\ngithub = \"ada\"\n",
        );
    }
    fake.issue(REG, "bosun registry", &["bosun:registry"], "");
    let dir = repo();
    stamps::ready_all(&fake, dir.path());
    (fake, dir)
}

fn verdict(fake: &FakeGithub, session: &str, decision: &str) {
    verdict_with(fake, session, decision, false);
}

fn verdict_with(fake: &FakeGithub, session: &str, decision: &str, escalate: bool) {
    let head = fake.pull_head(20);
    let v = Verdict::parse(
        &json!({"pr": 20, "head_sha": head, "verdict": decision, "confidence": 0.95,
            "findings": [], "escalate_risk": escalate})
        .to_string(),
    )
    .unwrap();
    fake.comment_by(20, "rev", &comment_body(&v, session, None));
}

fn chat(fake: &FakeGithub) {
    let head = fake.pull_head(20);
    fake.comment(
        REG,
        &format!(
            "{{\"bosun\":\"join\",\"session\":\"session_orch\",\"origin\":\"spawned\",\"depth\":1,\"expires\":\"{LIVE}\"}}"
        ),
    );
    fake.comment_by(
        20,
        "ada",
        &format!(
            "/bosun approve {head} chat\nApproved by the owner in chat.\n<!-- bosun:chat-approval v1 session=session_orch head={head} question=20-x-1 -->"
        ),
    );
}

fn run(fake: &FakeGithub, dir: &Path, args: &[&str]) -> (i32, Value) {
    let out = bosun()
        .env("BOSUN_GITHUB_API", fake.url())
        .env("BOSUN_SESSION", "session_w")
        .env("BOSUN_NOW", NOW)
        .arg("-C")
        .arg(dir)
        .args(args)
        .output()
        .unwrap();
    let v = one_object(&out.stdout);
    if v["ok"] == false {
        assert!(!v["error"]["fix"].as_str().unwrap().trim().is_empty());
    }
    (out.status.code().unwrap(), v)
}

fn ready(fake: &FakeGithub, dir: &Path) -> (i32, Value) {
    run(fake, dir, &["pr", "7", "--ready"])
}

fn summaries(fake: &FakeGithub) -> Vec<String> {
    (fake.comments_of(20).into_iter())
        .map(|c| c.body)
        .filter(|b| b.starts_with("/bosun ready "))
        .collect()
}

fn status_posts(fake: &FakeGithub) -> usize {
    (fake.requests().iter())
        .filter(|r| r.method == "POST" && r.path.contains("/statuses/"))
        .count()
}

#[test]
fn ready_no_ci_missing_stamp_exit5() {
    let (fake, dir) = setup(false, Some(NO_CI));
    std::fs::remove_file(stamps::stamp_dir(dir.path()).join("lint")).unwrap();
    verdict(&fake, "session_rev", "approve");
    let (code, v) = ready(&fake, dir.path());
    assert_eq!(code, 5, "{v}");
    assert_eq!(v["data"]["missing_gates"], json!(["lint"]), "{v}");
    assert!(summaries(&fake).is_empty());
}

#[test]
fn ready_no_ci_needs_a_verdict_from_another_session() {
    let (fake, dir) = setup(false, Some(NO_CI));
    verdict(&fake, "session_w", "approve");
    let (code, v) = ready(&fake, dir.path());
    assert_eq!(code, 5, "{v}");
    assert!(
        v["error"]["message"].as_str().unwrap().contains("verdict"),
        "{v}"
    );
    assert!(summaries(&fake).is_empty());
    // A reviewer that is not a writer does not count either.
    fake.permission("rev", "read");
    verdict(&fake, "session_rev", "approve");
    assert_eq!(ready(&fake, dir.path()).0, 5);
}

#[test]
fn ready_no_ci_gated_without_chat_exit5() {
    let (fake, dir) = setup(true, Some(NO_CI));
    verdict(&fake, "session_rev", "approve");
    let (code, v) = ready(&fake, dir.path());
    assert_eq!(code, 5, "{v}");
    assert_eq!(v["data"]["next_command"], "bosun approve --list", "{v}");
    assert!(
        v["error"]["fix"].as_str().unwrap().contains("--chat"),
        "{v}"
    );
    let s = summaries(&fake);
    assert_eq!(s.len(), 1, "{s:?}");
    assert!(s[0].contains("- risk: high: "), "{}", s[0]);
    assert!(s[0].contains("- approval: none"), "{}", s[0]);
    assert!(fake.labels_of(7).contains(&"bosun:state/review".into()));
}

#[test]
fn ready_no_ci_posts_summary_once() {
    let (fake, dir) = setup(true, Some(NO_CI));
    verdict(&fake, "session_rev", "approve");
    assert_eq!(ready(&fake, dir.path()).0, 5);
    assert_eq!(ready(&fake, dir.path()).0, 5);
    assert_eq!(summaries(&fake).len(), 1);
    chat(&fake);
    let (code, v) = ready(&fake, dir.path());
    assert_eq!(code, 0, "{v}");
    let s = summaries(&fake);
    assert_eq!(s.len(), 2, "{s:?}");
    assert!(s[1].contains("- approval: ada"), "{}", s[1]);
}

#[test]
fn ready_no_ci_no_status_no_auto_merge() {
    let (fake, dir) = setup(false, Some(NO_CI));
    verdict(&fake, "session_rev", "approve");
    let (code, v) = ready(&fake, dir.path());
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["task_state"], "queued", "{v}");
    assert_eq!(v["data"]["merge_next"], "bosun merge 20", "{v}");
    assert_eq!(v["data"]["auto_merge"], false);
    assert_eq!(status_posts(&fake), 0);
    assert_eq!(fake.auto_merge_enabled(20), None);
    assert!(fake.labels_of(7).contains(&"bosun:state/queued".into()));
    let s = summaries(&fake);
    assert_eq!(s.len(), 1, "{s:?}");
    assert!(s[0].contains("- risk: normal") && s[0].contains("- verdict: session_rev"));
}

#[test]
fn ready_no_ci_main_red_halts() {
    let (fake, dir) = setup(false, Some(NO_CI));
    verdict(&fake, "session_rev", "approve");
    fake.issue(9, "main is red", &["bosun:main-red"], "");
    let (code, v) = ready(&fake, dir.path());
    assert_eq!(code, 4, "{v}");
    assert!(
        v["error"]["message"]
            .as_str()
            .unwrap()
            .contains("main is red"),
        "{v}"
    );
    assert!(summaries(&fake).is_empty());
}

#[test]
fn ready_github_provider_unchanged() {
    let (fake, dir) = setup(false, None);
    verdict(&fake, "session_rev", "approve");
    let (code, v) = ready(&fake, dir.path());
    // The CI path: the base does not require the gates, so auto-merge is refused (exit 5).
    assert_eq!(code, 5, "{v}");
    assert!(
        v["error"]["message"]
            .as_str()
            .unwrap()
            .contains("does not require"),
        "{v}"
    );
    assert!(summaries(&fake).is_empty());
}

#[test]
fn approve_list_no_ci_uses_summary() {
    let (fake, dir) = setup(true, Some(NO_CI));
    verdict(&fake, "session_rev", "approve");
    assert_eq!(ready(&fake, dir.path()).0, 5);
    fake.allow_anonymous_reads();
    let head = fake.pull_head(20);
    fake.compare_file(BASE, &head, ".github/workflows/ci.yml", "@@ -0,0 +1 @@\n+x");
    let (code, v) = run(&fake, dir.path(), &["approve", "--list"]);
    assert_eq!(code, 0, "{v}");
    let ready = v["data"]["ready"].as_array().unwrap();
    assert_eq!(ready.len(), 1, "{v}");
    assert_eq!(ready[0]["pr"], 20);
    // Approved in chat: the next summary names it, and the board no longer asks.
    chat(&fake);
    assert_eq!(self::ready(&fake, dir.path()).0, 0);
    let (_, v) = run(&fake, dir.path(), &["approve", "--list"]);
    assert_eq!(v["data"]["ready"], json!([]), "{v}");
}

#[test]
fn ready_no_ci_escalated_verdict_does_not_count() {
    let (fake, dir) = setup(false, Some(NO_CI));
    verdict_with(&fake, "session_rev", "approve", true);
    let (code, v) = ready(&fake, dir.path());
    assert_eq!(code, 5, "{v}");
    assert!(
        v["error"]["message"].as_str().unwrap().contains("verdict"),
        "{v}"
    );
    assert!(summaries(&fake).is_empty());
}

#[test]
fn ready_no_ci_summary_with_proxy_footer_not_reposted() {
    let (fake, dir) = setup(true, Some(NO_CI));
    verdict(&fake, "session_rev", "approve");
    assert_eq!(ready(&fake, dir.path()).0, 5);
    fake.last_comment(|c| c.body.push_str("\n\n---\n_Generated by Claude Code_"));
    assert_eq!(ready(&fake, dir.path()).0, 5);
    assert_eq!(
        summaries(&fake).len(),
        1,
        "{:?}",
        fake.comments_of(20)
            .iter()
            .map(|c| c.body.clone())
            .collect::<Vec<_>>()
    );
}
