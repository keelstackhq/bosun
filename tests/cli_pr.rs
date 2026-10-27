mod common;

use std::path::Path;
use std::process::Command;

use common::fake::FakeGithub;
use common::{bosun, one_object};
use serde_json::Value;

/// A git repo on `branch` with one commit.
fn repo(branch: &str) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    for args in [
        vec!["init", "-q", "-b", branch],
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

/// A repo on `branch` whose head commit has `subject`.
fn repo_with(branch: &str, subject: &str) -> tempfile::TempDir {
    let dir = repo(branch);
    let st = Command::new("git")
        .arg("-C")
        .arg(dir.path())
        .args(["-c", "user.name=t", "-c", "user.email=t@t"])
        .args(["commit", "-q", "--allow-empty", "-m", subject])
        .status()
        .unwrap();
    assert!(st.success());
    dir
}

fn bosun_in(fake: &FakeGithub, dir: &Path, args: &[&str]) -> (i32, Value) {
    let out = bosun()
        .env("BOSUN_GITHUB_API", fake.url())
        .arg("-C")
        .arg(dir)
        .args(args)
        .output()
        .unwrap();
    (out.status.code().unwrap(), one_object(&out.stdout))
}

fn task(fake: &FakeGithub, state: &str) {
    fake.issue(7, "the task", &[state], "Do it.");
}

fn merge_calls(fake: &FakeGithub) -> usize {
    fake.requests()
        .iter()
        .filter(|r| r.path.ends_with("/merge") || r.method == "PUT")
        .count()
}

#[test]
fn creates_the_pr_once_then_reuses_it() {
    let fake = FakeGithub::start();
    task(&fake, "bosun:state/building");
    let dir = repo("claude/t7");
    let (code, v) = bosun_in(&fake, dir.path(), &["pr", "7"]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["created"], true);
    let number = v["data"]["pr"].as_u64().unwrap();
    let (code, v2) = bosun_in(&fake, dir.path(), &["pr", "7", "--title", "ignored"]);
    assert_eq!(code, 0, "{v2}");
    assert_eq!(v2["data"]["created"], false);
    assert_eq!(v2["data"]["pr"], number);
    let creates = fake
        .requests()
        .iter()
        .filter(|r| r.method == "POST" && r.path.ends_with("/pulls"))
        .count();
    assert_eq!(creates, 1);
    let body = fake.pull_body(number);
    assert!(body.starts_with("Closes #7"));
    assert!(body.contains("<!-- bosun:task-pr 7 -->"));
    assert!(fake.labels_of(7).contains(&"bosun:state/review".into()));
}

#[test]
fn refuses_the_main_branch() {
    let fake = FakeGithub::start();
    task(&fake, "bosun:state/building");
    let dir = repo("main");
    let (code, v) = bosun_in(&fake, dir.path(), &["pr", "7"]);
    assert_eq!(code, 2, "{v}");
}

#[test]
fn auto_merge_is_refused_while_review_verdict_is_pending() {
    let fake = FakeGithub::start();
    task(&fake, "bosun:state/review");
    fake.pull(20, "t", "claude/t7", "Closes #7");
    fake.status("sha20", "ci-ok", "success", "");
    fake.status("sha20", "risk-check", "success", "");
    fake.status("sha20", "main-open", "success", "");
    fake.status("sha20", "review-verdict", "pending", "");
    let dir = repo("claude/t7");
    let (code, v) = bosun_in(&fake, dir.path(), &["pr", "7", "--auto-merge"]);
    assert_eq!(code, 1, "{v}");
    let msg = v["error"]["message"].as_str().unwrap();
    assert!(msg.contains("review-verdict: pending"), "{msg}");
    assert!(
        v["error"]["fix"]
            .as_str()
            .unwrap()
            .contains("bosun wait --pr 20")
    );
    assert_eq!(fake.auto_merge_enabled(20), None);
    assert!(fake.labels_of(7).contains(&"bosun:state/review".into()));
}

#[test]
fn auto_merge_is_enabled_when_all_four_are_green() {
    let fake = FakeGithub::start();
    task(&fake, "bosun:state/review");
    fake.pull(20, "t", "claude/t7", "Closes #7");
    fake.status("sha20", "risk-check", "success", "");
    fake.status("sha20", "main-open", "success", "");
    fake.status(
        "sha20",
        "review-verdict",
        "success",
        "approve 0.9, 0 blocker, 0 major",
    );
    fake.check_run("sha20", "ci-ok", "completed", Some("success"));
    let dir = repo("claude/t7");
    let (code, v) = bosun_in(&fake, dir.path(), &["pr", "7", "--auto-merge"]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["auto_merge"], true);
    assert_eq!(fake.auto_merge_enabled(20).as_deref(), Some("SQUASH"));
    assert!(fake.labels_of(7).contains(&"bosun:state/queued".into()));
    assert!(
        fake.requests()
            .iter()
            .any(|r| r.method == "POST" && r.path.ends_with("/graphql"))
    );
    assert_eq!(merge_calls(&fake), 0, "bosun must never merge");
}

const CCR_MSG: &str = "GitHub GraphQL is not available from Claude Code sessions; use the REST API (gh api repos/{owner}/{repo}/...).";

fn green_pr(fake: &FakeGithub) {
    task(fake, "bosun:state/review");
    fake.pull(20, "t", "claude/t7", "Closes #7");
    fake.status("sha20", "risk-check", "success", "");
    fake.status("sha20", "main-open", "success", "");
    fake.status("sha20", "review-verdict", "success", "approve 0.9");
    fake.check_run("sha20", "ci-ok", "completed", Some("success"));
}

fn ccr_puts(fake: &FakeGithub) -> Vec<common::fake::Logged> {
    fake.requests()
        .into_iter()
        .filter(|r| r.method == "PUT" && r.path.ends_with("/pulls/20/ccr/auto_merge"))
        .collect()
}

fn merge_paths(fake: &FakeGithub) -> usize {
    fake.requests()
        .iter()
        .filter(|r| r.path.ends_with("/merge"))
        .count()
}

#[test]
fn graphql_is_the_default_auto_merge_route() {
    let fake = FakeGithub::start();
    green_pr(&fake);
    let dir = repo("claude/t7");
    let (code, v) = bosun_in(&fake, dir.path(), &["pr", "7", "--auto-merge"]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["auto_merge_route"], "graphql");
    assert!(ccr_puts(&fake).is_empty());
    assert_eq!(merge_paths(&fake), 0);
}

#[test]
fn a_cloud_session_falls_back_to_the_ccr_route() {
    let fake = FakeGithub::start();
    green_pr(&fake);
    fake.graphql_status(403, CCR_MSG);
    let dir = repo("claude/t7");
    let (code, v) = bosun_in(&fake, dir.path(), &["pr", "7", "--auto-merge"]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["auto_merge_route"], "ccr");
    assert_eq!(v["data"]["task_state"], "queued");
    let puts = ccr_puts(&fake);
    assert_eq!(puts.len(), 1);
    let body: Value = serde_json::from_str(puts[0].body.as_deref().unwrap()).unwrap();
    assert_eq!(body["merge_method"], "squash");
    assert!(fake.labels_of(7).contains(&"bosun:state/queued".into()));
    assert_eq!(merge_paths(&fake), 0);
}

#[test]
fn another_graphql_403_is_not_retried_on_the_ccr_route() {
    let fake = FakeGithub::start();
    green_pr(&fake);
    fake.graphql_status(403, "Resource not accessible by integration");
    let dir = repo("claude/t7");
    let (code, v) = bosun_in(&fake, dir.path(), &["pr", "7", "--auto-merge"]);
    assert_eq!(code, 1, "{v}");
    assert!(ccr_puts(&fake).is_empty());
    assert!(fake.labels_of(7).contains(&"bosun:state/review".into()));
    assert_eq!(merge_paths(&fake), 0);
}

#[test]
fn a_missing_check_is_listed() {
    let fake = FakeGithub::start();
    task(&fake, "bosun:state/review");
    fake.pull(20, "t", "claude/t7", "Closes #7");
    let dir = repo("claude/t7");
    let (code, v) = bosun_in(&fake, dir.path(), &["pr", "7", "--auto-merge"]);
    assert_eq!(code, 1);
    assert_eq!(v["data"]["not_green"].as_array().unwrap().len(), 4);
    assert_eq!(v["data"]["not_green"][0]["state"], "missing");
}

#[test]
fn auto_merge_respects_the_kill_switch() {
    let fake = FakeGithub::start();
    task(&fake, "bosun:state/review");
    fake.variable("BOSUN_HALT", "1");
    let dir = repo("claude/t7");
    let (code, _) = bosun_in(&fake, dir.path(), &["pr", "7", "--auto-merge"]);
    assert_eq!(code, 4);
}

fn graphql_calls(fake: &FakeGithub) -> usize {
    fake.requests()
        .iter()
        .filter(|r| r.path.ends_with("/graphql"))
        .count()
}

#[test]
fn auto_merge_is_refused_unless_the_task_is_in_review() {
    for state in ["needs-human", "blocked", "queued"] {
        let fake = FakeGithub::start();
        task(&fake, &format!("bosun:state/{state}"));
        fake.pull(20, "t", "claude/t7", "Closes #7");
        for c in ["ci-ok", "risk-check", "review-verdict", "main-open"] {
            fake.status("sha20", c, "success", "");
        }
        let dir = repo("claude/t7");
        let (code, v) = bosun_in(&fake, dir.path(), &["pr", "7", "--auto-merge"]);
        assert_eq!(code, 3, "{state}: {v}");
        assert!(!v["error"]["fix"].as_str().unwrap().is_empty());
        assert_eq!(graphql_calls(&fake), 0, "{state}");
        assert_eq!(fake.auto_merge_enabled(20), None);
    }
}

#[test]
fn plain_pr_respects_the_kill_switch() {
    let fake = FakeGithub::start();
    task(&fake, "bosun:state/building");
    fake.variable("BOSUN_HALT", "1");
    let dir = repo("claude/t7");
    let (code, _) = bosun_in(&fake, dir.path(), &["pr", "7"]);
    assert_eq!(code, 4);
    assert!(
        !fake
            .requests()
            .iter()
            .any(|r| r.method == "POST" && r.path.ends_with("/pulls"))
    );
}

#[test]
fn the_pr_base_is_the_repos_default_branch() {
    let fake = FakeGithub::start();
    fake.repo_set("default_branch", serde_json::json!("trunk"));
    task(&fake, "bosun:state/building");
    let dir = repo("claude/t7");
    let (code, v) = bosun_in(&fake, dir.path(), &["pr", "7"]);
    assert_eq!(code, 0, "{v}");
    let body = fake
        .requests()
        .into_iter()
        .find(|r| r.method == "POST" && r.path.ends_with("/pulls"))
        .and_then(|r| r.body)
        .unwrap();
    assert!(body.contains(r#""base":"trunk""#), "{body}");
}

#[test]
fn the_default_branch_is_refused_whatever_its_name() {
    let fake = FakeGithub::start();
    fake.repo_set("default_branch", serde_json::json!("trunk"));
    task(&fake, "bosun:state/building");
    let dir = repo("trunk");
    let (code, _) = bosun_in(&fake, dir.path(), &["pr", "7"]);
    assert_eq!(code, 2);
}

fn green_four(fake: &FakeGithub) {
    for c in ["ci-ok", "risk-check", "main-open", "review-verdict"] {
        fake.status("sha20", c, "success", "");
    }
}

fn pr_setup(hoist: Option<&str>) -> (FakeGithub, tempfile::TempDir) {
    let fake = FakeGithub::start();
    task(&fake, "bosun:state/review");
    fake.pull(20, "t", "claude/t7", "Closes #7");
    green_four(&fake);
    let dir = repo("claude/t7");
    if let Some(h) = hoist {
        fake.content("hoist.toml", "main", h);
    }
    (fake, dir)
}

const HULL: &str = "[github]\nrequired_checks = [\"ci-ok\", \"bosun-ok\", \"risk-check\", \"review-verdict\", \"main-open\"]\n";

#[test]
fn hoist_toml_required_checks_are_enforced() {
    let (fake, dir) = pr_setup(Some(HULL));
    fake.status("sha20", "bosun-ok", "pending", "");
    let (code, v) = bosun_in(&fake, dir.path(), &["pr", "7", "--auto-merge"]);
    assert_eq!(code, 1, "{v}");
    assert!(
        v["error"]["message"]
            .as_str()
            .unwrap()
            .contains("bosun-ok: pending"),
        "{v}"
    );
    assert_eq!(fake.auto_merge_enabled(20), None);
}

#[test]
fn the_env_overrides_hoist_toml() {
    let (fake, dir) = pr_setup(Some(HULL));
    let out = bosun()
        .env("BOSUN_GITHUB_API", fake.url())
        .env("BOSUN_REQUIRED_CHECKS", "ci-ok")
        .arg("-C")
        .arg(dir.path())
        .args(["pr", "7", "--auto-merge"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(0), "{}", one_object(&out.stdout));
}

#[test]
fn a_missing_or_invalid_hoist_toml_gives_the_defaults() {
    for hoist in [
        None,
        Some("not = [toml"),
        Some("[github]\nrequired_checks = []\n"),
        Some("[github]\nrequired_checks = [\"ci-ok\"]\n"),
    ] {
        let (fake, dir) = pr_setup(hoist);
        let (code, v) = bosun_in(&fake, dir.path(), &["pr", "7", "--auto-merge"]);
        assert_eq!(code, 0, "{hoist:?} {v}");
    }
}

#[test]
fn a_pr_cannot_change_the_checks_that_judge_it() {
    // The PR branch's working tree lists bosun-ok (pending); the default branch does not.
    let (fake, dir) = pr_setup(None);
    fake.status("sha20", "bosun-ok", "pending", "");
    std::fs::write(dir.path().join("hoist.toml"), HULL).unwrap();
    let (code, v) = bosun_in(&fake, dir.path(), &["pr", "7", "--auto-merge"]);
    assert_eq!(code, 0, "{v}");
}

#[test]
fn an_open_branch_pr_that_does_not_link_the_task_is_refused() {
    for (body, flag) in [
        ("Closes #9", None),
        ("no link at all", None),
        ("Closes #9", Some("--auto-merge")),
        ("no link at all", Some("--auto-merge")),
    ] {
        let fake = FakeGithub::start();
        task(&fake, "bosun:state/building");
        fake.pull(12, "other", "claude/t7", body);
        for c in ["ci-ok", "risk-check", "review-verdict", "main-open"] {
            fake.status("sha12", c, "success", "");
        }
        let dir = repo("claude/t7");
        let mut args = vec!["pr", "7"];
        args.extend(flag);
        let (code, v) = bosun_in(&fake, dir.path(), &args);
        assert_eq!(code, 3, "{body} {flag:?}: {v}");
        assert!(!v["error"]["fix"].as_str().unwrap().is_empty());
        assert_eq!(v["data"]["pr"], 12, "{v}");
        assert_eq!(fake.labels_of(7), vec!["bosun:state/building".to_string()]);
        assert!(fake.comments_of(7).is_empty());
        assert_eq!(fake.auto_merge_enabled(12), None);
        assert_eq!(graphql_calls(&fake), 0);
        assert_eq!(merge_calls(&fake), 0);
    }
}

#[test]
fn an_open_branch_pr_that_links_the_task_in_any_form_is_used() {
    for body in [
        "Fixes #7",
        "resolves: #7\n\nmore",
        "<!-- bosun:task-pr 7 -->",
    ] {
        let fake = FakeGithub::start();
        task(&fake, "bosun:state/building");
        fake.pull(12, "t", "claude/t7", body);
        let dir = repo("claude/t7");
        let (code, v) = bosun_in(&fake, dir.path(), &["pr", "7"]);
        assert_eq!(code, 0, "{body}: {v}");
        assert_eq!(v["data"]["pr"], 12);
        assert_eq!(v["data"]["created"], false);
    }
}

fn created_title(fake: &FakeGithub) -> String {
    let body = fake
        .requests()
        .iter()
        .find(|r| r.method == "POST" && r.path.ends_with("/pulls"))
        .and_then(|r| r.body.clone())
        .expect("a PR was created");
    let v: Value = serde_json::from_str(&body).unwrap();
    v["title"].as_str().unwrap().to_string()
}

#[test]
fn the_pr_title_is_a_conventional_head_commit_subject() {
    let fake = FakeGithub::start();
    task(&fake, "bosun:state/building");
    let dir = repo_with("claude/t7", "fix(hello): trim the name");
    let (code, v) = bosun_in(&fake, dir.path(), &["pr", "7"]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(created_title(&fake), "fix(hello): trim the name");
}

#[test]
fn a_title_flag_beats_the_commit_subject() {
    let fake = FakeGithub::start();
    task(&fake, "bosun:state/building");
    let dir = repo_with("claude/t7", "fix: trim the name");
    let (code, v) = bosun_in(&fake, dir.path(), &["pr", "7", "--title", "chosen"]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(created_title(&fake), "chosen");
}

#[test]
fn a_non_conventional_subject_falls_back_to_the_issue_title() {
    let fake = FakeGithub::start();
    task(&fake, "bosun:state/building");
    let dir = repo_with("claude/t7", "Trim the name");
    let (code, v) = bosun_in(&fake, dir.path(), &["pr", "7"]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(created_title(&fake), "the task");
}

#[test]
fn an_unpushed_branch_says_to_push_it() {
    let fake = FakeGithub::start();
    task(&fake, "bosun:state/building");
    fake.branch_missing("claude/t7");
    let dir = repo("claude/t7");
    let (code, v) = bosun_in(&fake, dir.path(), &["pr", "7"]);
    assert_eq!(code, 1, "{v}");
    let fix = v["error"]["fix"].as_str().unwrap();
    assert!(fix.contains("git push -u origin claude/t7"), "{fix}");
    assert!(fix.contains("bosun pr 7"), "{fix}");
    assert!(
        !fake
            .requests()
            .iter()
            .any(|r| r.method == "POST" && r.path.ends_with("/pulls"))
    );
}

#[test]
fn an_existing_pr_is_reused_without_a_branch_check() {
    let fake = FakeGithub::start();
    green_pr(&fake);
    fake.branch_missing("claude/t7");
    let dir = repo("claude/t7");
    let (code, v) = bosun_in(&fake, dir.path(), &["pr", "7"]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["created"], false);
}

fn assert_auto_merge_off_fix(v: &Value) {
    let fix = v["error"]["fix"].as_str().unwrap();
    assert!(fix.contains("Allow auto-merge"), "{fix}");
    assert!(fix.contains("hoist apply"), "{fix}");
    assert!(!fix.contains("githubstatus"), "{fix}");
}

#[test]
fn a_graphql_error_saying_auto_merge_is_not_allowed_gets_the_fix() {
    let fake = FakeGithub::start();
    green_pr(&fake);
    fake.graphql_error("Pull request Auto merge is not allowed for this repository");
    let dir = repo("claude/t7");
    let (code, v) = bosun_in(&fake, dir.path(), &["pr", "7", "--auto-merge"]);
    assert_eq!(code, 1, "{v}");
    assert_auto_merge_off_fix(&v);
}

#[test]
fn the_ccr_route_422_saying_auto_merge_is_not_allowed_gets_the_fix() {
    let fake = FakeGithub::start();
    green_pr(&fake);
    fake.graphql_status(403, CCR_MSG);
    fake.fail_path_message(
        "/pulls/20/ccr/auto_merge",
        422,
        "Auto merge is not allowed for this repository",
    );
    let dir = repo("claude/t7");
    let (code, v) = bosun_in(&fake, dir.path(), &["pr", "7", "--auto-merge"]);
    assert_eq!(code, 1, "{v}");
    assert_auto_merge_off_fix(&v);
}

#[test]
fn an_unreadable_branch_check_does_not_block_the_pr() {
    let fake = FakeGithub::start();
    task(&fake, "bosun:state/building");
    fake.fail_path("/branches/claude%2Ft7", 403);
    let dir = repo("claude/t7");
    let (code, v) = bosun_in(&fake, dir.path(), &["pr", "7"]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["created"], true);
    assert!(
        fake.requests()
            .iter()
            .any(|r| r.path.ends_with("/branches/claude%2Ft7"))
    );
}

#[test]
fn a_repo_that_forbids_auto_merge_says_how_to_allow_it() {
    let fake = FakeGithub::start();
    green_pr(&fake);
    fake.graphql_status(422, "Auto merge is not allowed for this repository");
    let dir = repo("claude/t7");
    let (code, v) = bosun_in(&fake, dir.path(), &["pr", "7", "--auto-merge"]);
    assert_eq!(code, 1, "{v}");
    let fix = v["error"]["fix"].as_str().unwrap();
    assert!(fix.contains("Allow auto-merge"), "{fix}");
    assert!(fix.contains("hoist apply"), "{fix}");
    assert!(!fix.contains("githubstatus"), "{fix}");
}
