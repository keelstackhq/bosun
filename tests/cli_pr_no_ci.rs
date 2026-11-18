//! `bosun pr --ready` with `[ci] provider = "none"` (plan 2026-11-10 T9, spec §4): bosun runs the
//! checks CI would, posts one summary per head and queues the task for `bosun merge`.
mod common;

use bosun::core::approve::{Readiness, parse_ready_summary, ready_summary, summary_readiness};

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
    let task = (
        &["bosun:state/review"][..],
        "Do it.\n<!-- bosun:task {} -->",
    );
    setup_with(high, ci, task)
}

fn setup_with(
    high: bool,
    ci: Option<&str>,
    (labels, body): (&[&str], &str),
) -> (FakeGithub, tempfile::TempDir) {
    let fake = FakeGithub::start();
    fake.issue(7, "the task", labels, body);
    fake.comment(7, &lease_note("session_w", LIVE, "**"));
    fake.pull(20, "t", "claude/t7", "Closes #7");
    fake.pull_as_issue(20, "t");
    if let Some(c) = ci {
        fake.content(".bosun/bosun.toml", BASE, c);
        fake.content(".bosun/bosun.toml", "refs/heads/main", c);
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
    let head = fake.pull_head(20);
    let v = Verdict::parse(
        &json!({"pr": 20, "head_sha": head, "verdict": decision, "confidence": 0.95,
            "findings": []})
        .to_string(),
    )
    .unwrap();
    fake.comment_by(20, "rev", &comment_body(&v, session, None));
}

fn chat(fake: &FakeGithub) {
    let head = fake.pull_head(20);
    let short = &head[..7];
    fake.comment(
        REG,
        &format!(
            "{{\"bosun\":\"join\",\"session\":\"session_orch\",\"origin\":\"local\",\"depth\":1,\"expires\":\"{LIVE}\"}}"
        ),
    );
    fake.comment_by(
        20,
        "ada",
        &format!(
            "/bosun approve {head} chat\nApproved by the owner in chat.\n<!-- bosun:chat-approval v1 session=session_orch head={head} question=20-{short}-1 -->"
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
    // Approved in chat: the board no longer asks, before and after the next summary.
    chat(&fake);
    let (_, v) = run(&fake, dir.path(), &["approve", "--list"]);
    assert_eq!(v["data"]["ready"], json!([]), "{v}");
    assert_eq!(self::ready(&fake, dir.path()).0, 0);
    let (_, v) = run(&fake, dir.path(), &["approve", "--list"]);
    assert_eq!(v["data"]["ready"], json!([]), "{v}");
}

fn rev_comment(fake: &FakeGithub, user: &str, edited: bool) {
    let head = fake.pull_head(20);
    let j = json!({"pr": 20, "head_sha": head, "verdict": "approve", "confidence": 0.95,
        "findings": []});
    let body = comment_body(
        &Verdict::parse(&j.to_string()).unwrap(),
        "session_rev",
        None,
    );
    if edited {
        fake.comment_edited(20, user, &body);
    } else {
        fake.comment_by(20, user, &body);
    }
}

#[test]
fn ready_no_ci_ignores_non_writer_and_edited_verdicts() {
    let (fake, dir) = setup(false, Some(NO_CI));
    fake.permission("mallory", "read");
    rev_comment(&fake, "mallory", false);
    rev_comment(&fake, "rev", true);
    let (code, v) = ready(&fake, dir.path());
    assert_eq!(code, 5, "{v}");
    assert!(summaries(&fake).is_empty());
    rev_comment(&fake, "rev", false);
    assert_eq!(ready(&fake, dir.path()).0, 0);
}

#[test]
fn approve_list_no_ci_ignores_a_non_writer_summary() {
    let (fake, dir) = setup(true, Some(NO_CI));
    fake.allow_anonymous_reads();
    let head = fake.pull_head(20);
    fake.compare_file(BASE, &head, ".github/workflows/ci.yml", "@@ -0,0 +1 @@\n+x");
    let body = ready_summary(&head, &[], "high: R-ci", "s", None);
    fake.permission("mallory", "read");
    fake.comment_by(20, "mallory", &body);
    let (_, v) = run(&fake, dir.path(), &["approve", "--list"]);
    assert_eq!(v["data"]["ready"], json!([]), "{v}");
    fake.comment_by(20, "rev", &body);
    let (_, v) = run(&fake, dir.path(), &["approve", "--list"]);
    assert_eq!(v["data"]["ready"].as_array().unwrap().len(), 1, "{v}");
}

#[test]
fn pr_without_a_task_is_refused_without_ci() {
    let (fake, dir) = setup(false, Some(NO_CI));
    fake.pull(21, "gov", "claude/gov", "no task");
    fake.pull_as_issue(21, "gov");
    let (code, v) = run(&fake, dir.path(), &["pr", "--pr", "21", "--ready"]);
    assert_ne!(code, 0, "{v}");
    assert_eq!(fake.auto_merge_enabled(21), None);
}

#[test]
fn ready_no_ci_leaves_a_plain_issue_unlabelled() {
    let (fake, dir) = setup_with(false, Some(NO_CI), (&[], "Do it."));
    verdict(&fake, "session_rev", "approve");
    let (code, v) = ready(&fake, dir.path());
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["task_state"], Value::Null, "{v}");
    assert!(fake.labels_of(7).is_empty(), "{:?}", fake.labels_of(7));
}

const H: &str = "abcdef1234567890abcdef1234567890abcdef12";
const H2: &str = "1111111111111111111111111111111111111111";

#[test]
fn ready_summary_round_trips() {
    let g = vec!["fmt".to_string(), "test".to_string()];
    let b = ready_summary(H, &g, "high: R-gov, R-ci", "session_r", None);
    assert!(b.starts_with(&format!("/bosun ready {H}\n")), "{b}");
    assert!(
        b.ends_with(&format!("<!-- bosun:ready v1 head={H} -->")),
        "{b}"
    );
    assert!(parse_ready_summary(&b, H));
    assert!(!parse_ready_summary(&b, H2));
    assert!(!parse_ready_summary(&format!("x\n{b}"), H));
    let (r, v) = summary_readiness(Some(&b));
    assert_eq!(r, Readiness::Ready(vec!["R-gov".into(), "R-ci".into()]));
    assert_eq!(v, "session_r");
    let ok = ready_summary(H, &g, "high: R-gov", "s", Some("ada (chat)"));
    assert_eq!(summary_readiness(Some(&ok)).0, Readiness::NotWaiting);
    let low = ready_summary(H, &g, "normal", "s", None);
    assert_eq!(summary_readiness(Some(&low)).0, Readiness::NotWaiting);
    assert_eq!(summary_readiness(None).0, Readiness::NotWaiting);
}

#[test]
fn ready_no_ci_refuses_a_pr_retargeted_off_the_default_branch() {
    let (fake, dir) = setup(false, Some(NO_CI));
    verdict(&fake, "session_rev", "approve");
    fake.pull_set(20, "base", json!({"sha": BASE, "ref": "release"}));
    let (code, v) = ready(&fake, dir.path());
    assert_eq!(code, 3, "{v}");
    assert!(summaries(&fake).is_empty());
}

#[test]
fn the_no_ci_path_reads_the_default_branch_not_the_head_or_the_pr_base() {
    // The checkout (the PR head) says none; the default branch says github: the no-CI path is
    // not used, so no local summary is posted.
    let (fake, dir) = setup(false, None);
    fake.content(
        ".bosun/bosun.toml",
        "refs/heads/main",
        "[ci]\nprovider = \"github\"\n",
    );
    fake.content(".bosun/bosun.toml", BASE, NO_CI);
    verdict(&fake, "session_rev", "approve");
    let _ = ready(&fake, dir.path());
    assert!(summaries(&fake).is_empty());
}
