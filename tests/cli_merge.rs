//! `bosun merge <pr>` (plan 2026-11-10 T10, spec D13 as amended by D-019): the orchestrator
//! re-checks the gates for the PR's current head and merges it with `sha = head`.
mod common;

use std::path::Path;
use std::process::Command;

use common::fake::FakeGithub;
use common::{bosun, lease_note, noci, one_object, stamps};
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
    fake.comment(
        REG,
        &format!(
            "{{\"bosun\":\"join\",\"session\":\"{ORCH}\",\"origin\":\"local\",\"depth\":1,\"expires\":\"{LIVE}\"}}"
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
    verdict_by(fake, "session_rev");
}

fn verdict_by(fake: &FakeGithub, session: &str) {
    noci::signed_verdict(fake, "rev", session);
}

fn chat(fake: &FakeGithub) {
    let head = fake.pull_head(20);
    let short = &head[..7];
    // Written at NOW (inside the join's live window), question id minted at NOW (unix 1794304800).
    fake.comment_at(
        20,
        "ada",
        &format!(
            "/bosun approve {head} chat\nApproved by the owner in chat.\n<!-- bosun:chat-approval v1 session={ORCH} head={head} question=20-{short}-1794304800 -->"
        ),
        NOW,
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

/// A stacked PR (base is not the default branch): exit 3, nothing merged, task unchanged.
#[test]
fn merge_stacked_base_refused() {
    let (fake, dir) = setup(false, None);
    green(&fake, None);
    fake.pull_set(20, "base", json!({"sha": BASE, "ref": "claude/other"}));
    let (code, v) = merge(&fake, dir.path());
    assert_eq!(code, 3, "{v}");
    assert!(merge_puts(&fake).is_empty());
    assert!(fake.labels_of(7).contains(&"bosun:state/review".into()));
}

/// No CI, a ready summary, but the only verdict is the orchestrator's own: exit 5.
#[test]
fn merge_no_ci_verdict_only_from_merger_refused() {
    let (fake, dir) = setup(false, Some(NO_CI));
    verdict_by(&fake, ORCH);
    // The author's ready accepts it (another session); the merger's own verdict does not count.
    assert_eq!(ready(&fake, dir.path()).0, 0);
    let (code, v) = merge(&fake, dir.path());
    assert_eq!(code, 5, "{v}");
    assert!(merge_puts(&fake).is_empty());
}

#[test]
fn merge_closed_unmerged_is_conflict() {
    let (fake, dir) = setup(false, None);
    fake.pull_set(20, "state", json!("closed"));
    assert_eq!(merge(&fake, dir.path()).0, 3);
    assert!(merge_puts(&fake).is_empty());
}

/// The token or proxy cannot merge: exit 5, the owner merges in the GitHub app.
#[test]
fn merge_forbidden_is_needs_human() {
    let (fake, dir) = setup(false, None);
    green(&fake, None);
    fake.fail_path("/pulls/20/merge", 403);
    let (code, v) = merge(&fake, dir.path());
    assert_eq!(code, 5, "{v}");
    assert!(
        v["error"]["fix"].as_str().unwrap().contains("GitHub app"),
        "{v}"
    );
}

#[test]
fn merge_spawned_origin_refused_even_as_orchestrator() {
    let (fake, dir) = setup(false, Some(NO_CI));
    verdict(&fake);
    assert_eq!(ready(&fake, dir.path()).0, 0);
    let out = bosun()
        .env("BOSUN_GITHUB_API", fake.url())
        .env("BOSUN_SESSION", ORCH)
        .env("BOSUN_ORIGIN", "spawned")
        .env("BOSUN_NOW", NOW)
        .arg("-C")
        .arg(dir.path())
        .args(["merge", "20"])
        .output()
        .unwrap();
    assert_eq!(out.status.code().unwrap(), 5);
    assert!(merge_puts(&fake).is_empty());
}

/// The orchestrator's checkout has no stamp for the head: a summary alone does not merge.
#[test]
fn merge_summary_without_local_stamps_is_exit5() {
    let (fake, dir) = setup(false, Some(NO_CI));
    verdict(&fake);
    assert_eq!(ready(&fake, dir.path()).0, 0);
    std::fs::remove_file(stamps::stamp_dir(dir.path()).join("lint")).unwrap();
    let (code, v) = merge(&fake, dir.path());
    assert_eq!(code, 5, "{v}");
    assert_eq!(v["data"]["missing_gates"], json!(["lint"]), "{v}");
    assert!(merge_puts(&fake).is_empty());
}

#[test]
fn merge_child_session_env_is_refused_whatever_the_origin() {
    let (fake, dir) = setup(false, Some(NO_CI));
    verdict(&fake);
    assert_eq!(ready(&fake, dir.path()).0, 0);
    for flag in ["1", "true", "yes"] {
        let out = bosun()
            .env("BOSUN_GITHUB_API", fake.url())
            .env("BOSUN_SESSION", ORCH)
            .env("BOSUN_ORIGIN", "local")
            .env("CLAUDE_CODE_CHILD_SESSION", flag)
            .env("BOSUN_NOW", NOW)
            .arg("-C")
            .arg(dir.path())
            .args(["merge", "20"])
            .output()
            .unwrap();
        assert_eq!(out.status.code().unwrap(), 5, "{flag}");
    }
    assert!(merge_puts(&fake).is_empty());
}

#[test]
fn merge_summary_from_a_non_writer_is_ignored() {
    let (fake, dir) = setup(false, Some(NO_CI));
    verdict(&fake);
    assert_eq!(ready(&fake, dir.path()).0, 0);
    let all = fake.comments_of(20);
    let c = all
        .iter()
        .find(|c| c.body.starts_with("/bosun ready "))
        .unwrap();
    fake.permission(&c.user, "read");
    let (code, v) = merge(&fake, dir.path());
    assert_eq!(code, 5, "{v}");
    assert!(merge_puts(&fake).is_empty());
}

#[test]
fn merge_405_is_conflict_and_a_merge_already_done_is_ok() {
    let (fake, dir) = setup(false, Some(NO_CI));
    verdict(&fake);
    assert_eq!(ready(&fake, dir.path()).0, 0);
    fake.fail_path("/pulls/20/merge", 405);
    let (code, v) = merge(&fake, dir.path());
    assert_eq!(code, 3, "{v}");
    assert!(!fake.merged(20));
    // The same 405, but the PR is merged at this head when it is re-read.
    fake.after_requests("/pulls/20", 1, move |s| {
        for p in s.pulls.iter_mut().filter(|p| p["number"] == 20) {
            p["merged"] = json!(true);
            p["state"] = json!("closed");
            p["merge_commit_sha"] = json!("m1");
        }
    });
    let (code, v) = merge(&fake, dir.path());
    assert_eq!(code, 0, "{v}");
}

/// The provider is the default branch's, read now: a PR whose own checkout says none, on a repo
/// whose default branch says github, takes the CI path (required checks), not the local one.
#[test]
fn merge_default_branch_github_head_none_takes_the_ci_path() {
    let (fake, dir) = setup(false, Some("[ci]\nprovider = \"github\"\n"));
    verdict(&fake);
    // `repo()` checks out a config that says none: the head's own claim.
    let (code, v) = merge(&fake, dir.path());
    assert_ne!(code, 0, "{v}");
    assert!(merge_puts(&fake).is_empty(), "{v}");
}

/// Merge counts only an enrolled key's signature as a verdict, never an App record.
#[test]
fn merge_counts_only_app_or_signed_verdicts() {
    let (fake, dir) = setup(false, Some(NO_CI));
    verdict(&fake);
    assert_eq!(ready(&fake, dir.path()).0, 0);
    fake.state
        .lock()
        .unwrap()
        .comments
        .retain(|c| !c.body.contains("bosun-sig"));
    let head = fake.pull_head(20);
    fake.comment_by(
        20,
        "rev",
        &noci::verdict_text(20, &head, "session_rev", "approve"),
    );
    let (code, v) = merge(&fake, dir.path());
    assert_eq!(code, 5, "{v}");
    assert!(merge_puts(&fake).is_empty());
    noci::app_verdict(&fake, "session_rev");
    assert_eq!(merge(&fake, dir.path()).0, 5);
}

/// What must pass is the default branch's: the checkout's profile drops `lint`, the base keeps it.
#[test]
fn merge_reads_the_required_gates_from_the_default_branch() {
    let (fake, dir) = setup(false, Some(NO_CI));
    let lax = "[adapters]\npackages = \"cargo-metadata\"\npending = \"rust-ignore\"\nassertions = \"rust\"\n[gates.fmt]\ncmd = [\"true\"]\n[gates.lint]\ncmd = [\"true\"]\n";
    fake.content(".bosun/profiles/rust.toml", "refs/heads/main", lax);
    verdict(&fake);
    assert_eq!(ready(&fake, dir.path()).0, 0);
    std::fs::remove_file(stamps::stamp_dir(dir.path()).join("test")).unwrap();
    // The base profile has no `test` gate: its absence is no gap, `deps` is not required either.
    let (code, v) = merge(&fake, dir.path());
    assert_eq!(code, 0, "{v}");
}

/// The identity-trusting commands refuse a `BOSUN_SESSION` that is not the session's own id or
/// `<own>/<task>` (exit 5, with a fix), before any GitHub call.
#[test]
fn identity_trusting_commands_refuse_a_foreign_session() {
    let dir = repo();
    let run = |session: &str, args: &[&str]| {
        let out = bosun()
            .current_dir(dir.path())
            .env("CLAUDE_CODE_REMOTE_SESSION_ID", "cse_own")
            .env("BOSUN_SESSION", session)
            .args(args)
            .output()
            .unwrap();
        (out.status.code().unwrap(), one_object(&out.stdout))
    };
    for args in [
        &["merge", "20"][..],
        &["approve", "20@abc", "--chat"],
        &["verdict", "post"],
        &["join", "--origin", "local"],
    ] {
        for bad in [
            "session_other",
            "session_other/12",
            "session_own/a/b",
            "session_own/",
        ] {
            let (code, v) = run(bad, args);
            assert_eq!(code, 5, "{args:?} {bad}: {v}");
            assert!(!v["error"]["fix"].as_str().unwrap().is_empty(), "{v}");
        }
        for ok in ["session_own", "session_own/12"] {
            let (code, v) = run(ok, args);
            assert_ne!(v["error"]["fix"].as_str().unwrap_or("").len(), 0, "{v}");
            let msg = v["error"]["message"].as_str().unwrap_or("");
            assert!(
                !msg.contains("is not this session"),
                "{args:?} {ok}: {v} {code}"
            );
        }
    }
}
