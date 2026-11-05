//! `bosun wait` marks itself in `<git common dir>/bosun/wait/<pid>.json` while it runs, so the
//! Stop hook does not block a session whose background wait is still going (spec §5).

mod common;

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use common::fake::FakeGithub;
use common::{bosun, one_object};
use serde_json::Value;

fn git(dir: &Path, args: &[&str]) -> String {
    let out = Command::new("git").arg("-C").arg(dir).args(args).output();
    let out = out.expect("git");
    assert!(out.status.success(), "git {args:?}: {out:?}");
    String::from_utf8(out.stdout).unwrap().trim().to_string()
}

fn repo() -> tempfile::TempDir {
    let tmp = tempfile::tempdir().unwrap();
    git(tmp.path(), &["init", "-q", "-b", "main"]);
    tmp
}

fn wait_dir(repo: &Path) -> PathBuf {
    PathBuf::from(git(repo, &["rev-parse", "--absolute-git-dir"])).join("bosun/wait")
}

/// The markers in `dir` as (file name, parsed body).
fn markers(dir: &Path) -> Vec<(String, Value)> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let body = |p: &Path| serde_json::from_str(&std::fs::read_to_string(p).unwrap_or_default());
    entries
        .flatten()
        .map(|e| (e.file_name().to_string_lossy().into_owned(), e.path()))
        .filter_map(|(name, p)| Some((name, body(&p).ok()?)))
        .collect()
}

/// Runs `bosun -C <dir> wait --pr 5 --timeout 2` (nothing changes) in the background and
/// returns the markers seen while it ran, the exit code and the envelope.
fn wait_in(
    fake: &FakeGithub,
    dir: &Path,
    markers_in: &Path,
) -> (Vec<(String, Value)>, i32, String) {
    // A plain Command to run it in the background, with `common::bosun`'s hermetic env.
    let elsewhere = tempfile::tempdir().unwrap();
    let child = Command::new(env!("CARGO_BIN_EXE_bosun"))
        .current_dir(elsewhere.path())
        .env_remove("GH_TOKEN")
        .env("GITHUB_TOKEN", common::TOKEN_CANARY)
        .env("BOSUN_REPO", "acme/widget")
        .env("BOSUN_SESSION", "session_test")
        .env("BOSUN_NOW", "2026-10-22T10:00:00Z")
        .env("BOSUN_ORIGIN", "local")
        .env("BOSUN_GITHUB_API", fake.url())
        .env("BOSUN_WAIT_BASE_MS", "20")
        .arg("-C")
        .arg(dir)
        .args(["wait", "--pr", "5", "--timeout", "2"])
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let (mut seen, start) = (Vec::new(), Instant::now());
    while seen.is_empty() && start.elapsed() < Duration::from_millis(1500) {
        seen = markers(markers_in);
        std::thread::sleep(Duration::from_millis(20));
    }
    let out = child.wait_with_output().unwrap();
    let stdout = String::from_utf8(out.stdout).unwrap();
    (seen, out.status.code().unwrap(), stdout)
}

#[test]
fn a_wait_marks_itself_in_the_c_repo_and_removes_the_marker_on_return() {
    let tmp = repo();
    let fake = FakeGithub::start();
    fake.pull(5, "Work", "claude/work", "");
    let dir = wait_dir(tmp.path());
    let (seen, code, stdout) = wait_in(&fake, tmp.path(), &dir);
    assert_eq!(code, 0, "{stdout}");
    assert_eq!(one_object(stdout.as_bytes())["data"]["changed"], false);
    assert_eq!(
        seen.len(),
        1,
        "no marker while waiting in {}",
        dir.display()
    );
    let (name, body) = &seen[0];
    assert_eq!(name, &format!("{}.json", body["pid"]));
    assert_eq!(body["session"], "session_test");
    assert!(body["started"].as_u64().unwrap() > 0, "{body}");
    assert!(markers(&dir).is_empty(), "marker left behind");
}

#[test]
fn a_wait_in_a_linked_worktree_marks_the_common_git_dir() {
    let tmp = repo();
    git(tmp.path(), &["commit", "-q", "--allow-empty", "-m", "x"]);
    let wt = tmp.path().join("wt");
    git(tmp.path(), &["worktree", "add", "-q", wt.to_str().unwrap()]);
    let fake = FakeGithub::start();
    fake.pull(5, "Work", "claude/work", "");
    // The Stop hook may run in the main checkout while the wait runs in the worktree.
    let (seen, code, stdout) = wait_in(&fake, &wt, &wait_dir(tmp.path()));
    assert_eq!(code, 0, "{stdout}");
    assert_eq!(seen.len(), 1, "no marker in the common git dir");
}

#[test]
fn a_marker_that_cannot_be_written_does_not_fail_the_wait() {
    let tmp = repo();
    let dir = wait_dir(tmp.path());
    std::fs::create_dir_all(dir.parent().unwrap()).unwrap();
    std::fs::write(&dir, "not a directory").unwrap();
    let fake = FakeGithub::start();
    fake.pull(5, "Work", "claude/work", "");
    let out = bosun()
        .env("BOSUN_GITHUB_API", fake.url())
        .env("BOSUN_WAIT_BASE_MS", "20")
        .arg("-C")
        .arg(tmp.path())
        .args(["wait", "--pr", "5", "--timeout", "1"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(0));
    assert_eq!(one_object(&out.stdout)["data"]["changed"], false);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("wait marker"), "{stderr}");
}
