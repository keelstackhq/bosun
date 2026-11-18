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
    join(&fake, "session_orch", "local", None, 1);
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
    join(&fake, "session_orch", "local", None, 1);
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
    join(&fake, "session_orch", "local", None, 1);
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
    join(&fake, "session_orch", "local", None, 1);
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

// `[approvals] chat_signed`: a chat record counts only with an enrolled owner key's signature.
mod signed {
    use super::*;
    use bosun::core::approve::signed_message_chat;
    use bosun::github::b64;
    use ring::signature::{Ed25519KeyPair, KeyPair};

    const MAIN: &str = "refs/heads/main";
    const Q: &str = "1-0123456-1";
    /// 2026-10-22T10:00:00Z, when the fake posts its comments unless told otherwise.
    const AT: i64 = 1_792_663_200;

    fn kp(seed: u8) -> Ed25519KeyPair {
        Ed25519KeyPair::from_seed_unchecked(&[seed; 32]).unwrap()
    }

    fn setup(cfg: &str) -> FakeGithub {
        let fake = high_pr(true);
        // The default branch decides; the PR's base says the same here.
        for r in [BASE, MAIN] {
            fake.content(".bosun/bosun.toml", r, cfg);
        }
        let k = b64::encode(kp(1).public_key().as_ref());
        let keys = format!("[[key]]\nid = \"k1\"\ned25519 = \"{k}\"\n");
        fake.content(".bosun/approvers.toml", MAIN, &keys);
        fake
    }

    /// A chat record for HEAD signed as `message` says, by the key `seed` under id `id`.
    fn record(seed: u8, id: &str, repo: &str, pr: u64, head: &str) -> String {
        record_at(seed, id, (repo, pr, head), ("session_orch", AT))
    }

    /// As [`record`], signed for `session` at unix time `at`.
    fn record_at(
        seed: u8,
        id: &str,
        (repo, pr, head): (&str, u64, &str),
        (session, at): (&str, i64),
    ) -> String {
        let (o, r) = repo.split_once('/').unwrap();
        let msg = signed_message_chat((o, r, pr), head, (Q, session, at));
        let sig = b64::encode(kp(seed).sign(msg.as_bytes()).as_ref());
        format!(
            "{}\n<!-- bosun:chat-sig v1 key={id} at={at} sig={sig} -->",
            chat(HEAD, "session_orch")
        )
    }

    const ON: &str = "[approvals]\nchat_signed = true\n";

    #[test]
    fn unsigned_record_counts_unless_chat_signed() {
        for cfg in [
            "[approvals]\nchat = true\n",
            "[approvals]\nchat_signed = false\n",
        ] {
            let fake = setup(cfg);
            app_comment(&fake, "ada", &chat(HEAD, "session_orch"));
            assert_eq!(approved(&fake), "ada", "{cfg}");
        }
        let fake = setup(ON);
        app_comment(&fake, "ada", &chat(HEAD, "session_orch"));
        refused(&fake);
    }

    #[test]
    fn enrolled_key_signature_counts() {
        let fake = setup(ON);
        app_comment(&fake, "ada", &record(1, "k1", "acme/widget", 1, HEAD));
        assert_eq!(approved(&fake), "ada");
        // Case of the repo name does not matter.
        let fake = setup(ON);
        app_comment(&fake, "ada", &record(1, "k1", "Acme/Widget", 1, HEAD));
        assert_eq!(approved(&fake), "ada");
    }

    #[test]
    fn wrong_key_head_pr_or_repo_does_not_count() {
        for (seed, id, repo, pr, head) in [
            (2, "k1", "acme/widget", 1, HEAD),
            (2, "k2", "acme/widget", 1, HEAD),
            (1, "k1", "acme/widget", 1, OLD),
            (1, "k1", "acme/widget", 2, HEAD),
            (1, "k1", "acme/gadget", 1, HEAD),
        ] {
            let fake = setup(ON);
            app_comment(&fake, "ada", &record(seed, id, repo, pr, head));
            refused(&fake);
        }
    }

    #[test]
    fn two_signature_lines_do_not_count() {
        let fake = setup(ON);
        let one = record(1, "k1", "acme/widget", 1, HEAD);
        let line = one.lines().last().unwrap().to_string();
        app_comment(&fake, "ada", &format!("{one}\n{line}"));
        refused(&fake);
    }

    #[test]
    fn a_key_that_is_not_enrolled_on_main_does_not_count() {
        let fake = setup(ON);
        fake.content(".bosun/approvers.toml", MAIN, "");
        app_comment(&fake, "ada", &record(1, "k1", "acme/widget", 1, HEAD));
        refused(&fake);
    }

    fn keys_file(entries: &[(&str, u8, &str)]) -> String {
        (entries.iter())
            .map(|(id, seed, role)| {
                let k = b64::encode(kp(*seed).public_key().as_ref());
                format!("[[key]]\nid = \"{id}\"\ned25519 = \"{k}\"\nrole = \"{role}\"\n")
            })
            .collect()
    }

    #[test]
    fn delegate_and_signer_keys_do_not_count() {
        for role in ["delegate", "signer"] {
            let fake = setup(ON);
            fake.content(
                ".bosun/approvers.toml",
                MAIN,
                &keys_file(&[("k1", 1, role)]),
            );
            app_comment(&fake, "ada", &record(1, "k1", "acme/widget", 1, HEAD));
            refused(&fake);
        }
    }

    #[test]
    fn head_config_cannot_turn_chat_signed_off() {
        let fake = setup(ON);
        fake.content(
            ".bosun/bosun.toml",
            HEAD,
            "[approvals]\nchat_signed = false\n",
        );
        app_comment(&fake, "ada", &chat(HEAD, "session_orch"));
        refused(&fake);
    }

    #[test]
    fn signed_record_without_question_is_refused() {
        let fake = setup(ON);
        let signed = record(1, "k1", "acme/widget", 1, HEAD);
        app_comment(&fake, "ada", &signed.replace(" question=1-0123456-1", ""));
        refused(&fake);
    }

    #[test]
    fn duplicate_key_ids_count_for_none() {
        let fake = setup(ON);
        let dup = keys_file(&[("k1", 1, "owner"), ("k1", 2, "owner")]);
        fake.content(".bosun/approvers.toml", MAIN, &dup);
        app_comment(&fake, "ada", &record(1, "k1", "acme/widget", 1, HEAD));
        refused(&fake);
    }

    #[test]
    fn the_failure_names_the_unsigned_record() {
        let fake = setup(ON);
        app_comment(&fake, "ada", &chat(HEAD, "session_orch"));
        let (code, v) = check(&fake);
        assert_eq!(code, 5, "{v}");
        let fix = v["error"]["fix"].as_str().unwrap();
        assert!(fix.contains("it does not verify"), "{fix}");
        assert!(!fix.contains("orchestrator session left"), "{fix}");
        // A wrong-key signature reads the same: it is the signature, not the orchestrator.
        let fake = setup(ON);
        app_comment(&fake, "ada", &record(2, "k1", "acme/widget", 1, HEAD));
        let fix = fix_of(&fake);
        assert!(
            fix.contains("chat_signed") && fix.contains("does not verify"),
            "{fix}"
        );
        assert!(!fix.contains("orchestrator session left"), "{fix}");
    }

    /// The signature binds the record's session and its time: another session or time fails.
    #[test]
    fn a_signature_for_another_session_or_time_does_not_count() {
        let fake = setup(ON);
        let other = record_at(1, "k1", ("acme/widget", 1, HEAD), ("session_other", AT));
        app_comment(&fake, "ada", &other);
        refused(&fake);
        let fake = setup(ON);
        // Signed at one time, claiming another in the line.
        let moved =
            record(1, "k1", "acme/widget", 1, HEAD).replace(&AT.to_string(), &(AT + 1).to_string());
        app_comment(&fake, "ada", &moved);
        refused(&fake);
        // A line with no time (the v1 form) never counts for a PR chat record.
        let fake = setup(ON);
        let old = record(1, "k1", "acme/widget", 1, HEAD).replace(&format!(" at={AT}"), "");
        app_comment(&fake, "ada", &old);
        refused(&fake);
    }

    /// A signed record posted long after it was signed is stale: a copy of an old one never counts.
    #[test]
    fn a_stale_copy_of_a_signed_record_does_not_count() {
        let fake = setup(ON);
        let body = record(1, "k1", "acme/widget", 1, HEAD);
        fake.comment_at(1, "ada", &body, "2026-10-22T11:00:00Z");
        fake.last_comment(|c| c.via_app = Some("claude".into()));
        refused(&fake);
    }

    /// After the owner's request for changes is dismissed, only a record signed after the
    /// dismissal counts: neither the old record nor a copy of it posted later revives.
    #[test]
    fn a_replayed_record_never_revives_after_a_dismissal() {
        let fake = setup(ON);
        let old = record(1, "k1", "acme/widget", 1, HEAD);
        app_comment(&fake, "ada", &old);
        assert_eq!(approved(&fake), "ada");
        fake.review(1, "ada", "CHANGES_REQUESTED", HEAD);
        fake.review(1, "ada", "DISMISSED", HEAD);
        fake.dismiss_event(1, 2, "2026-10-22T10:30:00Z");
        refused(&fake);
        fake.comment_at(1, "ada", &old, "2026-10-22T10:35:00Z");
        fake.last_comment(|c| c.via_app = Some("claude".into()));
        refused(&fake);
        let fresh = record_at(
            1,
            "k1",
            ("acme/widget", 1, HEAD),
            ("session_orch", AT + 2160),
        );
        fake.comment_at(1, "ada", &fresh, "2026-10-22T10:36:00Z");
        fake.last_comment(|c| c.via_app = Some("claude".into()));
        assert_eq!(approved(&fake), "ada");
    }

    /// `chat_signed` is read from the default branch: the PR's base cannot turn it off.
    #[test]
    fn the_pr_base_cannot_turn_chat_signed_off() {
        let fake = setup(ON);
        fake.content(
            ".bosun/bosun.toml",
            BASE,
            "[approvals]\nchat_signed = false\n",
        );
        app_comment(&fake, "ada", &chat(HEAD, "session_orch"));
        refused(&fake);
    }
}

fn fix_of(fake: &FakeGithub) -> String {
    let (code, v) = check(fake);
    assert_eq!(code, 5, "{v}");
    v["error"]["fix"].as_str().unwrap().to_string()
}

#[test]
fn the_chat_note_names_the_cause() {
    // Withdrawn by the owner's review.
    let fake = high_pr(true);
    app_comment(&fake, "ada", &chat(HEAD, "session_orch"));
    fake.review(1, "ada", "CHANGES_REQUESTED", HEAD);
    let fix = fix_of(&fake);
    assert!(fix.contains("withdrawn by the owner's review"), "{fix}");
    assert!(
        !fix.contains("left or expired") && !fix.contains("unsigned"),
        "{fix}"
    );
    // The orchestrator left.
    let fake = high_pr(true);
    app_comment(&fake, "ada", &chat(HEAD, "session_orch"));
    fake.comment(
        REG,
        r#"{"bosun":"leave","session":"session_orch","reason":"done"}"#,
    );
    let fix = fix_of(&fake);
    assert!(fix.contains("left or expired"), "{fix}");
    assert!(
        !fix.contains("withdrawn") && !fix.contains("unsigned"),
        "{fix}"
    );
    // Posted by someone who is not an owner: unsigned.
    let fake = high_pr(true);
    app_comment(&fake, "mallory", &chat(HEAD, "session_orch"));
    let fix = fix_of(&fake);
    assert!(fix.contains("unsigned"), "{fix}");
    assert!(
        !fix.contains("withdrawn") && !fix.contains("left or expired"),
        "{fix}"
    );
}
