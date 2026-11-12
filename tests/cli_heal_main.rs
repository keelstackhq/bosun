//! `bosun heal` for a red main (`[mode] heal_main`): revert the one PR merged since green.
mod common;

use std::path::Path;
use std::process::Command;

use common::fake::FakeGithub;
use common::{bosun, one_object};
use serde_json::{Value, json};

const NOW: &str = "2026-11-12T12:00:00Z";
const ACTIONS: &str = "github-actions";
const BOT: &str = "github-actions[bot]";
const AUTO: &str = "[mode]\nname = \"steady\"\nbrake = \"off\"\nheal_main = \"revert-auto\"\n";
const PR_ONLY: &str = "[mode]\nname = \"steady\"\nbrake = \"off\"\n";
const OFF: &str = "[mode]\nname = \"steady\"\nbrake = \"off\"\nheal_main = \"off\"\n";
const BRAKED: &str = "[mode]\nname = \"steady\"\nheal_main = \"revert-auto\"\n";

fn git(dir: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .env("GIT_AUTHOR_NAME", "t")
        .env("GIT_AUTHOR_EMAIL", "t@t")
        .env("GIT_COMMITTER_NAME", "t")
        .env("GIT_COMMITTER_EMAIL", "t@t")
        .output()
        .unwrap();
    assert!(out.status.success(), "git {args:?}: {out:?}");
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

/// A bare origin and a clone: main has "initial", then PR 5 adds `broken`.
fn repo() -> (tempfile::TempDir, std::path::PathBuf, String) {
    let tmp = tempfile::tempdir().unwrap();
    let (origin, clone) = (tmp.path().join("o.git"), tmp.path().join("c"));
    std::fs::create_dir(&origin).unwrap();
    git(&origin, &["init", "-q", "--bare", "-b", "main"]);
    let url = format!("file://{}", origin.display());
    Command::new("git")
        .args(["clone", "-q", &url])
        .arg(&clone)
        .output()
        .unwrap();
    git(&clone, &["checkout", "-q", "-B", "main"]);
    let mut sha = String::new();
    for f in ["init.txt", "broken"] {
        std::fs::write(clone.join(f), "x\n").unwrap();
        git(&clone, &["add", "-A"]);
        git(&clone, &["commit", "-q", "-m", f]);
        sha = git(&clone, &["rev-parse", "HEAD"]);
    }
    git(&clone, &["push", "-q", "-u", "origin", "main"]);
    (tmp, clone, sha)
}

fn merged(fake: &FakeGithub, n: u64, sha: &str, ci: &str, at: &str) {
    fake.pull(n, &format!("PR {n}"), &format!("claude/{n}"), "");
    fake.pull_merged(n, sha);
    fake.pull_set(n, "merged_at", json!(at));
    fake.pull_set(n, "updated_at", json!(at));
    fake.pull_set(n, "changed_files", 1.into());
    fake.check_run_by(sha, "ci-ok", ci, ACTIONS);
    fake.pull_commit_shas(n, &[&format!("sha{n}")]);
    // The PR's own head failed too: that is what trips the brake.
    fake.check_run_by(
        &format!("sha{n}"),
        "ci-ok",
        if n == 5 { "failure" } else { "success" },
        ACTIONS,
    );
}

/// Red main; #4 merged green, #5 (adds `broken`) merged red.
fn world(sha5: &str, path5: &str) -> FakeGithub {
    let fake = FakeGithub::start();
    fake.issue(3, "main is red", &["bosun:main-red"], "");
    merged(&fake, 4, "good4", "success", "2026-11-10T10:00:00Z");
    merged(&fake, 5, sha5, "failure", "2026-11-11T10:00:00Z");
    fake.pull_file(5, path5, "added", 1, 0, Some("@@ -0,0 +1 @@\n+x"));
    fake
}

fn heal(fake: &FakeGithub, dir: &Path, toml: Option<&str>) -> Value {
    heal_args(fake, dir, toml, &[])
}

fn heal_args(fake: &FakeGithub, dir: &Path, toml: Option<&str>, args: &[&str]) -> Value {
    if let Some(t) = toml {
        std::fs::create_dir_all(dir.join(".bosun")).unwrap();
        std::fs::write(dir.join(".bosun/bosun.toml"), t).unwrap();
        fake.content(".bosun/bosun.toml", "main", t);
    }
    let out = bosun()
        .current_dir(dir)
        .env("BOSUN_GITHUB_API", fake.url())
        .env("BOSUN_NOW", NOW)
        .env("GIT_AUTHOR_NAME", "t")
        .env("GIT_AUTHOR_EMAIL", "t@t")
        .env("GIT_COMMITTER_NAME", "t")
        .env("GIT_COMMITTER_EMAIL", "t@t")
        .arg("heal")
        .args(args)
        .output()
        .unwrap();
    one_object(&out.stdout)
}

fn why(v: &Value) -> String {
    v["data"]["main"]["why"].as_str().unwrap_or("").to_string()
}

/// Make the revert PR `m` the exact inverse of #5 and give it a clean verdict on its head.
fn inverse_with_verdict(fake: &FakeGithub, m: u64, path: &str) {
    fake.pull_file(m, path, "removed", 0, 1, Some("@@ -1 +0,0 @@\n-x"));
    fake.pull_set(m, "changed_files", 1.into());
    let head = fake.pull_head(m);
    fake.pull_set(m, "commits", 1.into());
    fake.git_commit_message(&head, "Revert\n\nBosun-Session: s_open\n");
    let rec = json!({"bosun": "verdict", "session": "s_rev", "head_sha": head,
        "approved": true, "confidence": 0.95, "major": 0});
    fake.comment_by(m, BOT, &format!("Review verdict: approve.\n\n{rec}"));
}

#[test]
fn gatekeeper_output_is_unchanged() {
    let (_t, dir, sha) = repo();
    let v = heal(&world(&sha, "broken"), &dir, None);
    assert_eq!(v["data"], json!({"knob": "off", "brake": null, "prs": []}));
}

#[test]
fn heal_main_off_and_green_main_skip() {
    let (_t, dir, sha) = repo();
    let v = heal(&world(&sha, "broken"), &dir, Some(OFF));
    assert_eq!(v["data"]["main"]["action"], "skip", "{v}");
    assert_eq!(why(&v), "heal_main is off");
    let fake = world(&sha, "broken");
    fake.label(3, "x");
    fake.close_issue(3, "2026-11-12T11:00:00Z");
    let v = heal(&fake, &dir, Some(PR_ONLY));
    assert_eq!(why(&v), "main is green", "{v}");
}

#[test]
fn several_merges_since_green_are_not_reverted() {
    let (_t, dir, sha) = repo();
    let fake = world(&sha, "broken");
    merged(&fake, 6, "sha6", "failure", "2026-11-11T11:00:00Z");
    let v = heal(&fake, &dir, Some(PR_ONLY));
    assert_eq!(v["data"]["main"]["action"], "skip", "{v}");
    assert!(why(&v).contains("2 PRs merged since green"), "{v}");
}

#[test]
fn one_merge_since_green_opens_a_draft_revert() {
    let (_t, dir, sha) = repo();
    let fake = world(&sha, "broken");
    let v = heal(&fake, &dir, Some(PR_ONLY));
    let row = &v["data"]["main"];
    assert_eq!(
        (row["action"].as_str(), row["pr"].as_u64()),
        (Some("revert"), Some(5)),
        "{v}"
    );
    let m = row["revert_pr"].as_u64().unwrap();
    assert!(fake.pull_is_draft(m));
    // A rerun finds the open revert and does nothing more.
    let v = heal(&fake, &dir, Some(PR_ONLY));
    assert_eq!(why(&v), format!("revert PR #{m} is open"), "{v}");
}

#[test]
fn revert_auto_readies_a_clean_exact_inverse() {
    let (_t, dir, sha) = repo();
    let fake = world(&sha, "broken");
    let v = heal(&fake, &dir, Some(AUTO));
    let m = v["data"]["main"]["revert_pr"].as_u64().unwrap();
    inverse_with_verdict(&fake, m, "broken");
    let v = heal(&fake, &dir, Some(AUTO));
    assert_eq!(v["data"]["main"]["action"], "ready", "{v}");
    assert!(!fake.pull_is_draft(m));
    assert!(fake.auto_merge_enabled(m).is_some());
}

#[test]
fn revert_auto_skips_an_owner_class_target_or_a_missing_verdict() {
    let (_t, dir, sha) = repo();
    let fake = world(&sha, "governance/x.toml");
    let v = heal(&fake, &dir, Some(AUTO));
    let m = v["data"]["main"]["revert_pr"].as_u64().unwrap();
    inverse_with_verdict(&fake, m, "governance/x.toml");
    let v = heal(&fake, &dir, Some(AUTO));
    assert_eq!(v["data"]["main"]["action"], "skip", "{v}");
    assert!(why(&v).contains("#5 needed the owner"), "{v}");
    assert_eq!(fake.auto_merge_enabled(m), None);

    let (_t, dir, sha) = repo();
    let fake = world(&sha, "broken");
    let v = heal(&fake, &dir, Some(AUTO));
    let m = v["data"]["main"]["revert_pr"].as_u64().unwrap();
    fake.pull_file(m, "broken", "removed", 0, 1, Some("@@ -1 +0,0 @@\n-x"));
    fake.pull_set(m, "changed_files", 1.into());
    fake.pull_set(m, "commits", 1.into());
    fake.git_commit_message(&fake.pull_head(m), "Revert\n\nBosun-Session: s_open\n");
    let v = heal(&fake, &dir, Some(AUTO));
    assert!(why(&v).contains("independent verdict"), "{v}");
}

#[test]
fn the_brake_caps_revert_auto() {
    let (_t, dir, sha) = repo();
    let fake = world(&sha, "broken");
    let v = heal(&fake, &dir, Some(BRAKED));
    assert_eq!(v["data"]["brake"]["on"], true, "{v}");
    let m = v["data"]["main"]["revert_pr"].as_u64().unwrap();
    inverse_with_verdict(&fake, m, "broken");
    let v = heal(&fake, &dir, Some(BRAKED));
    assert_eq!(why(&v), format!("revert PR #{m} is open"), "{v}");
    assert_eq!(fake.auto_merge_enabled(m), None);
}

#[test]
fn revert_auto_needs_a_verdict_by_another_session_than_the_revert_author() {
    let (_t, dir, sha) = repo();
    let fake = world(&sha, "broken");
    let v = heal(&fake, &dir, Some(AUTO));
    let m = v["data"]["main"]["revert_pr"].as_u64().unwrap();
    inverse_with_verdict(&fake, m, "broken");
    let head = fake.pull_head(m);
    fake.git_commit_message(&head, "Revert\n\nBosun-Session: s_rev\n");
    let v = heal(&fake, &dir, Some(AUTO));
    assert!(why(&v).contains("independent verdict"), "{v}");
    fake.pull_set(m, "commits", 2.into());
    let v = heal(&fake, &dir, Some(AUTO));
    assert!(why(&v).contains("commits beyond the revert"), "{v}");
    assert_eq!(fake.auto_merge_enabled(m), None);
}

#[test]
fn heal_pr_scoped_to_one_pr_never_reverts() {
    let (_t, dir, sha) = repo();
    let fake = world(&sha, "broken");
    let v = heal_args(&fake, &dir, Some(AUTO), &["--pr", "99"]);
    assert_eq!(v["data"]["main"], Value::Null, "{v}");
}
