//! `bosun pr` refuses a PR that edits files outside its session's lease (design C3).

mod common;

use std::process::Command;

use common::fake::FakeGithub;
use common::{bosun, one_object};
use serde_json::{Value, json};

const BRANCH: &str = "claude/t7";
const BUILDING: &str = "bosun:state/building";
const LEASE: &str = r#"{"bosun":"lease","session":"session_test","origin":"spawned","expires":"2026-10-22T10:30:00Z","touch":["src/cli/pr.rs","crates/a/Cargo.toml"]}"#;

fn repo() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    for args in [
        vec!["init", "-q", "-b", BRANCH],
        vec![
            "-c",
            "user.name=t",
            "-c",
            "user.email=t@t",
            "commit",
            "-q",
            "--allow-empty",
            "-m",
            "x",
        ],
    ] {
        let st = Command::new("git")
            .arg("-C")
            .arg(dir.path())
            .args(&args)
            .status()
            .unwrap();
        assert!(st.success());
    }
    dir
}

fn run_pr(fake: &FakeGithub, extra: &[&str]) -> (i32, Value) {
    let dir = repo();
    let mut args = vec!["pr", "7"];
    args.extend_from_slice(extra);
    let out = bosun()
        .env("BOSUN_GITHUB_API", fake.url())
        .arg("-C")
        .arg(dir.path())
        .args(&args)
        .output()
        .unwrap();
    (out.status.code().unwrap(), one_object(&out.stdout))
}

/// Task #7 in building, with or without this session's lease, no PR yet.
fn task(lease: bool) -> FakeGithub {
    let fake = FakeGithub::start();
    fake.issue(7, "the task", &[BUILDING], "Do it.");
    if lease {
        fake.comment(7, LEASE);
    }
    fake
}

/// Task #7 with an open PR #3 on the branch that lists `files` (name, status, previous name).
fn task_with_pr(files: &[(&str, &str, Option<&str>)], changed: u64) -> FakeGithub {
    let fake = task(true);
    fake.pull(3, "t", BRANCH, "Closes #7");
    fake.pull_set(3, "changed_files", json!(changed));
    for (name, status, previous) in files {
        fake.pull_file(3, name, status, 1, 0, Some("@@ -1 +1 @@"));
        if let Some(p) = previous {
            fake.renamed_from(3, p);
        }
    }
    fake
}

fn unchanged(fake: &FakeGithub) {
    let created = fake
        .requests()
        .iter()
        .any(|r| r.method == "POST" && r.path.ends_with("/pulls"));
    assert!(!created, "a PR was created");
    assert_eq!(fake.labels_of(7), vec![BUILDING]);
}

#[test]
fn a_compared_edit_outside_the_lease_is_refused_and_nothing_changes() {
    let fake = task(true);
    for f in ["src/cli/pr.rs", "src/cli/claim.rs", "src/core/zed.rs"] {
        fake.compare_file("main", BRANCH, f, "@@ -1 +1 @@");
    }
    let (code, v) = run_pr(&fake, &[]);
    assert_eq!(code, 3, "{v}");
    assert_eq!(
        v["error"]["message"],
        "#7 edits files outside its lease: src/cli/claim.rs, src/core/zed.rs"
    );
    let fix = v["error"]["fix"].as_str().unwrap();
    assert!(fix.contains("bosun claim 7 --touch <path>"), "{fix}");
    assert_eq!(
        v["data"]["outside"],
        json!(["src/cli/claim.rs", "src/core/zed.rs"])
    );
    unchanged(&fake);
}

#[test]
fn added_files_fragments_and_companion_lockfiles_pass() {
    let fake = task_with_pr(
        &[
            ("src/cli/pr.rs", "modified", None),
            ("src/brand_new.rs", "added", None),
            ("changelog.d/feat-x.md", "modified", None),
            ("docs/progress.d/feat-x.md", "removed", None),
            ("crates/a/Cargo.lock", "modified", None),
            ("Cargo.lock", "modified", None),
        ],
        6,
    );
    let (code, v) = run_pr(&fake, &[]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["touch_check"], "ok");
    assert_eq!(v["data"]["created"], false);
}

#[test]
fn a_rename_out_of_the_touch_is_refused() {
    let fake = task_with_pr(
        &[("src/cli/pr.rs", "renamed", Some("src/cli/old_name.rs"))],
        1,
    );
    let (code, v) = run_pr(&fake, &[]);
    assert_eq!(code, 3, "{v}");
    assert_eq!(v["data"]["outside"], json!(["src/cli/old_name.rs"]));
    assert_eq!(fake.labels_of(7), vec![BUILDING]);
}

#[test]
fn a_rename_into_the_touch_passes_as_an_added_file_and_a_copy_changes_nothing() {
    // The new path is effectively added; only the old name counts.
    let fake = task_with_pr(&[("src/other/new.rs", "renamed", Some("src/cli/pr.rs"))], 1);
    let (code, v) = run_pr(&fake, &[]);
    assert_eq!(code, 0, "{v}");
    let fake = task_with_pr(
        &[("src/other/new.rs", "copied", Some("src/other/old.rs"))],
        1,
    );
    let (code, v) = run_pr(&fake, &[]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["touch_check"], "ok");
}

#[test]
fn no_live_own_lease_skips_the_check() {
    let fake = task(false);
    fake.compare_file("main", BRANCH, "src/cli/claim.rs", "@@ -1 +1 @@");
    let (code, v) = run_pr(&fake, &[]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["touch_check"], "skipped (no lease)");
}

#[test]
fn a_truncated_diff_is_skipped_never_blocking() {
    let fake = task(true);
    for i in 0..300 {
        fake.compare_file("main", BRANCH, &format!("gen/f{i}.rs"), "@@ -1 +1 @@");
    }
    let (code, v) = run_pr(&fake, &[]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["touch_check"], "skipped (diff too large)");

    // The PR changed more files than it lists.
    let fake = task_with_pr(&[("src/cli/claim.rs", "modified", None)], 301);
    let (code, v) = run_pr(&fake, &[]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["touch_check"], "skipped (diff too large)");
}

#[test]
fn another_sessions_lease_skips_but_a_task_id_of_this_orchestrator_is_checked() {
    let other = LEASE.replace("session_test", "session_other");
    let fake = FakeGithub::start();
    fake.issue(7, "the task", &[BUILDING], "Do it.");
    fake.comment(7, &other);
    fake.compare_file("main", BRANCH, "src/cli/claim.rs", "@@ -1 +1 @@");
    let (code, v) = run_pr(&fake, &[]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["touch_check"], "skipped (no lease)");

    let wave = LEASE.replace("session_test", "session_test/t7");
    let fake = FakeGithub::start();
    fake.issue(7, "the task", &[BUILDING], "Do it.");
    fake.comment(7, &wave);
    fake.compare_file("main", BRANCH, "src/cli/claim.rs", "@@ -1 +1 @@");
    let (code, v) = run_pr(&fake, &[]);
    assert_eq!(code, 3, "{v}");
    assert_eq!(v["data"]["outside"], json!(["src/cli/claim.rs"]));
}

#[test]
fn an_unreadable_diff_is_skipped() {
    let fake = task(true);
    fake.fail_path("main...claude/t7", 500);
    let (code, v) = run_pr(&fake, &[]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["touch_check"], "skipped (diff unavailable)");
}

#[test]
fn an_own_lease_with_an_empty_touch_skips_the_check() {
    // A spawn's epic lease has `touch: []`; the edit hook treats it as unrestricted, so does pr.
    let empty = LEASE.replace(r#"["src/cli/pr.rs","crates/a/Cargo.toml"]"#, "[]");
    let fake = FakeGithub::start();
    fake.issue(7, "the task", &[BUILDING], "Do it.");
    fake.comment(7, &empty);
    fake.compare_file("main", BRANCH, "src/cli/claim.rs", "@@ -1 +1 @@");
    let (code, v) = run_pr(&fake, &[]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["touch_check"], "skipped (no lease)");
}
