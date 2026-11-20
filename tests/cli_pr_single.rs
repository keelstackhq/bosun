//! `bosun pr --single [--ready]`: the draft-to-auto-merge flow for the current branch with no task
//! issue at all (one PR, one change).
mod common;

use std::path::Path;
use std::process::Command;

use common::fake::FakeGithub;
use common::stamps;
use common::{bosun, one_object};
use serde_json::{Value, json};
use std::ops::Not;

const ALL: [&str; 4] = ["ci-ok", "risk-check", "review-verdict", "main-open"];

fn repo(branch: &str, subject: &str) -> tempfile::TempDir {
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
            subject,
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

fn pr(fake: &FakeGithub, dir: &Path, args: &[&str]) -> (i32, Value) {
    let out = bosun()
        .env("BOSUN_GITHUB_API", fake.url())
        .arg("-C")
        .arg(dir)
        .arg("pr")
        .args(args)
        .output()
        .unwrap();
    let v = one_object(&out.stdout);
    if v["ok"] == false {
        assert!(!v["error"]["fix"].as_str().unwrap().trim().is_empty());
    }
    (out.status.code().unwrap(), v)
}

fn setup() -> (FakeGithub, tempfile::TempDir) {
    let fake = FakeGithub::start();
    fake.enforce_checks(&ALL);
    (fake, repo("feat/widget", "feat: add the widget"))
}

fn create_request(fake: &FakeGithub) -> Value {
    let req = fake
        .requests()
        .into_iter()
        .find(|r| r.method == "POST" && r.path.ends_with("/pulls"))
        .expect("a PR was created");
    serde_json::from_str(req.body.as_deref().unwrap()).unwrap()
}

#[test]
fn it_opens_a_draft_titled_with_the_last_commit() {
    let (fake, dir) = setup();
    let (code, v) = pr(&fake, dir.path(), &["--single"]);
    assert_eq!(code, 0, "{v}");
    let d = &v["data"];
    assert_eq!(d["mode"], "single");
    assert_eq!(d["task"], Value::Null);
    assert_eq!(d["created"], true);
    assert_eq!(d["pr"]["draft"], true);
    let n = d["pr"]["number"].as_u64().unwrap();
    assert!(fake.pull_is_draft(n));
    let body = create_request(&fake);
    assert_eq!(body["title"], "feat: add the widget");
    assert_eq!(body["head"], "feat/widget");
    assert_eq!(body["draft"], true);
    assert!(!fake.pull_body(n).contains("Closes #"));
}

#[test]
fn title_overrides_the_commit_subject() {
    let (fake, dir) = setup();
    let (code, v) = pr(&fake, dir.path(), &["--single", "--title", "fix: other"]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(create_request(&fake)["title"], "fix: other");
}

#[test]
fn a_second_run_finds_the_pr() {
    let (fake, dir) = setup();
    let (_, first) = pr(&fake, dir.path(), &["--single"]);
    let (code, v) = pr(&fake, dir.path(), &["--single"]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["created"], false);
    assert_eq!(v["data"]["pr"]["number"], first["data"]["pr"]["number"]);
    let creates = fake
        .requests()
        .iter()
        .filter(|r| r.method == "POST" && r.path.ends_with("/pulls"))
        .count();
    assert_eq!(creates, 1);
}

#[test]
fn ready_with_stamps_marks_ready_and_turns_on_auto_merge() {
    let (fake, dir) = setup();
    let (_, first) = pr(&fake, dir.path(), &["--single"]);
    let n = first["data"]["pr"]["number"].as_u64().unwrap();
    stamps::ready(&fake, dir.path(), n);
    let (code, v) = pr(&fake, dir.path(), &["--single", "--ready"]);
    assert_eq!(code, 0, "{v}");
    let d = &v["data"];
    assert_eq!(d["mode"], "single");
    assert_eq!(d["task"], Value::Null);
    assert_eq!(d["ready"], true);
    assert_eq!(d["auto_merge"], true);
    assert_eq!(d["enforced_checks"], "verified");
    assert!(!fake.pull_is_draft(n));
    assert!(fake.auto_merge_enabled(n).is_some());
    assert!(fake.requests().iter().all(|r| !r.path.ends_with("/merge")));
}

#[test]
fn ready_without_stamps_names_the_missing_gates() {
    let (fake, dir) = setup();
    let (_, first) = pr(&fake, dir.path(), &["--single"]);
    let n = first["data"]["pr"]["number"].as_u64().unwrap();
    fake.pull_head_sha(n, &stamps::head_sha(dir.path()));
    let (code, v) = pr(&fake, dir.path(), &["--single", "--ready"]);
    assert_eq!(code, 1, "{v}");
    assert!(
        v["data"]["missing_gates"]
            .as_array()
            .unwrap()
            .is_empty()
            .not()
    );
    assert!(
        v["error"]["fix"]
            .as_str()
            .unwrap()
            .contains("bosun pr --single --ready")
    );
    assert!(fake.pull_is_draft(n));
}

#[test]
fn a_task_pr_is_refused_with_the_task_command() {
    let (fake, dir) = setup();
    fake.pull(31, "feat: x", "feat/widget", "Closes #7");
    let (code, v) = pr(&fake, dir.path(), &["--single"]);
    assert_eq!(code, 3, "{v}");
    assert!(v["error"]["fix"].as_str().unwrap().contains("bosun pr 7"));
}

#[test]
fn the_default_branch_is_a_usage_error() {
    let fake = FakeGithub::start();
    let dir = repo("main", "feat: x");
    let (code, v) = pr(&fake, dir.path(), &["--single"]);
    assert_eq!(code, 2, "{v}");
}

#[test]
fn single_conflicts_with_an_issue() {
    let (fake, dir) = setup();
    let (code, v) = pr(&fake, dir.path(), &["7", "--single"]);
    assert_eq!(code, 2, "{v}");
}

fn no_token(dir: &Path, args: &[&str]) -> (i32, Value) {
    let out = bosun()
        .env_remove("GITHUB_TOKEN")
        .arg("-C")
        .arg(dir)
        .arg("pr")
        .args(args)
        .output()
        .unwrap();
    (out.status.code().unwrap(), one_object(&out.stdout))
}

#[test]
fn without_a_token_it_hands_the_create_call_over() {
    let dir = repo("feat/widget", "feat: add the widget");
    let (code, v) = no_token(dir.path(), &["--single"]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["mode"], "single");
    assert_eq!(v["data"]["pr"], Value::Null);
    let next = &v["next"][0];
    assert_eq!(next["tool"], "mcp__github__create_pull_request");
    let a = &next["args"];
    assert_eq!(
        (a["owner"].clone(), a["repo"].clone(), a["head"].clone()),
        (json!("acme"), json!("widget"), json!("feat/widget"))
    );
    assert_eq!(a["title"], "feat: add the widget");
    assert_eq!(a["draft"], true);
}

#[test]
fn without_a_token_ready_refuses_with_the_login_fix() {
    let dir = repo("feat/widget", "feat: add the widget");
    stamps::stamp_all(dir.path());
    let (code, v) = no_token(dir.path(), &["--single", "--ready"]);
    assert_eq!(code, 1, "{v}");
    assert_eq!(
        v["error"]["fix"],
        "`--ready` needs GitHub: run `gh auth login` or export GITHUB_TOKEN, then rerun `bosun pr --single --ready`"
    );
    assert!(v["next"].as_array().is_none_or(|n| n.is_empty()));
    assert!(!v.to_string().contains("enable_pr_auto_merge"));
}

fn with_bosun_base(fake: &FakeGithub) {
    fake.git_tree(
        "main",
        &[
            (".bosun", "040000", "tree", "t1"),
            (".bosun/bosun.toml", "100644", "blob", "b1"),
        ],
        false,
    );
}

fn single_ready(fake: &FakeGithub, dir: &Path) -> (i32, Value) {
    let (_, first) = pr(fake, dir, &["--single"]);
    let n = first["data"]["pr"]["number"].as_u64().unwrap();
    stamps::ready(fake, dir, n);
    fake.pull_file(n, "src/a.rs", "modified", 3, 1, None);
    pr(fake, dir, &["--single", "--ready"])
}

#[test]
fn ready_reports_bootstrap_when_the_base_has_no_bosun_dir() {
    let (fake, dir) = setup();
    let (code, v) = single_ready(&fake, dir.path());
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["single_reason"], "bootstrap");
}

#[test]
fn ready_on_a_bosun_repo_allows_a_one_off_sized_change() {
    let (fake, dir) = setup();
    with_bosun_base(&fake);
    let (code, v) = single_ready(&fake, dir.path());
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["single_reason"], "one-off");
}

#[test]
fn ready_on_a_bosun_repo_refuses_a_path_outside_the_allow_set() {
    let (fake, dir) = setup();
    with_bosun_base(&fake);
    let (_, first) = pr(&fake, dir.path(), &["--single"]);
    let n = first["data"]["pr"]["number"].as_u64().unwrap();
    stamps::ready(&fake, dir.path(), n);
    fake.pull_file(n, ".github/workflows/ci.yml", "modified", 1, 0, None);
    let (code, v) = pr(&fake, dir.path(), &["--single", "--ready"]);
    assert_eq!(code, 3, "{v}");
    assert!(
        v["error"]["fix"]
            .as_str()
            .unwrap()
            .contains("bootstrap PR or a one-off-sized change")
    );
    assert!(fake.pull_is_draft(n));
    assert!(fake.auto_merge_enabled(n).is_none());
}

#[test]
fn ready_on_a_bosun_repo_refuses_over_the_one_off_cap() {
    let (fake, dir) = setup();
    with_bosun_base(&fake);
    let (_, first) = pr(&fake, dir.path(), &["--single"]);
    let n = first["data"]["pr"]["number"].as_u64().unwrap();
    stamps::ready(&fake, dir.path(), n);
    fake.pull_file(n, "src/big.rs", "added", 301, 0, None);
    let (code, v) = pr(&fake, dir.path(), &["--single", "--ready"]);
    assert_eq!(code, 3, "{v}");
    assert!(fake.auto_merge_enabled(n).is_none());
}
