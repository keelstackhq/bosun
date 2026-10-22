mod common;

use common::fake::FakeGithub;
use common::{bosun, one_object};
use serde_json::{Value, json};

const T0: &str = "2026-10-22T10:00:00Z";

fn lesson(dir: &std::path::Path, fake: &FakeGithub, args: &[&str]) -> (i32, Value) {
    let out = bosun()
        .env("BOSUN_GITHUB_API", fake.url())
        .env("BOSUN_NOW", T0)
        .arg("-C")
        .arg(dir)
        .arg("lesson")
        .args(args)
        .output()
        .unwrap();
    let v = one_object(&out.stdout);
    if v["ok"] == false {
        assert!(!v["error"]["fix"].as_str().unwrap().trim().is_empty());
    }
    (out.status.code().unwrap(), v)
}

fn file(dir: &std::path::Path) -> toml::Table {
    let text = std::fs::read_to_string(dir.join("docs/lessons.toml")).unwrap();
    text.parse().unwrap()
}

#[test]
fn add_creates_the_file_with_a_header_and_next_ids() {
    let (tmp, fake) = (tempfile::tempdir().unwrap(), FakeGithub::start());
    let args = [
        "add",
        "Run metadata once",
        "--scope",
        "src/gates/**",
        "--why",
        "slow",
    ];
    let (code, v) = lesson(tmp.path(), &fake, &args);
    assert_eq!(code, 0, "{v}");
    assert_eq!(
        v["data"],
        json!({"id": "L-001", "count": 1, "global_count": 0})
    );
    let (_, v) = lesson(
        tmp.path(),
        &fake,
        &["add", "Global rule", "--evidence", "#1"],
    );
    assert_eq!(
        v["data"],
        json!({"id": "L-002", "count": 2, "global_count": 1})
    );
    let raw = std::fs::read_to_string(tmp.path().join("docs/lessons.toml")).unwrap();
    assert!(raw.starts_with("# bosun lessons store, spec §6"));
    let t = file(tmp.path());
    let l = t["lesson"].as_array().unwrap();
    assert_eq!(l[0]["added"].as_str(), Some("2026-10-22"));
    assert_eq!(l[1]["evidence"][0].as_str(), Some("#1"));
}

#[test]
fn caps_exit_6_with_the_fix() {
    let (tmp, fake) = (tempfile::tempdir().unwrap(), FakeGithub::start());
    for i in 0..8 {
        let (code, _) = lesson(tmp.path(), &fake, &["add", &format!("g{i}")]);
        assert_eq!(code, 0);
    }
    let (code, v) = lesson(tmp.path(), &fake, &["add", "one global too many"]);
    assert_eq!(code, 6, "{v}");
    let fix = v["error"]["fix"].as_str().unwrap();
    assert!(fix.contains("mechanize, merge or retire"));
    for i in 0..32 {
        let name = format!("s{i}");
        let (code, _) = lesson(tmp.path(), &fake, &["add", &name, "--scope", "a/**"]);
        assert_eq!(code, 0);
    }
    let (code, _) = lesson(tmp.path(), &fake, &["add", "x", "--scope", "a/**"]);
    assert_eq!(code, 6);
    assert_eq!(file(tmp.path())["lesson"].as_array().unwrap().len(), 40);
}

#[test]
fn hit_counts_and_posts_a_record_with_issue() {
    let (tmp, fake) = (tempfile::tempdir().unwrap(), FakeGithub::start());
    fake.issue(7, "task", &[], "");
    lesson(tmp.path(), &fake, &["add", "r"]);
    let (code, v) = lesson(tmp.path(), &fake, &["hit", "L-001"]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["hits"], 1);
    let (code, _) = lesson(tmp.path(), &fake, &["hit", "L-001", "--issue", "7"]);
    assert_eq!(code, 0);
    let t = file(tmp.path());
    assert_eq!(t["lesson"][0]["hits"].as_integer(), Some(2));
    assert_eq!(t["lesson"][0]["last_hit"].as_str(), Some("2026-10-22"));
    let comments = fake.comments_of(7);
    assert_eq!(comments.len(), 1);
    assert!(comments[0].body.contains(r#""bosun":"lesson-hit""#));
    assert!(comments[0].body.contains(r#""id":"L-001""#));
}

#[test]
fn unknown_id_is_exit_1() {
    let (tmp, fake) = (tempfile::tempdir().unwrap(), FakeGithub::start());
    let (code, v) = lesson(tmp.path(), &fake, &["hit", "L-009"]);
    assert_eq!(code, 1, "{v}");
}

#[test]
fn hit_with_issue_obeys_the_kill_switch() {
    let (tmp, fake) = (tempfile::tempdir().unwrap(), FakeGithub::start());
    fake.issue(7, "task", &[], "");
    fake.variable("BOSUN_HALT", "1");
    lesson(tmp.path(), &fake, &["add", "r"]);
    let (code, _) = lesson(tmp.path(), &fake, &["hit", "L-001", "--issue", "7"]);
    assert_eq!(code, 4);
    assert_eq!(file(tmp.path())["lesson"][0]["hits"].as_integer(), Some(0));
}
