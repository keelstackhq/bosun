//! `bosun plan pr <plan> "<why>" --source <s>`: intake, claim of the plan file and the PR in
//! one step (F14).

mod common;

use std::path::Path;
use std::process::Command;

use common::fake::FakeGithub;
use serde_json::Value;

const T0: &str = "2026-10-23T10:00:00Z";

const PLAN: &str = "# Plan: widget

project: widget

## M1: Lifecycle

area: lifecycle

### Tasks

- T1: State machine
  touch: src/core/state.rs

### Done when

- UC-M1-1: a task moves planned -> merged.
";

/// A git repo on `branch` with plans/widget.md committed.
fn repo(branch: &str) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("plans")).unwrap();
    std::fs::write(dir.path().join("plans/widget.md"), PLAN).unwrap();
    let git = |args: &[&str]| {
        let st = Command::new("git")
            .arg("-C")
            .arg(dir.path())
            .args(["-c", "user.name=t", "-c", "user.email=t@t"])
            .args(args)
            .status()
            .unwrap();
        assert!(st.success());
    };
    git(&["init", "-q", "-b", branch]);
    git(&["add", "."]);
    git(&["commit", "-q", "-m", "plan: add a flag"]);
    dir
}

fn plan_pr(fake: &FakeGithub, dir: &Path, extra: &[&str]) -> (i32, Value) {
    let mut a = vec![
        "-C",
        dir.to_str().unwrap(),
        "plan",
        "pr",
        "plans/widget.md",
        "add a --dry-run flag",
        "--source",
        "human",
    ];
    a.extend_from_slice(extra);
    common::run(fake, "session_test", T0, &a)
}

#[test]
fn one_step_opens_the_intake_claims_the_plan_and_opens_the_pr() {
    let fake = FakeGithub::start();
    let dir = repo("claude/plan-flag");
    let (code, v) = plan_pr(&fake, dir.path(), &[]);
    assert_eq!(code, 0, "{v}");
    let d = &v["data"];
    assert_eq!(d["intake"]["created"], true);
    let n = d["intake"]["issue"].as_u64().unwrap();
    assert_eq!(d["claim"]["issue"], n);
    assert_eq!(
        d["claim"]["lease"]["touch"],
        serde_json::json!(["plans/widget.md"])
    );
    assert_eq!(d["pr"]["created"], true);
    let pr = d["pr"]["pr"].as_u64().unwrap();
    assert!(
        fake.pull_body(pr).contains(&format!("#{n}")),
        "the PR links the intake"
    );
    let labels = fake.issue_state(n).labels;
    assert!(
        labels.contains(&"bosun:state/review".to_string()),
        "{labels:?}"
    );
    assert!(
        labels.contains(&"bosun:source/human".to_string()),
        "{labels:?}"
    );

    // Again: the same intake and PR, no new claim.
    let (code, v) = plan_pr(&fake, dir.path(), &[]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["intake"]["created"], false);
    assert_eq!(v["data"]["intake"]["issue"], n);
    assert_eq!(v["data"]["claim"], Value::Null);
    assert_eq!(v["data"]["pr"]["created"], false);
    assert_eq!(v["data"]["pr"]["pr"], pr);
}

#[test]
fn a_failed_step_names_the_steps_done() {
    let fake = FakeGithub::start();
    // On the default branch `bosun pr` refuses: the intake and the claim are kept.
    let dir = repo("main");
    let (code, v) = plan_pr(&fake, dir.path(), &[]);
    assert_eq!(code, 2, "{v}");
    assert!(v["data"]["intake"]["issue"].as_u64().is_some(), "{v}");
    assert!(v["data"]["claim"]["lease"].is_object(), "{v}");
    assert!(!v["error"]["fix"].as_str().unwrap().is_empty());
}

#[test]
fn a_missing_plan_is_a_usage_error_and_writes_nothing() {
    let fake = FakeGithub::start();
    let dir = repo("claude/x");
    let (code, v) = common::run(
        &fake,
        "session_test",
        T0,
        &[
            "-C",
            dir.path().to_str().unwrap(),
            "plan",
            "pr",
            "plans/nope.md",
            "why",
            "--source",
            "human",
        ],
    );
    assert_eq!(code, 2, "{v}");
    assert!(fake.requests().iter().all(|r| r.method == "GET"));
}

#[test]
fn auto_merge_passes_on_to_bosun_pr() {
    let fake = FakeGithub::start();
    let dir = repo("claude/plan-am");
    // The branch requires none of the checks: `bosun pr --auto-merge` refuses (exit 5).
    let (code, v) = plan_pr(&fake, dir.path(), &["--auto-merge"]);
    assert_eq!(code, 5, "{v}");
    assert!(v["data"]["claim"]["lease"].is_object(), "{v}");
}

fn plan_pr_with(fake: &FakeGithub, dir: &Path, plan: &str) -> (i32, Value) {
    common::run(
        fake,
        "session_test",
        T0,
        &[
            "-C",
            dir.to_str().unwrap(),
            "plan",
            "pr",
            plan,
            "why",
            "--source",
            "human",
        ],
    )
}

#[test]
fn a_plan_path_that_could_widen_the_touch_is_refused() {
    let fake = FakeGithub::start();
    let dir = repo("claude/x");
    for bad in [
        "**",
        "src/**",
        "plans/*.md",
        "plans/{a,b}.md",
        "plans/[ab].md",
        "plans/../src/main.md",
        "/etc/plans/widget.md",
        "plans/widget.txt",
        "widget.md",
        "./plans/widget.md",
    ] {
        let (code, v) = plan_pr_with(&fake, dir.path(), bad);
        assert_eq!(code, 2, "{bad}: {v}");
        assert!(!v["error"]["fix"].as_str().unwrap().is_empty());
        let msg = v["error"]["message"].as_str().unwrap();
        assert!(
            msg.contains("plan path"),
            "refused for the path, not a missing file: {msg}"
        );
    }
    assert!(
        fake.requests().iter().all(|r| r.method == "GET"),
        "nothing is written"
    );
}

#[test]
fn intake_plan_refuses_globs_dots_absolute_and_non_markdown() {
    let fake = FakeGithub::start();
    let dir = repo("claude/x");
    for bad in ["**", "src/**", "../plan.md", "/tmp/plan.md", "plans/widget"] {
        let (code, v) = common::run(
            &fake,
            "session_test",
            T0,
            &[
                "-C",
                dir.path().to_str().unwrap(),
                "intake",
                "why",
                "--source",
                "human",
                "--plan",
                bad,
            ],
        );
        assert_eq!(code, 2, "{bad}: {v}");
        let msg = v["error"]["message"].as_str().unwrap();
        assert!(
            msg.contains("plan path"),
            "refused for the path, not a missing file: {msg}"
        );
    }
    assert!(fake.requests().iter().all(|r| r.method == "GET"));
}
