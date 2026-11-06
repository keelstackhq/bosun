//! `bosun pr` checks the PR's commits for Bosun trailers: a note by default, a refusal with
//! `[trailers] require = true`.

mod common;

use std::process::Command;

use common::fake::FakeGithub;
use common::{bosun, one_object};
use serde_json::Value;

const BRANCH: &str = "claude/t7";
const BUILDING: &str = "bosun:state/building";
const LEASE: &str = r#"{"bosun":"lease","session":"session_test","origin":"spawned","expires":"2026-10-22T10:30:00Z","touch":["src/a.rs"]}"#;
const GOOD: &str = "feat: a\n\nBosun-Session: s\nBosun-Task: 7\n";

fn run_pr(shas: &[(&str, &str)], config: Option<&str>) -> (i32, Value) {
    let fake = FakeGithub::start();
    fake.issue(7, "the task", &[BUILDING], "Do it.");
    fake.comment(7, LEASE);
    fake.pull(3, "t", BRANCH, "Closes #7");
    let names: Vec<&str> = shas.iter().map(|(s, _)| *s).collect();
    fake.pull_commit_shas(3, &names);
    for (sha, msg) in shas {
        fake.git_commit_message(sha, msg);
    }
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
    if let Some(c) = config {
        std::fs::create_dir_all(dir.path().join(".bosun")).unwrap();
        std::fs::write(dir.path().join(".bosun/bosun.toml"), c).unwrap();
    }
    let out = bosun()
        .env("BOSUN_GITHUB_API", fake.url())
        .arg("-C")
        .arg(dir.path())
        .args(["pr", "7"])
        .output()
        .unwrap();
    (out.status.code().unwrap(), one_object(&out.stdout))
}

#[test]
fn stamped_commits_pass_clean() {
    let (code, v) = run_pr(&[("a1", GOOD)], None);
    assert_eq!(code, 0, "{v}");
    let t = &v["data"]["trailers"];
    assert_eq!(t["missing"].as_array().unwrap().len(), 0, "{v}");
    assert_eq!(t["checked"], 1);
}

#[test]
fn a_missing_trailer_is_a_note_by_default() {
    let (code, v) = run_pr(&[("a1", GOOD), ("b2", "fix: b\n")], None);
    assert_eq!(code, 0, "{v}");
    let m = &v["data"]["trailers"]["missing"][0];
    assert_eq!(m["sha"], "b2", "{v}");
    assert_eq!(
        m["missing"],
        serde_json::json!(["Bosun-Session", "Bosun-Task"])
    );
}

#[test]
fn require_refuses_with_a_fix() {
    let (code, v) = run_pr(&[("b2", "fix: b\n")], Some("[trailers]\nrequire = true\n"));
    assert_eq!(code, 1, "{v}");
    assert!(
        v["error"]["message"].as_str().unwrap().contains("b2"),
        "{v}"
    );
    let fix = v["error"]["fix"].as_str().unwrap();
    assert!(fix.contains("bosun trailers install"), "{v}");
}

#[test]
fn require_passes_when_all_are_stamped() {
    let (code, v) = run_pr(&[("a1", GOOD)], Some("[trailers]\nrequire = true\n"));
    assert_eq!(code, 0, "{v}");
}
