//! `bosun gate ci-minutes`: the CI-minute rules over `.github/workflows` (docs/ci-minutes.md),
//! built in like test-integrity. Also: bosun's own workflows must pass it, and `bosun doctor`
//! reports the same findings as step 14.

mod common;

use common::fake::FakeGithub;
use common::{bosun, one_object};
use serde_json::Value;
use std::fs;
use std::path::Path;
use std::process::Command;

fn init(root: &Path) {
    let out = Command::new("git")
        .current_dir(root)
        .args(["init", "-q", "-b", "main"])
        .output()
        .expect("git");
    assert!(out.status.success(), "{out:?}");
}

fn gate(root: &Path) -> (i32, Value) {
    let out = bosun()
        .env_remove("GITHUB_BASE_REF")
        .arg("-C")
        .arg(root)
        .args(["gate", "ci-minutes"])
        .output()
        .unwrap();
    let v = one_object(&out.stdout);
    if v["ok"] == false {
        assert!(!v["error"]["fix"].as_str().unwrap().trim().is_empty());
    }
    (out.status.code().unwrap(), v)
}

/// A repo with the given workflow files, no `.bosun`: the gate needs no config.
fn repo(files: &[(&str, &str)]) -> tempfile::TempDir {
    let tmp = tempfile::tempdir().unwrap();
    init(tmp.path());
    for (name, text) in files {
        let p = tmp.path().join(".github/workflows").join(name);
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(p, text).unwrap();
    }
    tmp
}

const GOOD: &str = "name: ci
on:
  pull_request:
    types: [opened, synchronize, ready_for_review]
concurrency:
  group: ci-${{ github.ref }}
  cancel-in-progress: true
jobs:
  ci:
    if: github.event_name != 'pull_request' || github.event.pull_request.draft == false
    runs-on: ubuntu-latest
    steps:
      - run: true
";

#[test]
fn a_workflow_that_skips_drafts_passes() {
    let tmp = repo(&[("ci.yml", GOOD)]);
    let (code, v) = gate(tmp.path());
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["ok"], true);
    assert_eq!(v["data"]["findings"], serde_json::json!([]));
    assert_eq!(v["data"]["files"][0], ".github/workflows/ci.yml");
}

#[test]
fn a_repo_without_workflows_passes() {
    let tmp = repo(&[]);
    let (code, v) = gate(tmp.path());
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["findings"], serde_json::json!([]));
}

#[test]
fn findings_fail_the_gate_with_file_job_rule_and_fix() {
    let bad = GOOD
        .replace(
            "    if: github.event_name != 'pull_request' || github.event.pull_request.draft == false\n",
            "",
        )
        .replace(", ready_for_review", "")
        .replace("  cancel-in-progress: true\n", "");
    let tmp = repo(&[("ci.yml", &bad), ("other.yaml", GOOD)]);
    let (code, v) = gate(tmp.path());
    assert_eq!(code, 1, "{v}");
    assert_eq!(v["ok"], false);
    let f = v["data"]["findings"].as_array().unwrap();
    let mut rules: Vec<_> = f.iter().map(|x| x["rule"].as_str().unwrap()).collect();
    rules.sort();
    assert_eq!(rules, ["concurrency", "draft", "ready_for_review"], "{v}");
    for x in f {
        assert_eq!(x["file"], ".github/workflows/ci.yml");
        assert!(!x["fix"].as_str().unwrap().is_empty());
    }
    let draft = f.iter().find(|x| x["rule"] == "draft").unwrap();
    assert_eq!(draft["job"], "ci");
    assert!(v["error"]["fix"].as_str().unwrap().contains("draft"), "{v}");
}

#[test]
fn bosuns_own_workflows_pass_the_gate() {
    let (code, v) = gate(Path::new(env!("CARGO_MANIFEST_DIR")));
    assert_eq!(code, 0, "{v}");
    assert!(v["data"]["files"].as_array().unwrap().len() >= 5, "{v}");
}

fn doctor_step_14(tmp: &Path) -> Value {
    let fake = FakeGithub::start();
    let out = bosun()
        .env_remove("GITHUB_BASE_REF")
        .env("BOSUN_GITHUB_API", fake.url())
        .env("CLAUDE_CONFIG_DIR", tmp.join(".test-claude"))
        .arg("-C")
        .arg(tmp)
        .args(["doctor", "--full"])
        .output()
        .unwrap();
    let v = one_object(&out.stdout);
    v["data"]["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|i| i["step"] == 14)
        .unwrap_or_else(|| panic!("no step 14: {v}"))
        .clone()
}

#[test]
fn doctor_reports_the_same_findings_as_step_14() {
    let tmp = repo(&[("ci.yml", &GOOD.replace(", ready_for_review", ""))]);
    let item = doctor_step_14(tmp.path());
    assert_eq!(item["name"], "CI minutes");
    assert_eq!(item["status"], "missing", "{item}");
    assert!(
        item["detail"]
            .as_str()
            .unwrap()
            .contains("ready_for_review")
    );
    assert!(item["fix"].as_str().unwrap().contains("ready_for_review"));
    let tmp = repo(&[("ci.yml", GOOD)]);
    assert_eq!(doctor_step_14(tmp.path())["status"], "ok");
}

/// Commit `.bosun/bosun.toml` with `base_cfg` and point `origin/main` at it, then write `work_cfg`
/// into the working tree (a PR that changes the provider).
fn with_base(tmp: &Path, base_cfg: &str, work_cfg: &str) {
    let git = |args: &[&str]| {
        let out = Command::new("git")
            .current_dir(tmp)
            .args(["-c", "user.name=t", "-c", "user.email=t@t"])
            .args(args)
            .output()
            .unwrap();
        assert!(out.status.success(), "{args:?}: {out:?}");
    };
    fs::create_dir_all(tmp.join(".bosun")).unwrap();
    fs::write(tmp.join(".bosun/bosun.toml"), base_cfg).unwrap();
    git(&["add", ".bosun"]);
    git(&["commit", "-q", "-m", "base"]);
    git(&["update-ref", "refs/remotes/origin/main", "HEAD"]);
    fs::write(tmp.join(".bosun/bosun.toml"), work_cfg).unwrap();
}

const NONE: &str = "[ci]\nprovider = \"none\"\n";
const BAD: &str = "on: push\njobs:\n  a:\n    runs-on: ubuntu-latest\n    steps: []\n";

#[test]
fn ci_minutes_skips_without_ci_and_without_workflows() {
    let tmp = repo(&[]);
    with_base(tmp.path(), NONE, NONE);
    let (code, v) = gate(tmp.path());
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["skipped"], true);
    assert_eq!(v["data"]["reason"], "ci provider none");
}

#[test]
fn ci_minutes_scans_existing_workflows_even_under_provider_none() {
    let tmp = repo(&[("x.yml", BAD)]);
    with_base(tmp.path(), NONE, NONE);
    let (code, v) = gate(tmp.path());
    assert_eq!(code, 1, "{v}");
    assert!(v["data"]["skipped"].is_null(), "{v}");
    assert!(!v["data"]["findings"].as_array().unwrap().is_empty(), "{v}");
}

#[test]
fn ci_minutes_reads_the_provider_from_the_base_not_the_working_tree() {
    // The PR sets `none` in its working tree; the base says github: no skip.
    let tmp = repo(&[]);
    with_base(tmp.path(), "[ci]\nprovider = \"github\"\n", NONE);
    let (code, v) = gate(tmp.path());
    assert_eq!(code, 0, "{v}");
    assert!(v["data"]["skipped"].is_null(), "{v}");
    // And with workflows that break the rules, the PR cannot hide them by setting none.
    let tmp = repo(&[("x.yml", BAD)]);
    with_base(tmp.path(), "[ci]\nprovider = \"github\"\n", NONE);
    let (code, _) = gate(tmp.path());
    assert_eq!(code, 1);
}

/// The same base the other gates use (`resolve_base_in`): `GITHUB_BASE_REF`, else what
/// `origin/HEAD` points to, else `origin/main`.
#[test]
fn ci_minutes_base_follows_origin_head_and_github_base_ref() {
    let tmp = repo(&[]);
    with_base(tmp.path(), NONE, NONE);
    let git = |args: &[&str]| {
        let out = Command::new("git")
            .current_dir(tmp.path())
            .args(args)
            .output()
            .unwrap();
        assert!(out.status.success(), "{args:?}: {out:?}");
    };
    // The base branch is `trunk`; there is no `origin/main`.
    git(&["update-ref", "refs/remotes/origin/trunk", "HEAD"]);
    git(&["update-ref", "-d", "refs/remotes/origin/main"]);
    git(&[
        "symbolic-ref",
        "refs/remotes/origin/HEAD",
        "refs/remotes/origin/trunk",
    ]);
    let (code, v) = gate(tmp.path());
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["skipped"], true, "origin/HEAD is the base: {v}");
    // `GITHUB_BASE_REF` wins over origin/HEAD.
    git(&["update-ref", "refs/remotes/origin/other", "HEAD"]);
    git(&[
        "symbolic-ref",
        "refs/remotes/origin/HEAD",
        "refs/remotes/origin/nothing",
    ]);
    let out = bosun()
        .env("GITHUB_BASE_REF", "other")
        .arg("-C")
        .arg(tmp.path())
        .args(["gate", "ci-minutes"])
        .output()
        .unwrap();
    let v = one_object(&out.stdout);
    assert_eq!(
        v["data"]["skipped"], true,
        "GITHUB_BASE_REF is the base: {v}"
    );
}

#[test]
fn ci_minutes_invalid_config_is_exit_1() {
    let tmp = repo(&[("ci.yml", GOOD)]);
    std::fs::create_dir_all(tmp.path().join(".bosun")).unwrap();
    // A non-table `ci` must not read as github.
    std::fs::write(tmp.path().join(".bosun/bosun.toml"), "ci = \"none\"\n").unwrap();
    let (code, v) = gate(tmp.path());
    assert_eq!(code, 1, "{v}");
}
