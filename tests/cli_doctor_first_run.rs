//! `bosun doctor` in a freshly adopted repo: it runs without a token, names the exact fix and
//! points at the first missing step.

mod common;

use common::fake::FakeGithub;
use common::{bosun, one_object};
use serde_json::Value;
use std::path::Path;

const GH_FIX: &str = "export GITHUB_TOKEN (repo and workflow scopes), then rerun `bosun doctor`";

fn doctor(dir: &Path, token: bool) -> (i32, Value) {
    let fake = FakeGithub::start();
    let mut cmd = bosun();
    cmd.env("BOSUN_GITHUB_API", fake.url())
        .env("CLAUDE_CONFIG_DIR", dir.join(".test-claude"))
        .arg("-C")
        .arg(dir)
        .arg("doctor");
    if !token {
        cmd.env_remove("GITHUB_TOKEN").env_remove("GH_TOKEN");
    }
    let out = cmd.output().unwrap();
    let v = one_object(&out.stdout);
    if v["ok"] == false {
        assert!(!v["error"]["fix"].as_str().unwrap().trim().is_empty());
    }
    (out.status.code().unwrap(), v)
}

fn step(v: &Value, n: u64) -> &Value {
    v["data"]["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|i| i["step"] == n)
        .unwrap_or_else(|| panic!("no step {n}"))
}

#[test]
fn without_a_token_local_steps_run_and_github_steps_are_skipped() {
    let tmp = tempfile::tempdir().unwrap();
    let (code, v) = doctor(tmp.path(), false);
    assert_eq!(code, 5, "{v}");
    assert_eq!(step(&v, 4)["status"], "missing", "owners is judged locally");
    assert_eq!(step(&v, 12)["status"], "missing", "plan is judged locally");
    for n in [1, 2, 5, 8, 9] {
        let it = step(&v, n);
        assert_eq!(it["status"], "skipped", "{it}");
        assert_eq!(it["fix"], GH_FIX, "{it}");
    }
    let items = v["data"]["items"].as_array().unwrap();
    let missing = items.iter().filter(|i| i["status"] == "missing").count();
    assert_eq!(
        v["data"]["missing"], missing,
        "skipped steps are not missing"
    );
    assert!(
        v["error"]["message"]
            .as_str()
            .unwrap()
            .starts_with(&format!("{missing} checklist steps missing"))
    );
}

#[test]
fn fixes_name_no_bosun_repo_file_and_leave_no_placeholder() {
    let tmp = tempfile::tempdir().unwrap();
    let (_, v) = doctor(tmp.path(), true);
    let text = v.to_string();
    assert!(!text.contains("PROGRESS.md"), "{text}");
    assert!(!text.contains("<owner>"), "{text}");
    assert!(
        step(&v, 5)["fix"]
            .as_str()
            .unwrap()
            .ends_with("--repo acme/widget"),
        "{}",
        step(&v, 5)["fix"]
    );
    assert_eq!(
        step(&v, 8)["fix"],
        "run `bosun adopt` (with GITHUB_TOKEN set) or `bosun join`; either creates the registry issue"
    );
}

#[test]
fn next_step_names_the_first_missing_step_and_its_fix() {
    let tmp = tempfile::tempdir().unwrap();
    let (_, v) = doctor(tmp.path(), true);
    let first = v["data"]["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|i| i["status"] == "missing")
        .unwrap();
    assert_eq!(v["data"]["next_step"]["step"], first["step"]);
    assert_eq!(v["data"]["next_step"]["fix"], first["fix"]);
}

#[test]
fn the_setup_item_points_at_setup_snippet_instead_of_embedding_the_script() {
    let tmp = tempfile::tempdir().unwrap();
    let (_, v) = doctor(tmp.path(), true);
    let it = step(&v, 6);
    assert_eq!(it["run"], "bosun setup-snippet");
    assert!(it.get("bootstrap").is_none(), "{it}");
}

/// This repo with every local step passing: the plugin install is faked from the setup script's
/// pins and the Routines are declared done.
fn local_pass(token: bool, remote: bool) -> (i32, Value) {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let script = std::fs::read_to_string(root.join("docs/setup-script.sh")).unwrap();
    let pin = |k: &str| {
        let l = script
            .lines()
            .find(|l| l.starts_with(&format!("{k}=")))
            .unwrap();
        l[k.len() + 1..].trim().trim_matches('"').to_string()
    };
    let cfg = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(cfg.path().join("plugins")).unwrap();
    let installed = serde_json::json!({"plugins": {
        "superpowers@keelstack": [{"gitCommitSha": pin("SUPERPOWERS_SHA")}],
        "caveman@keelstack": [{"gitCommitSha": pin("CAVEMAN_SHA")}],
    }});
    std::fs::write(
        cfg.path().join("plugins/installed_plugins.json"),
        installed.to_string(),
    )
    .unwrap();
    let fake = FakeGithub::start();
    let mut cmd = bosun();
    cmd.env("BOSUN_GITHUB_API", fake.url())
        .env("CLAUDE_CONFIG_DIR", cfg.path())
        .env("BOSUN_ROUTINES_OK", "1")
        .arg("-C")
        .arg(root)
        .arg("doctor");
    if remote {
        cmd.arg("--remote");
    }
    if !token {
        cmd.env_remove("GITHUB_TOKEN").env_remove("GH_TOKEN");
    }
    let out = cmd.output().unwrap();
    (out.status.code().unwrap(), one_object(&out.stdout))
}

fn remote_item(v: &Value) -> &Value {
    v["data"]["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|i| i["name"] == "remote checks")
        .unwrap_or_else(|| panic!("no remote checks item: {v}"))
}

#[test]
fn offline_with_every_local_step_passing_is_not_a_pass() {
    let (code, v) = local_pass(false, false);
    assert_eq!(v["data"]["missing"], 0, "{v}");
    assert_eq!(code, 5, "{v}");
    assert_eq!(v["ok"], false);
    assert_eq!(v["data"]["offline"], true);
    assert_eq!(v["data"]["next_step"]["fix"], GH_FIX);
    assert_eq!(v["error"]["fix"], GH_FIX);
    assert!(
        v["error"]["message"]
            .as_str()
            .unwrap()
            .contains("GitHub not checked (no token)")
    );
    let it = remote_item(&v);
    assert_eq!(it["status"], "skipped");
    assert!(
        it["detail"].as_str().unwrap().contains("kill switch"),
        "{it}"
    );
}

#[test]
fn offline_remote_names_the_hoist_and_environment_checks() {
    let (code, v) = local_pass(false, true);
    assert_eq!(code, 5, "{v}");
    assert_eq!(v["data"]["offline"], true);
    let d = remote_item(&v)["detail"].as_str().unwrap();
    assert!(d.contains("hoist") && d.contains("environment"), "{d}");
}

#[test]
fn offline_keeps_the_first_missing_local_step_as_next_step() {
    let tmp = tempfile::tempdir().unwrap();
    let (code, v) = doctor(tmp.path(), false);
    assert_eq!(code, 5);
    assert_eq!(v["data"]["offline"], true);
    assert_ne!(v["data"]["next_step"]["fix"], GH_FIX);
    assert!(
        v["error"]["message"]
            .as_str()
            .unwrap()
            .ends_with("GitHub not checked (no token)")
    );
}

#[test]
fn with_a_token_doctor_is_not_offline() {
    let tmp = tempfile::tempdir().unwrap();
    let (_, v) = doctor(tmp.path(), true);
    assert_eq!(v["data"]["offline"], false);
    assert!(
        v["data"]["items"]
            .as_array()
            .unwrap()
            .iter()
            .all(|i| i["name"] != "remote checks")
    );
}
