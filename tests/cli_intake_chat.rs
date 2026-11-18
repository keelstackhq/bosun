//! `bosun intake verify --chat <issue>`: the orchestrator's chat record of the owner's yes on
//! the intake issue stands in for the intake signature (spec §3, §8; plan 2026-11-10 T12).
mod common;

use bosun::core::approve::signed_message_chat_intake;
use bosun::core::intake::intake_sha256;
use bosun::github::b64;
use bosun::time::parse_rfc3339;
use common::fake::FakeGithub;
use ring::signature::{Ed25519KeyPair, KeyPair};
use serde_json::{Value, json};

const NOW: &str = "2026-11-10T10:00:00Z";
const LIVE: &str = "2026-11-10T11:00:00Z";
const MAIN: &str = "refs/heads/main";
const OWNERS: &str = "[[owner]]\ngithub = \"ada\"\n";
const ISSUE: u64 = 7;
const REG: u64 = 50;

const OK: &str = r#"version = 1
repo = { name = "widget", owner = "acme", visibility = "private" }
stack = ["rust"]
pieces = ["base", "rust-cli"]
bosun_pieces = ["claude-md"]
goal = "A CLI that does a thing"
publish = ["crates"]
approvals = "orchestrator"
notify = "chat"
slack_channel = ""
"#;

fn sha() -> String {
    intake_sha256(OK.as_bytes())
}

const Q: &str = "7-abcdef0-1";

fn key(seed: u8) -> Ed25519KeyPair {
    Ed25519KeyPair::from_seed_unchecked(&[seed; 32]).unwrap()
}

fn unsigned_mode(sha: &str, session: &str, mode: &str) -> String {
    format!(
        "/bosun approve-intake {sha} {mode} chat\nApproved by the owner in chat (session {session}, {NOW}).\n<!-- bosun:chat-approval v1 session={session} intake={sha} mode={mode} question={Q} -->"
    )
}

/// A record signed by key `seed` under the id `k1` (the key enrolled by `setup` is seed 1),
/// signing time `at` (RFC 3339).
fn signed_at(seed: u8, (sha, session, mode): (&str, &str, &str), at: &str) -> String {
    let t = parse_rfc3339(at).unwrap().0;
    let msg = signed_message_chat_intake(("acme", "widget", ISSUE), sha, mode, (Q, session, t));
    let sig = b64::encode(key(seed).sign(msg.as_bytes()).as_ref());
    format!(
        "{}\n<!-- bosun:chat-sig v1 key=k1 at={t} sig={sig} -->",
        unsigned_mode(sha, session, mode)
    )
}

fn signed_by(seed: u8, sha: &str, session: &str, mode: &str) -> String {
    signed_at(seed, (sha, session, mode), NOW)
}

fn record_at(sha: &str, session: &str, at: &str) -> String {
    signed_at(1, (sha, session, "new"), at)
}

fn record_mode(sha: &str, session: &str, mode: &str) -> String {
    signed_by(1, sha, session, mode)
}

fn record(sha: &str, session: &str) -> String {
    record_mode(sha, session, "new")
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

/// Intake issue #7, owners on main, registry with root `session_orch` and child `session_kid`.
fn setup(owners: bool) -> FakeGithub {
    let fake = FakeGithub::start();
    fake.issue(ISSUE, "intake: app", &["bosun:kind/plan"], "");
    if owners {
        fake.content("governance/owners.toml", MAIN, OWNERS);
    }
    let public = b64::encode(key(1).public_key().as_ref());
    let keys = format!("[[key]]\nid = \"k1\"\ned25519 = \"{public}\"\n");
    fake.content(".bosun/approvers.toml", MAIN, &keys);
    fake.issue(REG, "bosun registry", &["bosun:registry"], "");
    join(&fake, "session_orch", "local", None, 1);
    join(&fake, "session_kid", "spawned", Some("session_orch"), 2);
    fake
}

/// A record by `user` at `at` through the Claude app, as a cloud session posts it.
fn post(fake: &FakeGithub, user: &str, body: &str, at: &str) {
    fake.comment_at(ISSUE, user, body, at);
    fake.last_comment(|c| c.via_app = Some("claude".into()));
}

fn verify(fake: &FakeGithub, intake: &str, extra: &[&str]) -> (i32, Value) {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("intake.toml");
    std::fs::write(&file, intake).unwrap();
    let file = file.to_str().unwrap().to_string();
    let issue = ISSUE.to_string();
    let mut args = vec!["intake", "verify", &file, "--chat", &issue, "--mode", "new"];
    args.extend_from_slice(extra);
    common::run(fake, "session_ci", NOW, &args)
}

fn refused(fake: &FakeGithub, reason: &str) {
    let (code, v) = verify(fake, OK, &[]);
    assert_eq!(code, 5, "{v}");
    assert_eq!(v["data"]["reason"], reason, "{v}");
}

#[test]
fn chat_record_verifies_the_intake() {
    let fake = setup(true);
    post(
        &fake,
        "ada",
        &record_at(&sha(), "session_orch", "2026-11-10T09:30:00Z"),
        "2026-11-10T09:30:00Z",
    );
    let (code, v) = verify(&fake, OK, &[]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["command"], "intake verify");
    let d = &v["data"];
    assert_eq!(d["verified"], true, "{v}");
    assert_eq!(d["via"], "chat");
    assert_eq!(d["approved_by"], "ada");
    assert_eq!(d["session"], "session_orch");
    assert_eq!(d["repo"], "acme/widget");
    assert_eq!(d["issue"], ISSUE);
    assert_eq!(d["intake_sha256"], sha());
    assert_eq!(d["mode"], "new");
    assert_eq!(d["expires"], "2026-11-10T10:30:00Z");
    assert!(d["nonce"].as_str().unwrap().starts_with("chat-"), "{v}");
    assert_eq!(d["intake"]["repo"]["name"], "widget");
}

#[test]
fn no_record_or_other_intake_refused() {
    let fake = setup(true);
    refused(&fake, "no_chat_record");
    // A record for other intake bytes, and a PR-style record, do not count.
    let other = intake_sha256(b"other");
    post(&fake, "ada", &record(&other, "session_orch"), NOW);
    let pr = "/bosun approve 0123456789abcdef0123456789abcdef01234567 chat\n<!-- bosun:chat-approval v1 session=session_orch head=0123456789abcdef0123456789abcdef01234567 -->";
    post(&fake, "ada", pr, NOW);
    refused(&fake, "no_chat_record");
}

#[test]
fn edited_record_refused() {
    let fake = setup(true);
    post(&fake, "ada", &record(&sha(), "session_orch"), NOW);
    fake.last_comment(|c| c.updated_at = "2026-11-10T10:00:01Z".into());
    refused(&fake, "chat_edited");
}

#[test]
fn non_owner_author_refused() {
    let fake = setup(true);
    post(&fake, "bob", &record(&sha(), "session_orch"), NOW);
    refused(&fake, "chat_not_owner");
}

#[test]
fn child_or_wave_session_refused() {
    let fake = setup(true);
    post(&fake, "ada", &record(&sha(), "session_kid"), NOW);
    refused(&fake, "chat_not_orchestrator");
    let fake = setup(true);
    post(&fake, "ada", &record(&sha(), "session_orch/3"), NOW);
    refused(&fake, "chat_not_orchestrator");
}

#[test]
fn old_or_future_record_refused() {
    let fake = setup(true);
    post(
        &fake,
        "ada",
        &record_at(&sha(), "session_orch", "2026-11-10T08:59:59Z"),
        "2026-11-10T08:59:59Z",
    );
    refused(&fake, "expired");
    let fake = setup(true);
    post(
        &fake,
        "ada",
        &record_at(&sha(), "session_orch", "2026-11-10T10:01:01Z"),
        "2026-11-10T10:01:01Z",
    );
    refused(&fake, "issued_in_future");
    // Exactly an hour old still counts.
    let fake = setup(true);
    post(
        &fake,
        "ada",
        &record_at(&sha(), "session_orch", "2026-11-10T09:00:00Z"),
        "2026-11-10T09:00:00Z",
    );
    assert_eq!(verify(&fake, OK, &[]).0, 0);
}

#[test]
fn newest_counting_record_wins_over_a_bad_one() {
    let fake = setup(true);
    post(
        &fake,
        "ada",
        &record_at(&sha(), "session_orch", "2026-11-10T09:50:00Z"),
        "2026-11-10T09:50:00Z",
    );
    post(
        &fake,
        "bob",
        &record_at(&sha(), "session_orch", "2026-11-10T09:55:00Z"),
        "2026-11-10T09:55:00Z",
    );
    let (code, v) = verify(&fake, OK, &[]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["approved_by"], "ada");
}

#[test]
fn chat_off_on_main_refused() {
    let fake = setup(true);
    fake.content(
        ".bosun/bosun.toml",
        MAIN,
        "project = \"x\"\nstacks = [\"rust\"]\n\n[approvals]\nchat = false\n",
    );
    post(&fake, "ada", &record(&sha(), "session_orch"), NOW);
    refused(&fake, "chat_off");
}

#[test]
fn no_owners_file_user_repo_owner_counts() {
    let fake = setup(false);
    fake.repo_set("owner", json!({"login": "ada", "type": "User"}));
    post(&fake, "ada", &record(&sha(), "session_orch"), NOW);
    assert_eq!(verify(&fake, OK, &[]).0, 0);
    let fake = setup(false);
    fake.repo_set("owner", json!({"login": "ada", "type": "User"}));
    post(&fake, "bob", &record(&sha(), "session_orch"), NOW);
    refused(&fake, "chat_not_owner");
}

#[test]
fn bad_intake_refused_before_github() {
    let fake = setup(true);
    let (code, v) = verify(&fake, "version = 1\n", &[]);
    assert_eq!(code, 5, "{v}");
    assert_eq!(v["data"]["reason"], "bad_intake");
}

#[test]
fn sig_and_chat_conflict_and_one_is_required() {
    let fake = setup(true);
    let (code, v) = verify(&fake, OK, &["--sig", "x.sig"]);
    assert_eq!(code, 2, "{v}");
    let (code, v) = common::run(&fake, "s", NOW, &["intake", "verify", "intake.toml"]);
    assert_eq!(code, 2, "{v}");
    // --chat needs --mode.
    let args = ["intake", "verify", "intake.toml", "--chat", "7"];
    assert_eq!(common::run(&fake, "s", NOW, &args).0, 2);
}

#[test]
fn join_expired_at_comment_time_refused() {
    let fake = setup(true);
    post(
        &fake,
        "ada",
        &record_at(&sha(), "session_orch", "2026-11-10T11:30:00Z"),
        "2026-11-10T11:30:00Z",
    );
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("intake.toml");
    std::fs::write(&file, OK).unwrap();
    let (code, v) = common::run(
        &fake,
        "session_ci",
        "2026-11-10T11:30:00Z",
        &[
            "intake",
            "verify",
            file.to_str().unwrap(),
            "--chat",
            "7",
            "--mode",
            "new",
        ],
    );
    assert_eq!(code, 5, "{v}");
    assert_eq!(v["data"]["reason"], "chat_not_orchestrator", "{v}");
}

#[test]
fn owners_file_naming_nobody_gets_no_fallback() {
    let fake = setup(false);
    fake.content("governance/owners.toml", MAIN, "");
    fake.repo_set("owner", json!({"login": "ada", "type": "User"}));
    post(&fake, "ada", &record(&sha(), "session_orch"), NOW);
    assert_ne!(verify(&fake, OK, &[]).0, 0);
}

#[test]
fn record_for_the_other_mode_refused() {
    let fake = setup(true);
    post(
        &fake,
        "ada",
        &record_mode(&sha(), "session_orch", "adopt"),
        NOW,
    );
    refused(&fake, "chat_wrong_mode");
    // `--mode` given twice is a usage error, so verify the adopt side by its own call.
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("intake.toml");
    std::fs::write(&file, OK).unwrap();
    let args = [
        "intake",
        "verify",
        file.to_str().unwrap(),
        "--chat",
        "7",
        "--mode",
        "adopt",
    ];
    let (code, v) = common::run(&fake, "session_ci", NOW, &args);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["mode"], "adopt");
    // A record without a mode (the old form) never parses.
    let old = record(&sha(), "session_orch")
        .replace(" new chat", " chat")
        .replace(" mode=new", "");
    let fake = setup(true);
    post(&fake, "ada", &old, NOW);
    refused(&fake, "no_chat_record");
}

#[test]
fn fallback_is_judged_for_the_issue_author_not_the_commenter() {
    // User repo: the repo owner counts, whoever opened the issue.
    let fake = setup(false);
    fake.repo_set("owner", json!({"login": "ada", "type": "User"}));
    fake.issue_by(ISSUE, "bob");
    post(&fake, "ada", &record(&sha(), "session_orch"), NOW);
    assert_eq!(verify(&fake, OK, &[]).0, 0);
    // Org repo: the issue author needs admin; a commenter with admin who did not open it does not count.
    let fake = setup(false);
    fake.repo_set("owner", json!({"login": "acme", "type": "Organization"}));
    fake.issue_by(ISSUE, "ada");
    fake.permission("ada", "admin");
    fake.permission("bob", "admin");
    post(&fake, "bob", &record(&sha(), "session_orch"), NOW);
    refused(&fake, "chat_not_owner");
}

#[test]
fn org_repo_admin_issue_author_counts() {
    let fake = setup(false);
    fake.repo_set("owner", json!({"login": "acme", "type": "Organization"}));
    fake.issue_by(ISSUE, "ada");
    fake.permission("ada", "admin");
    post(&fake, "ada", &record(&sha(), "session_orch"), NOW);
    let (code, v) = verify(&fake, OK, &[]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["approved_by"], "ada");
    let fake = setup(false);
    fake.repo_set("owner", json!({"login": "acme", "type": "Organization"}));
    fake.issue_by(ISSUE, "ada");
    fake.permission("ada", "write");
    post(&fake, "ada", &record(&sha(), "session_orch"), NOW);
    refused(&fake, "chat_not_owner");
}

#[test]
fn a_failed_github_read_is_exit_1() {
    let fake = setup(true);
    post(&fake, "ada", &record(&sha(), "session_orch"), NOW);
    fake.fail_path("/comments", 500);
    let (code, v) = verify(&fake, OK, &[]);
    assert_eq!(code, 1, "{v}");
    assert_eq!(v["ok"], false);
}

#[test]
fn missing_registry_issue_is_not_an_orchestrator() {
    let fake = FakeGithub::start();
    fake.issue(ISSUE, "intake: app", &["bosun:kind/plan"], "");
    fake.content("governance/owners.toml", MAIN, OWNERS);
    post(&fake, "ada", &record(&sha(), "session_orch"), NOW);
    refused(&fake, "chat_not_orchestrator");
}

#[test]
fn only_a_valid_enrolled_signature_counts() {
    let sha = sha();
    // Unsigned, signed by an unenrolled key, signed for another mode or sha: none counts.
    for body in [
        unsigned_mode(&sha, "session_orch", "new"),
        signed_by(2, &sha, "session_orch", "new"),
        signed_by(1, &sha, "session_orch", "new").replace("sig=", "sig=AA"),
        signed_by(1, &sha, "session_orch", "adopt").replace("adopt", "new"),
        signed_by(1, &"ab".repeat(32), "session_orch", "new").replace(&"ab".repeat(32), &sha),
    ] {
        let fake = setup(true);
        post(&fake, "ada", &body, NOW);
        refused(&fake, "chat_unsigned");
    }
    // No approvers file at all: fail closed.
    let fake = setup(true);
    fake.content(".bosun/approvers.toml", MAIN, "");
    post(&fake, "ada", &record(&sha, "session_orch"), NOW);
    refused(&fake, "chat_unsigned");
    // `chat_signed = false` does not loosen it.
    let fake = setup(true);
    fake.content(
        ".bosun/bosun.toml",
        MAIN,
        "[approvals]\nchat_signed = false\n",
    );
    post(
        &fake,
        "ada",
        &unsigned_mode(&sha, "session_orch", "new"),
        NOW,
    );
    refused(&fake, "chat_unsigned");
}

#[test]
fn an_intake_for_another_repo_refused() {
    let other = OK.replace("widget", "other");
    let fake = setup(true);
    post(
        &fake,
        "ada",
        &record(&intake_sha256(other.as_bytes()), "session_orch"),
        NOW,
    );
    let (code, v) = verify(&fake, &other, &[]);
    assert_eq!(code, 5, "{v}");
    assert_eq!(v["data"]["reason"], "repo_mismatch", "{v}");
}

#[test]
fn a_later_owner_hold_revokes_and_a_later_record_restores() {
    let fake = setup(true);
    post(
        &fake,
        "ada",
        &record_at(&sha(), "session_orch", "2026-11-10T09:30:00Z"),
        "2026-11-10T09:30:00Z",
    );
    // A non-owner's hold changes nothing.
    post(&fake, "bob", "/bosun hold", "2026-11-10T09:31:00Z");
    assert_eq!(verify(&fake, OK, &[]).0, 0);
    post(&fake, "ada", "/bosun hold\nnot yet", "2026-11-10T09:32:00Z");
    refused(&fake, "chat_held");
    post(
        &fake,
        "ada",
        &record_at(&sha(), "session_orch", "2026-11-10T09:33:00Z"),
        "2026-11-10T09:33:00Z",
    );
    assert_eq!(verify(&fake, OK, &[]).0, 0);
    post(&fake, "ada", "/bosun no", "2026-11-10T09:34:00Z");
    refused(&fake, "chat_held");
    // Other words that merely start alike are not a hold.
    let fake = setup(true);
    post(
        &fake,
        "ada",
        &record_at(&sha(), "session_orch", "2026-11-10T09:30:00Z"),
        "2026-11-10T09:30:00Z",
    );
    post(&fake, "ada", "/bosun nothing", "2026-11-10T09:31:00Z");
    assert_eq!(verify(&fake, OK, &[]).0, 0);
}

#[test]
fn the_signing_time_is_bound_and_fresh() {
    let sha = sha();
    // Signed 20 minutes before it was posted (a replayed old signature), or with no time at all.
    let fake = setup(true);
    post(
        &fake,
        "ada",
        &signed_at(1, (&sha, "session_orch", "new"), "2026-11-10T09:30:00Z"),
        "2026-11-10T09:50:00Z",
    );
    refused(&fake, "chat_unsigned");
    let fake = setup(true);
    let body = record_at(&sha, "session_orch", NOW).replace(&format!(" at={}", unix(NOW)), "");
    post(&fake, "ada", &body, NOW);
    refused(&fake, "chat_unsigned");
    // The session is signed: another session name in the marker does not verify.
    let fake = setup(true);
    let body =
        record_at(&sha, "session_kid", NOW).replace("session=session_kid", "session=session_orch");
    post(&fake, "ada", &body, NOW);
    refused(&fake, "chat_unsigned");
    // Signed before the newest owner hold: dead, though close to its posting time.
    let fake = setup(true);
    post(&fake, "ada", "/bosun hold", "2026-11-10T09:40:00Z");
    post(
        &fake,
        "ada",
        &record_at(&sha, "session_orch", "2026-11-10T09:35:00Z"),
        "2026-11-10T09:44:00Z",
    );
    refused(&fake, "chat_unsigned");
}

fn unix(at: &str) -> i64 {
    parse_rfc3339(at).unwrap().0
}

#[test]
fn hold_matching_is_forgiving() {
    for hold in [
        "/BOSUN HOLD",
        "  /bosun   hold  ",
        "**/bosun hold**",
        "> /bosun no",
        "`/bosun hold`",
        "/bosun hold.",
        "/bosun no!",
        "\n\n/Bosun No, not yet",
    ] {
        let fake = setup(true);
        post(
            &fake,
            "ada",
            &record_at(&sha(), "session_orch", "2026-11-10T09:30:00Z"),
            "2026-11-10T09:30:00Z",
        );
        post(&fake, "ada", hold, "2026-11-10T09:31:00Z");
        refused(&fake, "chat_held");
    }
    for other in [
        "/bosun nothing",
        "/bosun holdup",
        "/bosunhold",
        "bosun hold",
    ] {
        let fake = setup(true);
        post(
            &fake,
            "ada",
            &record_at(&sha(), "session_orch", "2026-11-10T09:30:00Z"),
            "2026-11-10T09:30:00Z",
        );
        post(&fake, "ada", other, "2026-11-10T09:31:00Z");
        assert_eq!(verify(&fake, OK, &[]).0, 0, "{other}");
    }
}

#[test]
fn an_edited_hold_counts_by_its_current_text_and_time() {
    let at = "2026-11-10T09:30:00Z";
    let rec = || record_at(&sha(), "session_orch", at);
    // A hold edited away to something else no longer holds.
    let fake = setup(true);
    post(&fake, "ada", &rec(), at);
    post(&fake, "ada", "/bosun hold", "2026-11-10T09:31:00Z");
    fake.last_comment(|c| c.body = "never mind".into());
    assert_eq!(verify(&fake, OK, &[]).0, 0);
    // A comment written before the record and edited into a hold after it holds.
    let fake = setup(true);
    post(&fake, "ada", "thinking", "2026-11-10T09:20:00Z");
    fake.last_comment(|c| {
        c.body = "/bosun hold".into();
        c.updated_at = "2026-11-10T09:32:00Z".into();
    });
    post(&fake, "ada", &rec(), at);
    refused(&fake, "chat_held");
    // The same hold, never edited, before the record: the record stands.
    let fake = setup(true);
    post(&fake, "ada", "/bosun hold", "2026-11-10T09:20:00Z");
    post(&fake, "ada", &rec(), at);
    assert_eq!(verify(&fake, OK, &[]).0, 0);
}
