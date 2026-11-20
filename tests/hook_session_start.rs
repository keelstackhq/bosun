//! SessionStart: version skew, config errors, the janitor and the short rules block.
mod common;

use std::path::Path;
use std::process::Command;

use common::fake::FakeGithub;
use common::{bosun, run};
use serde_json::{Value, json};

fn repo() -> tempfile::TempDir {
    let tmp = tempfile::tempdir().unwrap();
    let git = |a: &[&str]| {
        let o = Command::new("git")
            .arg("-C")
            .arg(tmp.path())
            .args(a)
            .output()
            .unwrap();
        assert!(o.status.success());
    };
    git(&["init", "-q", "-b", "main"]);
    git(&[
        "remote",
        "add",
        "origin",
        "https://github.com/acme/widget.git",
    ]);
    // A set-up repo: session-start skips an un-adopted one.
    std::fs::create_dir_all(tmp.path().join(".bosun")).unwrap();
    std::fs::write(tmp.path().join(".bosun/version"), env!("CARGO_PKG_VERSION")).unwrap();
    tmp
}

fn start(api: &str, dir: &Path, now: &str) -> String {
    let out = bosun()
        .current_dir(dir)
        .env("BOSUN_SESSION", "session_me")
        .env("BOSUN_NOW", now)
        .env("BOSUN_GITHUB_API", api)
        .args(["hook", "session-start"])
        .write_stdin(json!({"cwd": dir.to_str().unwrap()}).to_string())
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(0));
    let v: Value = common::one_object(&out.stdout);
    v["hookSpecificOutput"]["additionalContext"]
        .as_str()
        .unwrap()
        .to_string()
}

fn pin_file(dir: &Path, version: &str) {
    std::fs::create_dir_all(dir.join(".bosun")).unwrap();
    std::fs::write(dir.join(".bosun/version"), format!("{version}\n")).unwrap();
}

#[test]
fn a_binary_behind_the_repo_pin_says_how_to_fix_it_in_one_line() {
    let (fake, tmp) = (FakeGithub::start(), repo());
    pin_file(tmp.path(), "99.0.0");
    let want = format!(
        "installed {}, repo pins 99.0.0: run bash docs/setup-script.sh",
        env!("CARGO_PKG_VERSION")
    );
    for api in [fake.url(), "http://127.0.0.1:9".to_string()] {
        let text = start(&api, tmp.path(), "2026-10-22T10:00:00Z");
        assert!(text.contains(&want), "{text}");
    }
}

#[test]
fn a_matching_pin_is_silent() {
    let (fake, tmp) = (FakeGithub::start(), repo());
    pin_file(tmp.path(), env!("CARGO_PKG_VERSION"));
    let text = start(&fake.url(), tmp.path(), "2026-10-22T10:00:00Z");
    assert!(!text.contains("repo pins"), "{text}");
}

#[test]
fn a_config_parse_error_is_named_not_blamed_on_github() {
    let tmp = repo();
    std::fs::create_dir_all(tmp.path().join(".bosun")).unwrap();
    std::fs::write(tmp.path().join(".bosun/bosun.toml"), "bogus_key = 1\n").unwrap();
    let text = start("http://127.0.0.1:9", tmp.path(), "2026-10-22T10:00:00Z");
    assert!(text.contains("bosun.toml"), "{text}");
    assert!(!text.contains("could not reach GitHub"), "{text}");
}

#[test]
fn session_start_reaps_expired_leases() {
    let fake = FakeGithub::start();
    let task = r#"x
<!-- bosun:task {"item":"M1","task":"T5","touch":["src/x/**"],"kind":"feature"} -->"#;
    fake.issue(5, "task", &["bosun:state/ready"], task);
    run(&fake, "session_a", "2026-10-22T10:00:00Z", &["join"]);
    run(&fake, "session_a", "2026-10-22T10:00:00Z", &["claim", "5"]);
    let tmp = repo();
    start(&fake.url(), tmp.path(), "2026-10-22T11:00:00Z");
    assert_eq!(fake.labels_of(5), vec!["bosun:state/ready"]);
}

#[test]
fn the_default_rules_stay_short() {
    let (fake, tmp) = (FakeGithub::start(), repo());
    let text = start(&fake.url(), tmp.path(), "2026-10-22T10:00:00Z");
    let rules = text.split("Session rules").nth(1).unwrap();
    assert!(rules.len() < 420, "{} chars: {rules}", rules.len());
}
