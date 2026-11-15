//! `bosun pr --one-off` stays inside the base autonomous allow set and outside gated paths; a
//! one-off PR over 300 changed lines is not made ready (spec §4, §8).

mod common;

use std::process::Command;

use common::fake::FakeGithub;
use common::stamps;

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

fn one_off(fake: &FakeGithub, dir: &std::path::Path, touch: &[&str]) -> (i32, serde_json::Value) {
    let mut args = vec![
        "-C",
        dir.to_str().unwrap(),
        "pr",
        "--one-off",
        "--title",
        "feat: quick fix",
    ];
    for t in touch {
        args.extend(["--touch", t]);
    }
    common::run(fake, "session_test", T0, &args)
}

fn created_issues(fake: &FakeGithub) -> usize {
    fake.requests()
        .iter()
        .filter(|r| r.method == "POST" && r.path.ends_with("/issues"))
        .count()
}

#[test]
fn a_one_off_touch_outside_the_base_allow_set_or_in_a_gated_path_is_refused() {
    for touch in [
        "Cargo.toml",
        "**",
        "scripts/x.sh",
        "docs/agents/x.md",
        ".github/workflows/ci.yml",
        "src/<!--x",
        "docs/-->x",
    ] {
        let fake = FakeGithub::start();
        let dir = repo("claude/quick-fix");
        let (code, v) = one_off(&fake, dir.path(), &["src/ok.rs", touch]);
        assert_ne!(code, 0, "{touch}: {v}");
        assert!(!v["error"]["fix"].as_str().unwrap().is_empty(), "{touch}");
        assert_eq!(created_issues(&fake), 0, "{touch} created a task");
    }
}

#[test]
fn the_owner_widens_the_one_off_allow_set_with_autonomous_allow_on_the_base() {
    let fake = FakeGithub::start();
    let dir = repo("claude/quick-fix");
    let (code, _) = one_off(&fake, dir.path(), &["other/x.rs"]);
    assert_ne!(code, 0);
    let fake = FakeGithub::start();
    fake.content(
        ".bosun/bosun.toml",
        "main",
        "project = \"widget\"\n\n[approvals]\nautonomous_allow = [\"other/**\"]\n",
    );
    let (code, v) = one_off(&fake, dir.path(), &["other/x.rs"]);
    assert_eq!(code, 0, "{v}");
}

#[test]
fn a_one_off_pr_over_300_changed_lines_is_not_made_ready() {
    let review = format!("bosun:{}/review", "state");
    for (lines, ok) in [(300, true), (301, false)] {
        let fake = FakeGithub::start();
        let body =
            "<!-- bosun:task {\"item\":\"oneoff\",\"task\":\"x\",\"touch\":[\"src/**\"]} -->";
        fake.issue(7, "the task", &[review.as_str()], body);
        fake.enforce_checks(&["ci-ok", "risk-check", "review-verdict", "main-open"]);
        fake.pull(20, "t", "claude/t7", "Closes #7");
        fake.pull_set(20, "draft", true.into());
        fake.pull_file(20, "src/a.rs", "modified", lines - 100, 100, Some("@@\n+x"));
        let dir = repo("claude/t7");
        stamps::ready(&fake, dir.path(), 20);
        let (code, v) = common::run(
            &fake,
            "session_test",
            T0,
            &["-C", dir.path().to_str().unwrap(), "pr", "7", "--ready"],
        );
        assert_eq!(code == 0, ok, "{lines}: {v}");
        if !ok {
            assert_eq!(code, 3, "{v}");
            assert!(!v["error"]["fix"].as_str().unwrap().is_empty());
            assert!(fake.pull_is_draft(20));
        }
    }
}
