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

/// A base commit (the config) and a head commit (`files` on top): `merge` fetches both.
fn repo(files: &[(&str, &str)]) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    git(dir.path(), &["init", "-q", "-b", "claude/t7"]);
    std::fs::create_dir_all(dir.path().join(".bosun")).unwrap();
    std::fs::write(dir.path().join(".bosun/bosun.toml"), NO_CI).unwrap();
    git(dir.path(), &["add", "-A"]);
    git(dir.path(), &["commit", "-q", "-m", "base"]);
    for (path, text) in files {
        let p = dir.path().join(path);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, text).unwrap();
    }
    git(dir.path(), &["add", "-A"]);
    git(dir.path(), &["commit", "-q", "--allow-empty", "-m", "head"]);
    dir
}

fn base_sha(dir: &Path) -> String {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(["rev-parse", "HEAD~1"])
        .output()
        .unwrap();
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

/// Task 7 in review with PR 20 by `session_w`; `ci` is the base config; `high` adds a
/// workflow change and an owner. `session_orch` is a root session in the registry.
fn setup(high: bool, ci: Option<&str>) -> (FakeGithub, tempfile::TempDir) {
    setup_files(high, ci, &[])
}

fn setup_files(
    high: bool,
    ci: Option<&str>,
    files: &[(&str, &str)],
) -> (FakeGithub, tempfile::TempDir) {
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
    let dir = repo(files);
    stamps::ready_all(&fake, dir.path());
    // The PR's base is a real commit of the checkout, as in production.
    let base = base_sha(dir.path());
    fake.pull_set(20, "base", json!({"sha": base, "ref": "main"}));
    if let Some(c) = ci {
        fake.content(".bosun/bosun.toml", &base, c);
    }
    if high {
        fake.content(
            "governance/owners.toml",
            &base,
            "[[owner]]\ngithub = \"ada\"\n",
        );
    }
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
    fake.comment_by(
        20,
        "ada",
        &format!(
            "/bosun approve {head} chat\nApproved by the owner in chat.\n<!-- bosun:chat-approval v1 session={ORCH} head={head} question=20-{short}-1 -->"
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

/// Stamp files are never trusted: with none at all the gates still run, on the head, and pass.
#[test]
fn merge_ignores_stamps_and_runs_the_gates() {
    let (fake, dir) = setup(false, Some(NO_CI));
    verdict(&fake);
    assert_eq!(ready(&fake, dir.path()).0, 0);
    std::fs::remove_dir_all(stamps::stamp_dir(dir.path())).unwrap();
    let (code, v) = merge(&fake, dir.path());
    assert_eq!(code, 0, "{v}");
    assert!(fake.merged(20));
}

/// A tiny base profile: `fmt` fails when `bad.txt` exists, `lint` always passes.
const TINY: &str = "[adapters]\npackages = \"cargo-metadata\"\npending = \"rust-ignore\"\nassertions = \"rust\"\n[gates.fmt]\ncmd = [\"sh\", \"-c\", \"test ! -e bad.txt\"]\n[gates.lint]\ncmd = [\"true\"]\n";
/// The head's own profile: every gate passes. It must never be the one that judges.
const LAX: &str = "[adapters]\npackages = \"cargo-metadata\"\npending = \"rust-ignore\"\nassertions = \"rust\"\n[gates.fmt]\ncmd = [\"true\"]\n";

fn tiny_base(fake: &FakeGithub) {
    fake.content(".bosun/profiles/rust.toml", "refs/heads/main", TINY);
}

fn worktrees(dir: &Path) -> String {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(["worktree", "list"])
        .output()
        .unwrap();
    String::from_utf8_lossy(&out.stdout).to_string()
}

/// A failing required gate on the exact head: exit 5 naming it, nothing merged, worktree gone.
/// The head's profile says `true` and every stamp is present; the base's definition judges.
#[test]
fn merge_refuses_a_failing_gate_run_in_a_worktree_of_the_head() {
    let files = [("bad.txt", "x"), (".bosun/profiles/rust.toml", LAX)];
    let (fake, dir) = setup_files(false, Some(NO_CI), &files);
    tiny_base(&fake);
    verdict(&fake);
    // The summary and verdict are fine; only the gates decide.
    assert_eq!(ready(&fake, dir.path()).0, 0);
    let (code, v) = merge(&fake, dir.path());
    assert_eq!(code, 5, "{v}");
    assert_eq!(v["data"]["failed_gates"], json!(["fmt"]), "{v}");
    assert!(merge_puts(&fake).is_empty());
    assert_eq!(
        worktrees(dir.path()).lines().count(),
        1,
        "{}",
        worktrees(dir.path())
    );
}

/// The same head passes when the gate it fails is not there: the worktree run is real.
#[test]
fn merge_passes_when_the_gates_pass_in_the_worktree() {
    let (fake, dir) = setup_files(false, Some(NO_CI), &[("ok.txt", "x")]);
    tiny_base(&fake);
    verdict(&fake);
    assert_eq!(ready(&fake, dir.path()).0, 0);
    let (code, v) = merge(&fake, dir.path());
    assert_eq!(code, 0, "{v}");
    let puts = merge_puts(&fake);
    assert_eq!(puts[0]["sha"], fake.pull_head(20).as_str());
}

/// What must pass is the default branch's: the head drops `lint` from its own profile, the
/// base keeps it, so the missing lint stamp is exit 5 at `--ready`.
#[test]
fn ready_reads_the_required_gates_from_the_default_branch() {
    let (fake, dir) = setup_files(false, Some(NO_CI), &[(".bosun/profiles/rust.toml", LAX)]);
    tiny_base(&fake);
    verdict(&fake);
    std::fs::remove_file(stamps::stamp_dir(dir.path()).join("lint")).unwrap();
    let (code, v) = ready(&fake, dir.path());
    assert_eq!(code, 5, "{v}");
    assert_eq!(v["data"]["missing_gates"], json!(["lint"]), "{v}");
}

/// Merge refuses an unsigned verdict carrying a forged session tag, and takes the App's record.
#[test]
fn merge_counts_only_app_or_signed_verdicts() {
    let (fake, dir) = setup(false, Some(NO_CI));
    verdict(&fake);
    assert_eq!(ready(&fake, dir.path()).0, 0);
    // Replace the signed verdict by an unsigned one: only the ready summary remains.
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

#[test]
fn merge_child_session_env_is_refused_whatever_the_origin() {
    let (fake, dir) = setup(false, Some(NO_CI));
    verdict(&fake);
    assert_eq!(ready(&fake, dir.path()).0, 0);
    let out = bosun()
        .env("BOSUN_GITHUB_API", fake.url())
        .env("BOSUN_SESSION", ORCH)
        .env("BOSUN_ORIGIN", "local")
        .env("CLAUDE_CODE_CHILD_SESSION", "1")
        .env("BOSUN_NOW", NOW)
        .arg("-C")
        .arg(dir.path())
        .args(["merge", "20"])
        .output()
        .unwrap();
    assert_eq!(out.status.code().unwrap(), 5);
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

/// The head's `rust-toolchain.toml` (and other tool files) never reach the gates: the base has
/// none, so the overlay deletes it before `lint` checks that it is gone.
#[test]
fn merge_deletes_head_tool_files_the_base_lacks() {
    let files = [
        ("rust-toolchain.toml", "[toolchain]\npath = \"/fake\"\n"),
        (".cargo/config.toml", "x = 1\n"),
    ];
    let (fake, dir) = setup_files(false, Some(NO_CI), &files);
    let check = "[adapters]\npackages = \"cargo-metadata\"\npending = \"rust-ignore\"\nassertions = \"rust\"\n[gates.lint]\ncmd = [\"sh\", \"-c\", \"test ! -e rust-toolchain.toml && test ! -e .cargo/config.toml\"]\n";
    fake.content(".bosun/profiles/rust.toml", "refs/heads/main", check);
    verdict(&fake);
    assert_eq!(ready(&fake, dir.path()).0, 0);
    let (code, v) = merge(&fake, dir.path());
    assert_eq!(code, 0, "{v}");
    // A base that has the file gets exactly the base's text.
    let (fake, dir) = setup_files(false, Some(NO_CI), &files);
    let want = "[adapters]\npackages = \"cargo-metadata\"\npending = \"rust-ignore\"\nassertions = \"rust\"\n[gates.lint]\ncmd = [\"sh\", \"-c\", \"grep -q real rust-toolchain.toml\"]\n";
    fake.content(".bosun/profiles/rust.toml", "refs/heads/main", want);
    fake.content("rust-toolchain.toml", "refs/heads/main", "# real\n");
    verdict(&fake);
    assert_eq!(ready(&fake, dir.path()).0, 0);
    let (code, v) = merge(&fake, dir.path());
    assert_eq!(code, 0, "{v}");
}

/// A symlinked `.bosun` in the head would redirect the overlay: exit 5, the target untouched.
#[cfg(unix)]
#[test]
fn merge_refuses_a_symlinked_overlay_target() {
    let outside = tempfile::tempdir().unwrap();
    std::fs::write(outside.path().join("keep.txt"), "keep").unwrap();
    // The head replaces `.bosun` with a symlink to the outside directory.
    let (fake, dir) = setup(false, Some(NO_CI));
    let d = dir.path();
    git(d, &["rm", "-rq", ".bosun"]);
    std::os::unix::fs::symlink(outside.path(), d.join(".bosun")).unwrap();
    git(d, &["add", "-A"]);
    git(d, &["commit", "-q", "-m", "link"]);
    stamps::ready_all(&fake, d);
    verdict(&fake);
    let head = fake.pull_head(20);
    let summary = bosun::core::approve::ready_summary(&head, &[], "normal", "session_rev", None);
    fake.comment_by(20, "w", &summary);
    let (code, v) = merge(&fake, d);
    assert_eq!(code, 5, "{v}");
    assert!(
        v["error"]["message"].as_str().unwrap().contains("symlink"),
        "{v}"
    );
    let names: Vec<_> = std::fs::read_dir(outside.path())
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .collect();
    assert_eq!(names, ["keep.txt"], "{names:?}");
    assert!(merge_puts(&fake).is_empty());
}

/// A head or base that is not a 40-hex commit id is exit 5 before any git call.
#[test]
fn merge_refuses_a_head_that_is_not_40_hex() {
    let (fake, dir) = setup(false, Some(NO_CI));
    fake.pull_head_sha(20, "not-a-sha");
    verdict(&fake);
    let summary =
        bosun::core::approve::ready_summary("not-a-sha", &[], "normal", "session_rev", None);
    fake.comment_by(20, "w", &summary);
    let (code, v) = merge(&fake, dir.path());
    assert_eq!(code, 5, "{v}");
    assert!(
        v["error"]["message"].as_str().unwrap().contains("40-hex"),
        "{v}"
    );
    assert!(merge_puts(&fake).is_empty());
}

/// A base whose profiles require nothing but test-integrity cannot vouch for a head: exit 5.
#[test]
fn merge_refuses_a_base_that_requires_no_gates() {
    let (fake, dir) = setup(false, Some(NO_CI));
    let none = "[adapters]\npackages = \"cargo-metadata\"\npending = \"rust-ignore\"\nassertions = \"rust\"\n[gates.mutants]\ncmd = [\"true\"]\n";
    fake.content(".bosun/profiles/rust.toml", "refs/heads/main", none);
    verdict(&fake);
    // `--ready` itself refuses first; post the summary as a writer would.
    let head = fake.pull_head(20);
    let summary = bosun::core::approve::ready_summary(&head, &[], "normal", "session_rev", None);
    fake.comment_by(20, "w", &summary);
    let (code, v) = merge(&fake, dir.path());
    assert_eq!(code, 5, "{v}");
    assert!(
        v["error"]["message"]
            .as_str()
            .unwrap()
            .contains("test-integrity"),
        "{v}"
    );
    assert!(merge_puts(&fake).is_empty());
}

fn profile(fmt: &str, lint: &str) -> String {
    format!(
        "[adapters]\npackages = \"cargo-metadata\"\npending = \"rust-ignore\"\nassertions = \"rust\"\n[gates.fmt]\ncmd = [\"sh\", \"-c\", \"{fmt}\"]\n[gates.lint]\ncmd = [\"sh\", \"-c\", \"{lint}\"]\n"
    )
}

/// Gates execute the head's code, so they run last: an unapproved high-risk PR never reaches them.
#[test]
fn merge_runs_no_gate_before_the_owner_approves() {
    let marker = tempfile::tempdir().unwrap().keep().join("ran");
    let (fake, dir) = setup(true, Some(NO_CI));
    fake.content(
        ".bosun/profiles/rust.toml",
        "refs/heads/main",
        &profile(&format!("touch {}", marker.display()), "true"),
    );
    verdict(&fake);
    assert_eq!(ready(&fake, dir.path()).0, 5);
    let (code, v) = merge(&fake, dir.path());
    assert_eq!(code, 5, "{v}");
    assert!(!marker.exists(), "a gate ran before the owner approved");
    chat(&fake);
    let (code, v) = merge(&fake, dir.path());
    assert_eq!(code, 0, "{v}");
    assert!(marker.exists());
}

/// A gate that rewrites the base's tool config is caught after it ran: exit 5.
#[test]
fn merge_refuses_a_gate_that_rewrites_the_overlay() {
    let (fake, dir) = setup(false, Some(NO_CI));
    fake.content(
        ".bosun/profiles/rust.toml",
        "refs/heads/main",
        &profile("echo x > clippy.toml", "true"),
    );
    verdict(&fake);
    assert_eq!(ready(&fake, dir.path()).0, 0);
    let (code, v) = merge(&fake, dir.path());
    assert_eq!(code, 5, "{v}");
    assert!(
        v["error"]["message"].as_str().unwrap().contains("changed"),
        "{v}"
    );
    assert!(merge_puts(&fake).is_empty());
}

/// Tool files under a stack directory are the base's too.
#[test]
fn merge_overlays_tool_files_under_each_stack_dir() {
    let cfg = format!("{NO_CI}[[stack]]\npath = \"web\"\nprofile = \"rust\"\n");
    let files = [("web/rust-toolchain.toml", "[toolchain]\npath = \"/fake\"\n")];
    let (fake, dir) = setup_files(false, Some(&cfg), &files);
    fake.content(
        ".bosun/profiles/rust.toml",
        "refs/heads/main",
        &profile("true", "test ! -e rust-toolchain.toml"),
    );
    verdict(&fake);
    assert_eq!(ready(&fake, dir.path()).0, 0);
    let (code, v) = merge(&fake, dir.path());
    assert_eq!(code, 0, "{v}");
}
