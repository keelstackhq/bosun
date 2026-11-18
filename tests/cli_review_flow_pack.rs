//! M14: the pack carries the draft flag, the required check states and the head's gate stamps.
mod common;

use std::process::Command;

use common::fake::FakeGithub;
use common::run;
use serde_json::json;

const T0: &str = "2026-10-22T10:00:00Z";

fn git(dir: &std::path::Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

#[test]
fn the_pack_shows_draft_checks_and_stamps() {
    let d = tempfile::tempdir().unwrap();
    let dir = d.path();
    git(dir, &["init", "-q"]);
    std::fs::create_dir_all(dir.join(".bosun")).unwrap();
    std::fs::write(
        dir.join(".bosun/bosun.toml"),
        "[[stack]]\npath = \".\"\nprofile = \"rust\"\n",
    )
    .unwrap();
    git(dir, &["add", "-A"]);
    git(
        dir,
        &[
            "-c",
            "user.name=t",
            "-c",
            "user.email=t@t",
            "commit",
            "-qm",
            "x",
        ],
    );
    let sha = git(dir, &["rev-parse", "HEAD"]);
    let tree = git(dir, &["rev-parse", "HEAD^{tree}"]);
    let common = git(
        dir,
        &["rev-parse", "--path-format=absolute", "--git-common-dir"],
    );
    let stamp = std::path::Path::new(&common).join(format!("bosun/gates/{tree}/fmt"));
    std::fs::create_dir_all(stamp.parent().unwrap()).unwrap();
    std::fs::write(&stamp, "ok\n").unwrap();

    let fake = FakeGithub::start();
    fake.pull_as_issue(42, "the pr");
    fake.pull(42, "the pr", "claude/x", "");
    fake.pull_set(42, "head", json!({"sha": sha, "ref": "claude/x"}));
    fake.pull_set(42, "draft", json!(true));
    fake.pull_file(42, "src/a.rs", "added", 1, 0, Some("@@ a"));
    fake.status(&sha, "ci-ok", "success", "ok");
    fake.status(&sha, "risk-check", "failure", "high");

    let (code, v) = run(
        &fake,
        "s",
        T0,
        &["-C", dir.to_str().unwrap(), "review-pack", "42"],
    );
    assert_eq!(code, 0, "{v}");
    let d = &v["data"];
    assert_eq!(d["draft"], true);
    let checks = d["required_checks"].as_array().unwrap();
    assert!(
        checks.contains(&json!({"context": "ci-ok", "state": "success"})),
        "{checks:?}"
    );
    assert!(checks.contains(&json!({"context": "risk-check", "state": "failure"})));
    assert!(checks.contains(&json!({"context": "main-open", "state": "missing"})));
    assert_eq!(d["gate_stamps"]["tree"], tree.as_str());
    assert!(
        d["gate_stamps"]["present"]
            .as_array()
            .unwrap()
            .contains(&json!("fmt"))
    );
    assert!(
        d["gate_stamps"]["missing"]
            .as_array()
            .unwrap()
            .contains(&json!("test"))
    );
}

#[test]
fn a_head_not_in_the_checkout_has_no_stamps() {
    let fake = FakeGithub::start();
    fake.pull_as_issue(42, "the pr");
    fake.pull(42, "the pr", "claude/x", "");
    fake.pull_file(42, "src/a.rs", "added", 1, 0, Some("@@ a"));
    let (code, v) = run(&fake, "s", T0, &["review-pack", "42"]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["draft"], false);
    let text = v["data"]["instructions"].as_str().unwrap();
    assert!(
        text.contains("scratchpad") && text.contains("--from -"),
        "{text}"
    );
    assert!(v["data"]["gate_stamps"].is_null(), "{v}");
}
