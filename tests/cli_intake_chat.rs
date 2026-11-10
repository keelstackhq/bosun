//! `bosun intake verify --chat <issue>`: the orchestrator's chat record of the owner's yes on
//! the intake issue stands in for the intake signature (spec §3, §8; plan 2026-11-10 T12).
mod common;

use bosun::core::intake::intake_sha256;
use common::fake::FakeGithub;
use serde_json::{Value, json};

const NOW: &str = "2026-11-10T10:00:00Z";
const LIVE: &str = "2026-11-10T11:00:00Z";
const MAIN: &str = "refs/heads/main";
const OWNERS: &str = "[[owner]]\ngithub = \"ada\"\n";
const ISSUE: u64 = 7;
const REG: u64 = 50;

const OK: &str = r#"version = 1
repo = { name = "app", owner = "keelstackhq", visibility = "private" }
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

fn record(sha: &str, session: &str) -> String {
    format!(
        "/bosun approve-intake {sha} chat\nApproved by the owner in chat (session {session}, {NOW}).\n<!-- bosun:chat-approval v1 session={session} intake={sha} question=7-abcdef0-1 -->"
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

/// Intake issue #7, owners on main, registry with root `session_orch` and child `session_kid`.
fn setup(owners: bool) -> FakeGithub {
    let fake = FakeGithub::start();
    fake.issue(ISSUE, "intake: app", &["bosun:kind/plan"], "");
    if owners {
        fake.content("governance/owners.toml", MAIN, OWNERS);
    }
    fake.issue(REG, "bosun registry", &["bosun:registry"], "");
    join(&fake, "session_orch", "spawned", None, 1);
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
    let mut args = vec!["intake", "verify", &file, "--chat", &issue];
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
        &record(&sha(), "session_orch"),
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
    assert_eq!(d["repo"], "keelstackhq/app");
    assert_eq!(d["issue"], ISSUE);
    assert_eq!(d["intake_sha256"], sha());
    assert_eq!(d["expires"], "2026-11-10T10:30:00Z");
    assert!(d["nonce"].as_str().unwrap().starts_with("chat-"), "{v}");
    assert_eq!(d["intake"]["repo"]["name"], "app");
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
        &record(&sha(), "session_orch"),
        "2026-11-10T08:59:59Z",
    );
    refused(&fake, "expired");
    let fake = setup(true);
    post(
        &fake,
        "ada",
        &record(&sha(), "session_orch"),
        "2026-11-10T10:01:01Z",
    );
    refused(&fake, "issued_in_future");
    // Exactly an hour old still counts.
    let fake = setup(true);
    post(
        &fake,
        "ada",
        &record(&sha(), "session_orch"),
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
        &record(&sha(), "session_orch"),
        "2026-11-10T09:50:00Z",
    );
    post(
        &fake,
        "bob",
        &record(&sha(), "session_orch"),
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
}
