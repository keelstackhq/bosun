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
