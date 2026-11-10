//! `bosun merge <pr>` (plan 2026-11-10 T10, spec D13 as amended by D-019): the orchestrator
//! re-checks the gates for the PR's current head and merges it with `sha = head`.
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
const ORCH: &str = "session_orch";

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

fn repo() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    git(dir.path(), &["init", "-q", "-b", "claude/t7"]);
    std::fs::create_dir_all(dir.path().join(".bosun")).unwrap();
    std::fs::write(dir.path().join(".bosun/bosun.toml"), NO_CI).unwrap();
    git(dir.path(), &["add", "-A"]);
    git(dir.path(), &["commit", "-q", "-m", "x"]);
    dir
}

/// Task 7 in review with PR 20 by `session_w`; `ci` is the base config; `high` adds a
/// workflow change and an owner. `session_orch` is a root session in the registry.
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
    fake.comment(
        REG,
        &format!(
            "{{\"bosun\":\"join\",\"session\":\"{ORCH}\",\"origin\":\"spawned\",\"depth\":1,\"expires\":\"{LIVE}\"}}"
        ),
    );
    fake.comment(
        REG,
        &format!(
            "{{\"bosun\":\"join\",\"session\":\"session_kid\",\"origin\":\"spawned\",\"depth\":2,\"parent\":\"{ORCH}\",\"expires\":\"{LIVE}\"}}"
        ),
    );
    let dir = repo();
    stamps::ready_all(&fake, dir.path());
    (fake, dir)
}

fn verdict(fake: &FakeGithub) {
    let head = fake.pull_head(20);
    let v = Verdict::parse(
        &json!({"pr": 20, "head_sha": head, "verdict": "approve", "confidence": 0.95,
            "findings": [], "escalate_risk": false})
        .to_string(),
    )
    .unwrap();
    fake.comment_by(20, "rev", &comment_body(&v, "session_rev", None));
}

fn chat(fake: &FakeGithub) {
    let head = fake.pull_head(20);
    fake.comment_by(
        20,
        "ada",
        &format!(
            "/bosun approve {head} chat\nApproved by the owner in chat.\n<!-- bosun:chat-approval v1 session={ORCH} head={head} question=20-x-1 -->"
        ),
    );
}

fn run_as(fake: &FakeGithub, dir: &Path, session: &str, args: &[&str]) -> (i32, Value) {
    let out = bosun()
        .env("BOSUN_GITHUB_API", fake.url())
        .env("BOSUN_SESSION", session)
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
    run_as(fake, dir, "session_w", &["pr", "7", "--ready"])
}

fn merge(fake: &FakeGithub, dir: &Path) -> (i32, Value) {
    run_as(fake, dir, ORCH, &["merge", "20"])
}

fn merge_puts(fake: &FakeGithub) -> Vec<Value> {
    (fake.requests().into_iter())
        .filter(|r| r.method == "PUT" && r.path.ends_with("/pulls/20/merge"))
        .map(|r| serde_json::from_str(r.body.as_deref().unwrap_or("")).unwrap_or(Value::Null))
        .collect()
}

/// No CI, normal risk, clean verdict, ready summary: the orchestrator merges at the head.
#[test]
fn merge_no_ci_ok() {
    let (fake, dir) = setup(false, Some(NO_CI));
    verdict(&fake);
    assert_eq!(ready(&fake, dir.path()).0, 0);
    let head = fake.pull_head(20);
    let (code, v) = merge(&fake, dir.path());
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["merged"], true, "{v}");
    assert_eq!(v["data"]["head_sha"], head.as_str());
    let puts = merge_puts(&fake);
    assert_eq!(puts.len(), 1, "{puts:?}");
    assert_eq!(puts[0]["sha"], head.as_str());
    assert_eq!(puts[0]["merge_method"], "squash");
    assert!(fake.merged(20));
    assert!(fake.labels_of(7).contains(&"bosun:state/merged".into()));
}

/// The head moved after the ready summary: exit 3, nothing merged.
#[test]
fn merge_head_moved_refused() {
    let (fake, dir) = setup(false, Some(NO_CI));
    verdict(&fake);
    assert_eq!(ready(&fake, dir.path()).0, 0);
    fake.pull_head_sha(20, "9999999999999999999999999999999999999999");
    let (code, v) = merge(&fake, dir.path());
    assert_eq!(code, 3, "{v}");
    assert!(merge_puts(&fake).is_empty());
    assert!(!fake.merged(20));
}

/// GitHub refuses the PUT because the head moved between the checks and the merge.
#[test]
fn merge_head_moved_at_put_is_conflict() {
    let (fake, dir) = setup(false, Some(NO_CI));
    verdict(&fake);
    assert_eq!(ready(&fake, dir.path()).0, 0);
    fake.fail_path_message("/pulls/20/merge", 409, "Head branch was modified");
    let (code, v) = merge(&fake, dir.path());
    assert_eq!(code, 3, "{v}");
    assert!(!fake.merged(20));
}

#[test]
fn merge_child_refused() {
    let (fake, dir) = setup(false, Some(NO_CI));
    verdict(&fake);
    assert_eq!(ready(&fake, dir.path()).0, 0);
    for s in ["session_kid", "session_w", "S/7"] {
        let (code, v) = run_as(&fake, dir.path(), s, &["merge", "20"]);
        assert_eq!(code, 5, "{s}: {v}");
        assert!(
            v["error"]["fix"].as_str().unwrap().contains("handoff-pr"),
            "{v}"
        );
    }
    assert!(merge_puts(&fake).is_empty());
}

#[test]
fn merge_halted_exit4() {
    let (fake, dir) = setup(false, Some(NO_CI));
    verdict(&fake);
    assert_eq!(ready(&fake, dir.path()).0, 0);
    fake.variable("BOSUN_HALT", "1");
    assert_eq!(merge(&fake, dir.path()).0, 4);
    assert!(merge_puts(&fake).is_empty());
}

/// Gated without the owner's chat answer: no summary says ready, the merge waits (exit 5).
#[test]
fn merge_gated_without_chat_refused() {
    let (fake, dir) = setup(true, Some(NO_CI));
    verdict(&fake);
    assert_eq!(ready(&fake, dir.path()).0, 5);
    let (code, v) = merge(&fake, dir.path());
    assert_eq!(code, 5, "{v}");
    assert!(merge_puts(&fake).is_empty());
    // With the owner's answer recorded, the merge goes through (re-checked, no rerun needed).
    chat(&fake);
    let (code, v) = merge(&fake, dir.path());
    assert_eq!(code, 0, "{v}");
    assert!(fake.merged(20));
}

/// No CI and nobody ran `pr --ready`: no summary for the head, exit 5.
#[test]
fn merge_no_ci_without_summary_refused() {
    let (fake, dir) = setup(false, Some(NO_CI));
    verdict(&fake);
    let (code, v) = merge(&fake, dir.path());
    assert_eq!(code, 5, "{v}");
    assert!(merge_puts(&fake).is_empty());
}

#[test]
fn merge_no_ci_main_red_halts() {
    let (fake, dir) = setup(false, Some(NO_CI));
    verdict(&fake);
    assert_eq!(ready(&fake, dir.path()).0, 0);
    fake.issue(9, "main is red", &["bosun:main-red"], "");
    assert_eq!(merge(&fake, dir.path()).0, 4);
    assert!(merge_puts(&fake).is_empty());
}

fn green(fake: &FakeGithub, except: Option<&str>) {
    let head = fake.pull_head(20);
    for c in ["ci-ok", "risk-check", "review-verdict", "main-open"] {
        let state = if Some(c) == except {
            "pending"
        } else {
            "success"
        };
        fake.status(&head, c, state, "x");
    }
}

#[test]
fn merge_github_requires_green_checks() {
    let (fake, dir) = setup(false, None);
    green(&fake, Some("review-verdict"));
    let (code, v) = merge(&fake, dir.path());
    assert_eq!(code, 5, "{v}");
    assert!(
        v["error"]["message"]
            .as_str()
            .unwrap()
            .contains("review-verdict"),
        "{v}"
    );
    assert!(merge_puts(&fake).is_empty());
}

/// The `already_clean` case: every required check green, GitHub cannot turn on auto-merge.
#[test]
fn merge_github_already_clean_ok() {
    let (fake, dir) = setup(false, None);
    green(&fake, None);
    let (code, v) = run_as(
        &fake,
        dir.path(),
        ORCH,
        &["merge", "20", "--method", "rebase"],
    );
    assert_eq!(code, 0, "{v}");
    let puts = merge_puts(&fake);
    assert_eq!(puts.len(), 1);
    assert_eq!(puts[0]["merge_method"], "rebase");
    assert!(fake.merged(20));
}

/// Already merged: nothing to do, the task follows.
#[test]
fn merge_already_merged_is_ok() {
    let (fake, dir) = setup(false, None);
    fake.pull_merged(20, "m1");
    let (code, v) = merge(&fake, dir.path());
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["merged"], true);
    assert!(merge_puts(&fake).is_empty());
    assert!(fake.labels_of(7).contains(&"bosun:state/merged".into()));
}
