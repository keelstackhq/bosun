//! risk-check counts a chat approval record as the owner's approval (spec §8, plan 2026-11-10 T2).
mod common;

use common::fake::FakeGithub;
use common::run;
use serde_json::{Value, json};

const NOW: &str = "2026-11-10T10:00:00Z";
const LIVE: &str = "2026-11-10T11:00:00Z";
const BASE: &str = "base0000";
const HEAD: &str = "0123456789abcdef0123456789abcdef01234567";
const OLD: &str = "1111111111111111111111111111111111111111";
const OWNERS: &str = "[[owner]]\ngithub = \"ada\"\n";
const REG: u64 = 50;

fn chat(head: &str, session: &str) -> String {
    format!(
        "/bosun approve {head} chat\nApproved by the owner in chat (session {session}, {NOW}).\n<!-- bosun:chat-approval v1 session={session} head={head} question=1-0123456-1 -->"
    )
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

/// High PR 1 at HEAD (a workflow change, owner class), owners at base, registry with root
/// `session_orch`.
fn high_pr(owners: bool) -> FakeGithub {
    let fake = FakeGithub::start();
    fake.pull(1, "change", "feature", "");
    fake.pull_as_issue(1, "change");
    fake.pull_set(1, "head", json!({"sha": HEAD, "ref": "feature"}));
    if owners {
        fake.content("governance/owners.toml", BASE, OWNERS);
    }
    let patch = Some("@@ -0,0 +1,1 @@\n+x");
    fake.pull_file(1, ".github/workflows/ci.yml", "modified", 1, 0, patch);
    fake.issue(REG, "bosun registry", &["bosun:registry"], "");
    join(&fake, "session_orch", "spawned", None, 1);
    fake
}

/// A comment by `user` through the Claude app, as a cloud session posts it.
fn app_comment(fake: &FakeGithub, user: &str, text: &str) {
    fake.comment_by(1, user, text);
    fake.last_comment(|c| c.via_app = Some("claude".into()));
}

fn check(fake: &FakeGithub) -> (i32, Value) {
    run(fake, "session_test", NOW, &["risk-check", "1"])
}

fn approved(fake: &FakeGithub) -> Value {
    let (code, v) = check(fake);
    assert_eq!(code, 0, "{v}");
    v["data"]["approved_by"].clone()
}

fn refused(fake: &FakeGithub) {
    let (code, v) = check(fake);
    assert_eq!(code, 5, "{v}");
    assert_eq!(v["data"]["approved_by"], Value::Null, "{v}");
}

#[test]
fn chat_record_counts_as_owner() {
    let fake = high_pr(true);
    app_comment(&fake, "ada", &chat(HEAD, "session_orch"));
    assert_eq!(approved(&fake), "ada");
}

#[test]
fn via_app_author_still_counts() {
    // A plain `/bosun approve` through the app does not count; the chat record does.
    let fake = high_pr(true);
    app_comment(&fake, "ada", &format!("/bosun approve {HEAD}"));
    refused(&fake);
    app_comment(&fake, "ada", &chat(HEAD, "session_orch"));
    assert_eq!(approved(&fake), "ada");
}

#[test]
fn chat_record_edited_or_stale_not_counted() {
    let fake = high_pr(true);
    app_comment(&fake, "ada", &chat(HEAD, "session_orch"));
    fake.last_comment(|c| c.updated_at = "2026-11-10T09:59:00Z".into());
    refused(&fake);
    let fake = high_pr(true);
    app_comment(&fake, "ada", &chat(OLD, "session_orch"));
    refused(&fake);
}

#[test]
fn chat_record_child_session_not_counted() {
    let fake = high_pr(true);
    join(&fake, "session_kid", "spawned", Some("session_orch"), 2);
    app_comment(&fake, "ada", &chat(HEAD, "session_kid"));
    refused(&fake);
    let fake = high_pr(true);
    app_comment(&fake, "ada", &chat(HEAD, "S/3"));
    refused(&fake);
    // A session the registry does not know.
    let fake = high_pr(true);
    app_comment(&fake, "ada", &chat(HEAD, "session_ghost"));
    refused(&fake);
}

#[test]
fn chat_record_non_owner_author_not_counted() {
    let fake = high_pr(true);
    app_comment(&fake, "mallory", &chat(HEAD, "session_orch"));
    refused(&fake);
}

#[test]
fn chat_record_withdrawn_by_changes_requested() {
    let fake = high_pr(true);
    fake.review(1, "ada", "CHANGES_REQUESTED", HEAD);
    app_comment(&fake, "ada", &chat(HEAD, "session_orch"));
    refused(&fake);
}

#[test]
fn chat_off_ignores_record() {
    let fake = high_pr(true);
    fake.content(".bosun/bosun.toml", BASE, "[approvals]\nchat = false\n");
    app_comment(&fake, "ada", &chat(HEAD, "session_orch"));
    refused(&fake);
}

#[test]
fn chat_counts_on_r_weaken() {
    let fake = FakeGithub::start();
    fake.pull(1, "change", "feature", "");
    fake.pull_as_issue(1, "change");
    fake.pull_set(1, "head", json!({"sha": HEAD, "ref": "feature"}));
    fake.content("governance/owners.toml", BASE, OWNERS);
    let patch = "@@ -1,2 +1,2 @@\n [budget]\n-daily_usd = 150\n+daily_usd = 300";
    fake.pull_file(1, ".bosun/limits.toml", "modified", 1, 1, Some(patch));
    fake.content(".bosun/limits.toml", BASE, "[budget]\ndaily_usd = 150\n");
    fake.content(".bosun/limits.toml", HEAD, "[budget]\ndaily_usd = 300\n");
    fake.issue(REG, "bosun registry", &["bosun:registry"], "");
    join(&fake, "session_orch", "local", None, 1);
    app_comment(&fake, "ada", &chat(HEAD, "session_orch"));
    let (code, v) = check(&fake);
    assert_eq!(code, 0, "{v}");
    let rules = v["data"]["matches"].to_string();
    assert!(rules.contains("\"R-weaken\""), "{v}");
    assert_eq!(v["data"]["approved_by"], "ada");
}

#[test]
fn no_owners_file_user_repo_owner_counts() {
    let fake = high_pr(false);
    fake.repo_set("owner", json!({"login": "ada", "type": "User"}));
    app_comment(&fake, "ada", &chat(HEAD, "session_orch"));
    assert_eq!(approved(&fake), "ada");
    // Someone else's record does not count for the repo owner.
    let fake = high_pr(false);
    fake.repo_set("owner", json!({"login": "ada", "type": "User"}));
    app_comment(&fake, "bob", &chat(HEAD, "session_orch"));
    refused(&fake);
}

#[test]
fn no_owners_file_org_admin_counts() {
    let fake = high_pr(false);
    fake.repo_set("owner", json!({"login": "acme", "type": "Organization"}));
    fake.pull_set(1, "user", json!({"login": "ada"}));
    fake.permission("ada", "admin");
    app_comment(&fake, "ada", &chat(HEAD, "session_orch"));
    assert_eq!(approved(&fake), "ada");
}

#[test]
fn no_owners_file_org_member_not_counted() {
    let fake = high_pr(false);
    fake.repo_set("owner", json!({"login": "acme", "type": "Organization"}));
    fake.pull_set(1, "user", json!({"login": "ada"}));
    app_comment(&fake, "ada", &chat(HEAD, "session_orch"));
    refused(&fake);
    // Without an owners file a plain owner comment still counts for no one.
    let fake = high_pr(false);
    fake.repo_set("owner", json!({"login": "ada", "type": "User"}));
    fake.comment_by(1, "ada", &format!("/bosun approve {HEAD}"));
    refused(&fake);
}

#[test]
fn empty_owners_file_is_not_the_fallback() {
    // An owners file that exists but names nobody keeps the empty-owners halt (fail closed),
    // even for the repo owner's own record.
    let fake = high_pr(false);
    fake.content("governance/owners.toml", BASE, "");
    fake.repo_set("owner", json!({"login": "ada", "type": "User"}));
    app_comment(&fake, "ada", &chat(HEAD, "session_orch"));
    let (code, v) = check(&fake);
    assert_ne!(code, 0, "{v}");
    assert_eq!(v["data"]["approved_by"], Value::Null, "{v}");
}

#[test]
fn failed_repo_or_permission_read_does_not_approve() {
    let fake = high_pr(false);
    fake.repo_set("owner", json!({"login": "acme", "type": "Organization"}));
    fake.pull_set(1, "user", json!({"login": "ada"}));
    fake.permission("ada", "admin");
    fake.fail_path("/collaborators/ada/permission", 500);
    app_comment(&fake, "ada", &chat(HEAD, "session_orch"));
    let (code, v) = check(&fake);
    assert_ne!(code, 0, "{v}");
    assert_ne!(v["data"]["approved_by"], "ada", "{v}");
    let fake = high_pr(false);
    fake.repo_set("owner", json!({"login": "ada", "type": "User"}));
    fake.fail_path("/repos/acme/widget", 500);
    app_comment(&fake, "ada", &chat(HEAD, "session_orch"));
    let (code, v) = check(&fake);
    assert_ne!(code, 0, "{v}");
    assert_ne!(v["data"]["approved_by"], "ada", "{v}");
}

#[test]
fn chat_record_after_session_expired_not_counted() {
    let fake = high_pr(true);
    app_comment(&fake, "ada", &chat(HEAD, "session_orch"));
    fake.last_comment(|c| {
        c.created_at = "2026-11-10T12:00:00Z".into();
        c.updated_at = "2026-11-10T12:00:00Z".into();
    });
    refused(&fake);
}

const ESCALATED: &str =
    "escalated to the owner (approvable): approve 0.95 (approves at 0.85), 0 blocker, 0 major";

fn last_status(fake: &FakeGithub, ctx: &str) -> Value {
    (fake.statuses_of(HEAD).into_iter().rev())
        .find(|s| s["context"] == ctx)
        .unwrap()
}

#[test]
fn chat_clears_low_risk_escalation() {
    let fake = FakeGithub::start();
    fake.pull(1, "change", "feature", "");
    fake.pull_as_issue(1, "change");
    fake.pull_set(1, "head", json!({"sha": HEAD, "ref": "feature"}));
    fake.content("governance/owners.toml", BASE, OWNERS);
    fake.pull_file(
        1,
        "docs/notes.md",
        "modified",
        1,
        0,
        Some("@@ -0,0 +1,1 @@\n+x"),
    );
    fake.issue(REG, "bosun registry", &["bosun:registry"], "");
    join(&fake, "session_orch", "spawned", None, 1);
    fake.status(HEAD, "review-verdict", "failure", ESCALATED);
    app_comment(&fake, "ada", &chat(HEAD, "session_orch"));
    let (code, v) = check(&fake);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["level"], "normal", "{v}");
    assert_eq!(v["data"]["verdict_cleared_by"], "ada", "{v}");
    assert_eq!(last_status(&fake, "review-verdict")["state"], "success");
}

#[test]
fn chat_after_orchestrator_left_not_counted() {
    // Fail closed: the registry is read at check time, so a Leave of the session makes its records stop counting.
    let fake = high_pr(true);
    app_comment(&fake, "ada", &chat(HEAD, "session_orch"));
    assert_eq!(approved(&fake), "ada");
    fake.comment(
        REG,
        r#"{"bosun":"leave","session":"session_orch","reason":"done"}"#,
    );
    refused(&fake);
}

#[test]
fn chat_counts_under_owner_gated_and_autonomous_policy() {
    for policy in ["owner-gated", "autonomous"] {
        let fake = high_pr(true);
        let cfg = format!("[approvals]\npolicy = \"{policy}\"\n");
        fake.content(".bosun/bosun.toml", BASE, &cfg);
        app_comment(&fake, "ada", &chat(HEAD, "session_orch"));
        assert_eq!(approved(&fake), "ada", "{policy}");
    }
}

#[test]
fn chat_never_counts_on_trust_file_pr() {
    let fake = high_pr(true);
    let patch = Some("@@ -1,1 +1,2 @@\n [[owner]]\n+github = \"eve\"");
    fake.pull_file(1, "governance/owners.toml", "modified", 1, 0, patch);
    app_comment(&fake, "ada", &chat(HEAD, "session_orch"));
    refused(&fake);
    // The owner's own GitHub comment still counts there.
    fake.comment_by(1, "ada", &format!("/bosun approve {HEAD}"));
    assert_eq!(approved(&fake), "ada");
}

#[test]
fn chat_never_counts_when_the_diff_is_unseen() {
    let fake = high_pr(true);
    // A patch-less file the rules cannot read: R-unseen.
    fake.pull_file(1, "src/big.rs", "modified", 1, 0, None);
    app_comment(&fake, "ada", &chat(HEAD, "session_orch"));
    let (code, v) = check(&fake);
    assert_eq!(code, 5, "{v}");
    assert!(v["data"]["matches"].to_string().contains("R-unseen"), "{v}");
    assert_eq!(v["data"]["approved_by"], Value::Null, "{v}");
}

#[test]
fn chat_record_for_another_pr_not_counted() {
    let fake = high_pr(true);
    let other = chat(HEAD, "session_orch").replace("question=1-", "question=2-");
    app_comment(&fake, "ada", &other);
    refused(&fake);
}

#[test]
fn chat_judged_by_the_join_in_force_at_record_time() {
    // The orchestrator rejoined (a new comment) after the record: the earlier join was in force.
    let fake = high_pr(true);
    app_comment(&fake, "ada", &chat(HEAD, "session_orch"));
    join(&fake, "session_orch", "spawned", None, 1);
    assert_eq!(approved(&fake), "ada");
    // A join only after the record does not vouch for it.
    let fake = FakeGithub::start();
    fake.pull(1, "change", "feature", "");
    fake.pull_as_issue(1, "change");
    fake.pull_set(1, "head", json!({"sha": HEAD, "ref": "feature"}));
    fake.content("governance/owners.toml", BASE, OWNERS);
    let patch = Some("@@ -0,0 +1,1 @@\n+x");
    fake.pull_file(1, ".github/workflows/ci.yml", "modified", 1, 0, patch);
    fake.issue(REG, "bosun registry", &["bosun:registry"], "");
    app_comment(&fake, "ada", &chat(HEAD, "session_orch"));
    join(&fake, "session_orch", "spawned", None, 1);
    refused(&fake);
}

#[test]
fn a_lapsed_orchestrator_is_named_in_the_fix() {
    let fake = high_pr(true);
    app_comment(&fake, "ada", &chat(HEAD, "session_orch"));
    fake.comment(
        REG,
        r#"{"bosun":"leave","session":"session_orch","reason":"done"}"#,
    );
    let (code, v) = check(&fake);
    assert_eq!(code, 5, "{v}");
    let fix = v["error"]["fix"].as_str().unwrap();
    assert!(fix.contains("chat approval record"), "{fix}");
    // No record at all: the fix does not mention one.
    let (_, v) = check(&high_pr(true));
    assert!(
        !v["error"]["fix"]
            .as_str()
            .unwrap()
            .contains("chat approval record")
    );
}

#[test]
fn chat_record_withdrawn_by_newer_changes_requested() {
    let fake = high_pr(true);
    app_comment(&fake, "ada", &chat(HEAD, "session_orch"));
    assert_eq!(approved(&fake), "ada");
    fake.review(1, "ada", "CHANGES_REQUESTED", HEAD);
    refused(&fake);
}
