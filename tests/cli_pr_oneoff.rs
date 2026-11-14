//! `bosun pr --one-off --title T --touch GLOB`: a task is opened, claimed and its PR created in
//! one step, for work nobody planned (spec §4).

mod common;

use std::process::Command;

use common::fake::FakeGithub;

const T0: &str = "2026-10-23T10:00:00Z";

fn repo(branch: &str) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("a.txt"), "a").unwrap();
    for args in [
        vec!["init", "-q", "-b", branch],
        vec!["add", "."],
        vec!["commit", "-q", "-m", "feat: one-off"],
    ] {
        let st = Command::new("git")
            .arg("-C")
            .arg(dir.path())
            .args(["-c", "user.name=t", "-c", "user.email=t@t"])
            .args(args)
            .status()
            .unwrap();
        assert!(st.success());
    }
    dir
}

#[test]
fn a_one_off_pr_opens_and_claims_its_own_task() {
    let fake = FakeGithub::start();
    let dir = repo("claude/quick-fix");
    let (code, v) = common::run(
        &fake,
        "session_test",
        T0,
        &[
            "-C",
            dir.path().to_str().unwrap(),
            "pr",
            "--one-off",
            "--title",
            "feat: quick fix",
            "--touch",
            "src/core/**",
        ],
    );
    assert_eq!(code, 0, "{v}");
    let n = v["data"]["task"].as_u64().unwrap();
    assert_eq!(
        v["data"]["claim"]["lease"]["touch"],
        serde_json::json!(["src/core/**"])
    );
    let labels = fake.issue_state(n).labels;
    assert!(
        labels.iter().any(|l| l.ends_with("state/review")),
        "{labels:?}"
    );
    let pr = v["data"]["pr"].as_u64().unwrap();
    assert!(fake.pull_body(pr).contains(&format!("#{n}")));
}

#[test]
fn a_one_off_needs_a_title_and_a_touch_and_creates_nothing_without() {
    let fake = FakeGithub::start();
    let dir = repo("claude/quick-fix");
    let (code, v) = common::run(
        &fake,
        "session_test",
        T0,
        &[
            "-C",
            dir.path().to_str().unwrap(),
            "pr",
            "--one-off",
            "--title",
            "feat: x",
        ],
    );
    assert_eq!(code, 2, "{v}");
    assert!(!v["error"]["fix"].as_str().unwrap().is_empty());
}
