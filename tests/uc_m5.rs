//! Use cases of M5 (spec §12): the learning loop.
mod common;

use common::fake::FakeGithub;
use common::{bosun, one_object, run};

fn capture(fake: &FakeGithub, text: &str) -> serde_json::Value {
    let out = bosun()
        .env("BOSUN_GITHUB_API", fake.url())
        .args(["lesson", "capture", "--kind", "stop-rule", "--issue", "7"])
        .write_stdin(text.to_string())
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(0));
    one_object(&out.stdout)
}

/// UC-M5-1: a stop rule opens a lesson; the same signature again adds a hit, not a new issue.
#[test]
fn uc_m5_1() {
    let fake = FakeGithub::start();
    let first = capture(&fake, "stop rule: no progress\nsha 3f9c2ab81d04\n");
    let second = capture(&fake, "stop rule: no progress\nsha 77aa88bb99cc\n");
    assert_eq!(first["data"]["created"], true);
    assert_eq!(second["data"]["created"], false);
    assert_eq!(second["data"]["issue"], first["data"]["issue"]);
    let n = first["data"]["issue"].as_u64().unwrap();
    let seen: Vec<_> = fake
        .comments_of(n)
        .into_iter()
        .filter(|c| c.body.contains(r#""bosun":"lesson-seen""#))
        .collect();
    assert_eq!(seen.len(), 1);
}

/// UC-M5-2: a blocking lesson sets process-red, and waves pause.
#[test]
fn uc_m5_2() {
    let now = "2026-10-23T10:00:00Z";
    let fake = FakeGithub::start();
    fake.issue(10, "Epic", &["bosun:state/claimed"], "");
    fake.issue(
        11,
        "task",
        &["bosun:state/ready"],
        r#"<!-- bosun:task {"item":"M5","task":"T1","epic":10,"touch":["src/a.rs"]} -->"#,
    );
    fake.issue(
        20,
        "lesson",
        &["bosun:lesson", "bosun:lesson/blocking", "bosun:process-red"],
        "",
    );
    let (code, v) = run(&fake, "session_a", now, &["wave", "plan", "10"]);
    assert_eq!(code, 4, "{v}");
    let (code, v) = run(&fake, "session_a", now, &["status"]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["process_red"], 20);
}

/// UC-M5-3: a pack includes only lessons whose scope matches the task paths, plus global ones.
#[test]
fn uc_m5_3() {
    let fake = FakeGithub::start();
    fake.issue(7, "task", &[], "Do it.");
    fake.pull_as_issue(42, "the pr");
    fake.pull(42, "the pr", "claude/x", "Closes #7");
    fake.pull_file(42, "src/core/a.rs", "modified", 2, 1, Some("@@ a"));
    fake.content(
        "docs/lessons.toml",
        "base0000",
        "[[lesson]]\nid = \"L-001\"\nrule = \"global\"\n\n[[lesson]]\nid = \"L-002\"\nrule = \"core\"\nscope = [\"src/core/**\"]\n\n[[lesson]]\nid = \"L-003\"\nrule = \"web\"\nscope = [\"web/**\"]\n",
    );
    let (code, v) = run(&fake, "s", "2026-10-23T10:00:00Z", &["review-pack", "42"]);
    assert_eq!(code, 0, "{v}");
    let ids: Vec<&str> = v["data"]["lessons"]
        .as_array()
        .unwrap()
        .iter()
        .map(|l| l["id"].as_str().unwrap())
        .collect();
    assert_eq!(ids, ["L-001", "L-002"]);
}

/// UC-M5-4: at 40 entries, bosun lesson add refuses with exit 6.
#[test]
fn uc_m5_4() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(tmp.path().join("docs")).unwrap();
    let mut text = String::new();
    for i in 1..=40 {
        text.push_str(&format!(
            "[[lesson]]\nid = \"L-{i:03}\"\nrule = \"r{i}\"\nscope = [\"a/**\"]\n\n"
        ));
    }
    let file = tmp.path().join("docs/lessons.toml");
    std::fs::write(&file, &text).unwrap();
    let out = bosun()
        .arg("-C")
        .arg(tmp.path())
        .args(["lesson", "add", "x", "--scope", "a/**"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(6));
    assert_eq!(one_object(&out.stdout)["exit_code"], 6);
    assert_eq!(std::fs::read_to_string(&file).unwrap(), text);
}

/// UC-M5-5: a hit on a mechanized lesson reopens it as blocking.
#[test]
fn uc_m5_5() {
    let fake = FakeGithub::start();
    let first = capture(&fake, "stop rule: no progress\nsha 3f9c2ab81d04\n");
    let n = first["data"]["issue"].as_u64().unwrap();
    fake.close_issue(n, "2026-10-20T09:00:00Z");
    let again = capture(&fake, "stop rule: no progress\nsha 77aa88bb99cc\n");
    assert_eq!(again["data"]["issue"].as_u64(), Some(n));
    assert_eq!(again["data"]["reopened"], true);
    let i = fake.issue_state(n);
    assert_eq!(i.state, "open");
    assert!(i.labels.contains(&"bosun:lesson/blocking".to_string()));
    assert!(i.labels.contains(&"bosun:process-red".to_string()));
}
