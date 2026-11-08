//! Follow-ups to the first-run PRs: the adopt key in `git add`, doctor on a bad API URL.

mod common;

use common::{bosun, one_object};
use serde_json::Value;
use std::path::Path;

fn adopt(dir: &Path, extra: &[&str]) -> Value {
    let out = bosun()
        .env_remove("GITHUB_TOKEN")
        .env_remove("BOSUN_REPO")
        .args(["-C", dir.to_str().unwrap(), "adopt", "--no-remote"])
        .args(extra)
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(0));
    one_object(&out.stdout)
}

fn commands(v: &Value) -> Vec<String> {
    v["data"]["commands"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c.as_str().unwrap().to_string())
        .collect()
}

fn keyless_repo() -> tempfile::TempDir {
    let t = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(t.path().join(".bosun")).unwrap();
    std::fs::write(t.path().join(".bosun/approvers.toml"), "# no key yet\n").unwrap();
    t
}

#[test]
fn a_kept_keyless_approvers_file_is_still_added() {
    let t = keyless_repo();
    let c = commands(&adopt(t.path(), &["--owner", "alice"]));
    let add: Vec<&str> = c[1].strip_prefix("git add ").unwrap().split(' ').collect();
    assert_eq!(
        add.iter()
            .filter(|p| **p == ".bosun/approvers.toml")
            .count(),
        1,
        "{add:?}"
    );
    let mut sorted = add.clone();
    sorted.sort();
    assert_eq!(add, sorted);
}

#[test]
fn a_noop_rerun_with_the_key_step_still_commits_the_approvers_file() {
    let t = keyless_repo();
    adopt(t.path(), &["--owner", "alice"]);
    let v = adopt(t.path(), &["--owner", "alice"]);
    assert!(v["data"]["owner_steps"].to_string().contains("keygen"));
    let c = commands(&v);
    assert_eq!(c[1], "git add .bosun/approvers.toml", "{c:?}");
    assert_eq!(c.last().unwrap(), "bosun doctor");
}

#[test]
fn doctor_names_a_malformed_api_url() {
    let t = tempfile::tempdir().unwrap();
    let out = bosun()
        .env("GITHUB_TOKEN", "t")
        .env("BOSUN_GITHUB_API", "not-a-url")
        .env("CLAUDE_CONFIG_DIR", t.path().join(".test-claude"))
        .arg("-C")
        .arg(t.path())
        .arg("doctor")
        .output()
        .unwrap();
    let v = one_object(&out.stdout);
    let all = v.to_string();
    assert_eq!(v["ok"], false, "{all}");
    assert!(
        v["error"]["message"]
            .as_str()
            .unwrap()
            .contains("BOSUN_GITHUB_API is not a valid URL"),
        "{all}"
    );
    assert!(
        v["error"]["fix"].as_str().unwrap().contains("unset"),
        "{all}"
    );
    assert!(!all.contains("no GITHUB_TOKEN"), "{all}");
}
