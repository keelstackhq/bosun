//! `bosun gate sources --pr N`: who posted each gate on the PR head (spec §15).

mod common;

use common::fake::FakeGithub;
use common::{bosun, one_object};
use serde_json::Value;

const SLUG: &str = "keelstackhq-bosun-gate";
const APP_ID: u64 = 4242;
const HEAD: &str = "sha1";
const GATES: [&str; 3] = ["risk-check", "review-verdict", "main-open"];

fn repo(mode: &str) -> tempfile::TempDir {
    let dir = common::git_tempdir();
    std::fs::create_dir_all(dir.path().join(".bosun")).unwrap();
    let text = match mode {
        "statuses" => "[checks]\nmode = \"statuses\"\n".to_string(),
        _ => format!("[checks]\nmode = \"{mode}\"\napp_id = {APP_ID}\napp_slug = \"{SLUG}\"\n"),
    };
    std::fs::write(dir.path().join(".bosun/bosun.toml"), text).unwrap();
    dir
}

fn run(fake: &FakeGithub, dir: &tempfile::TempDir, args: &[&str]) -> (i32, Value) {
    let out = bosun()
        .env("BOSUN_GITHUB_API", fake.url())
        .env("BOSUN_HTTP_MAX_SLEEP_MS", "0")
        .arg("-C")
        .arg(dir.path())
        .args(args)
        .output()
        .unwrap();
    let v = one_object(&out.stdout);
    if v["ok"] == false {
        assert!(!v["error"]["fix"].as_str().unwrap().trim().is_empty());
    }
    (out.status.code().unwrap(), v)
}

fn pr1() -> FakeGithub {
    let fake = FakeGithub::start();
    fake.pull(1, "t", "b1", "");
    fake.pull_head_sha(1, HEAD);
    fake
}

fn gate<'a>(v: &'a Value, name: &str) -> &'a Value {
    let gates = v["data"]["gates"].as_array().unwrap();
    gates.iter().find(|g| g["context"] == name).unwrap()
}

fn all_green(fake: &FakeGithub) {
    for g in GATES {
        fake.status(HEAD, g, "success", "ok");
        fake.app_run(HEAD, g, "success", "ok", SLUG, APP_ID);
    }
}

#[test]
fn dual_with_agreeing_app_runs_is_verified() {
    let fake = pr1();
    all_green(&fake);
    let (code, v) = run(&fake, &repo("dual"), &["gate", "sources", "--pr", "1"]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["verified"], true);
    assert_eq!(v["data"]["mode"], "dual");
    assert_eq!(v["data"]["app_id"], APP_ID);
    assert_eq!(v["data"]["head_sha"], HEAD);
    for g in GATES {
        let g = gate(&v, g);
        assert_eq!(g["trusted"], true, "{g}");
        assert_eq!(g["disagree"], false, "{g}");
        assert_eq!(g["state"], "success");
        assert_eq!(g["app_run"]["state"], "success");
        assert_eq!(g["status"]["state"], "success");
    }
}

#[test]
fn dual_forged_green_status_against_app_failure_is_a_conflict() {
    let fake = pr1();
    all_green(&fake);
    fake.status_by(HEAD, "risk-check", "success", "forged", "mallory");
    fake.app_run(HEAD, "risk-check", "failure", "high risk", SLUG, APP_ID);
    let (code, v) = run(&fake, &repo("dual"), &["gate", "sources", "--pr", "1"]);
    assert_eq!(code, 3, "{v}");
    assert_eq!(v["data"]["verified"], false);
    let g = gate(&v, "risk-check");
    assert_eq!(g["disagree"], true, "{g}");
    assert_eq!(g["state"], "failure");
    assert_eq!(g["status"]["creator"], "mallory");
    assert!(
        v["error"]["message"]
            .as_str()
            .unwrap()
            .contains("risk-check")
    );
    assert!(v["error"]["fix"].as_str().unwrap().contains("re-run"));
}

#[test]
fn statuses_mode_is_never_verified() {
    let fake = pr1();
    all_green(&fake);
    let (code, v) = run(&fake, &repo("statuses"), &["gate", "sources", "--pr", "1"]);
    assert_eq!(code, 3, "{v}");
    for g in GATES {
        assert_eq!(gate(&v, g)["trusted"], false);
        assert_eq!(gate(&v, g)["state"], "success");
    }
    assert!(
        v["error"]["fix"]
            .as_str()
            .unwrap()
            .contains("[checks] mode")
    );
}

#[test]
fn runs_by_actions_or_another_app_never_count() {
    let fake = pr1();
    for g in GATES {
        fake.status(HEAD, g, "success", "ok");
        fake.check_run(HEAD, g, "completed", Some("success"));
        fake.app_run(HEAD, g, "success", "ok", "other-app", 999);
    }
    let (code, v) = run(&fake, &repo("checks"), &["gate", "sources", "--pr", "1"]);
    assert_eq!(code, 3, "{v}");
    for g in GATES {
        let g = gate(&v, g);
        assert_eq!(g["trusted"], false, "{g}");
        assert_eq!(g["app_run"], Value::Null);
        assert_eq!(g["other_runs"], 2);
        assert_eq!(g["state"], Value::Null);
    }
}

#[test]
fn checks_mode_ignores_statuses() {
    let fake = pr1();
    for g in GATES {
        fake.status(HEAD, g, "failure", "forged red");
        fake.app_run(HEAD, g, "success", "ok", SLUG, APP_ID);
    }
    let (code, v) = run(&fake, &repo("checks"), &["gate", "sources", "--pr", "1"]);
    assert_eq!(code, 0, "a status counts for nothing in checks mode: {v}");
    assert_eq!(v["data"]["verified"], true);
    let g = gate(&v, "main-open");
    assert_eq!(g["state"], "success", "{g}");
    assert_eq!(g["trusted"], true);
    assert_eq!(g["disagree"], true);
}

#[test]
fn a_missing_pr_is_an_error() {
    let fake = FakeGithub::start();
    let (code, v) = run(&fake, &repo("dual"), &["gate", "sources", "--pr", "7"]);
    assert_eq!(code, 1, "{v}");
}

#[test]
fn no_pr_number_is_a_usage_error() {
    let fake = FakeGithub::start();
    let (code, v) = run(&fake, &repo("dual"), &["gate", "sources"]);
    assert_eq!(code, 2, "{v}");
}
