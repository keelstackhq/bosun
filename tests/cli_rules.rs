//! `bosun rules --write` renders the session rules into a marked block of CLAUDE.md or
//! AGENTS.md, for environments where hooks do not load (Claude in Slack); `doctor` warns there
//! when the block is missing (spec §4).
mod common;

use std::fs;
use std::path::Path;

use common::fake::FakeGithub;
use common::{bosun, one_object};
use serde_json::Value;

const START: &str = "<!-- bosun:session-rules:start -->";

fn rules(dir: &Path, write: bool) -> (i32, Value) {
    let mut cmd = bosun();
    cmd.arg("-C").arg(dir).arg("rules");
    if write {
        cmd.arg("--write");
    }
    let out = cmd.output().unwrap();
    (out.status.code().unwrap(), one_object(&out.stdout))
}

#[test]
fn write_renders_the_block_once_and_keeps_the_rest() {
    let tmp = tempfile::tempdir().unwrap();
    let d = tmp.path();
    // No file yet: a dry run writes nothing; --write creates CLAUDE.md.
    let (code, v) = rules(d, false);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["changed"], true);
    assert!(!d.join("CLAUDE.md").exists());
    assert_eq!(rules(d, true).0, 0);
    let text = fs::read_to_string(d.join("CLAUDE.md")).unwrap();
    assert!(
        text.contains(START) && text.contains("Spawn no sessions."),
        "{text}"
    );
    assert!(
        text.contains("npx -y @keelstackhq/bosun@$(cat .bosun/version)"),
        "{text}"
    );
    // AGENTS.md alone is the target when CLAUDE.md is absent; text around the block stays.
    let tmp = tempfile::tempdir().unwrap();
    let d = tmp.path();
    fs::write(d.join("AGENTS.md"), "# Agents\n\nmine\n").unwrap();
    rules(d, true);
    fs::create_dir_all(d.join(".bosun")).unwrap();
    fs::write(d.join(".bosun/session-rules.toml"), "worker = \"W-RULE\"\n").unwrap();
    let (_, v) = rules(d, true);
    assert_eq!(v["data"]["file"], "AGENTS.md");
    assert_eq!(v["data"]["changed"], true);
    let text = fs::read_to_string(d.join("AGENTS.md")).unwrap();
    assert!(text.starts_with("# Agents\n\nmine\n"), "{text}");
    assert_eq!(text.matches(START).count(), 1, "{text}");
    assert!(text.contains("W-RULE") && !text.contains("Spawn no sessions."));
    // Idempotent.
    let (_, v) = rules(d, true);
    assert_eq!(v["data"]["changed"], false);
    assert_eq!(fs::read_to_string(d.join("AGENTS.md")).unwrap(), text);
    // A start marker without an end is refused, nothing written.
    fs::write(d.join("AGENTS.md"), format!("x\n{START}\ny\n")).unwrap();
    let (code, v) = rules(d, true);
    assert_eq!(code, 1, "{v}");
    assert!(!v["error"]["fix"].as_str().unwrap().is_empty());
    assert_eq!(
        fs::read_to_string(d.join("AGENTS.md")).unwrap(),
        format!("x\n{START}\ny\n")
    );
}

#[test]
fn an_unreadable_or_linked_file_is_refused_never_overwritten() {
    let tmp = tempfile::tempdir().unwrap();
    let d = tmp.path();
    fs::write(d.join("CLAUDE.md"), b"caf\xe9\n").unwrap();
    let (code, v) = rules(d, true);
    assert_eq!(code, 1, "{v}");
    assert_eq!(fs::read(d.join("CLAUDE.md")).unwrap(), b"caf\xe9\n");
    #[cfg(unix)]
    {
        let tmp = tempfile::tempdir().unwrap();
        let d = tmp.path();
        std::os::unix::fs::symlink(d.join("elsewhere.md"), d.join("CLAUDE.md")).unwrap();
        assert_eq!(rules(d, true).0, 1);
        assert!(!d.join("elsewhere.md").exists());
    }
}

fn doctor_item(dir: &Path, origin: &str) -> Option<Value> {
    let fake = FakeGithub::start();
    let out = bosun()
        .env("BOSUN_GITHUB_API", fake.url())
        .env("BOSUN_ORIGIN", origin)
        .env("CLAUDE_CONFIG_DIR", dir.join(".test-claude"))
        .arg("-C")
        .arg(dir)
        .args(["doctor", "--full"])
        .output()
        .unwrap();
    let v = one_object(&out.stdout);
    let items = v["data"]["items"].as_array().unwrap().clone();
    items.into_iter().find(|i| i["name"] == "session-rules")
}

#[test]
fn doctor_warns_in_slack_when_the_block_is_missing() {
    let tmp = tempfile::tempdir().unwrap();
    let d = tmp.path();
    let it = doctor_item(d, "slack").expect("item in slack");
    assert_eq!(it["status"], "warn", "{it}");
    assert!(it["fix"].as_str().unwrap().contains("bosun rules --write"));
    assert!(doctor_item(d, "local").is_none());
    rules(d, true);
    assert_eq!(doctor_item(d, "slack").unwrap()["status"], "ok");
}
