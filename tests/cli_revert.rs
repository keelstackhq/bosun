mod common;

use std::path::Path;
use std::process::Command;

use common::fake::FakeGithub;
use common::{bosun, one_object};
use serde_json::Value;

fn git(dir: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .env("GIT_AUTHOR_NAME", "t")
        .env("GIT_AUTHOR_EMAIL", "t@t")
        .env("GIT_COMMITTER_NAME", "t")
        .env("GIT_COMMITTER_EMAIL", "t@t")
        .output()
        .unwrap();
    assert!(out.status.success(), "git {args:?}: {out:?}");
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

struct Repos {
    _tmp: tempfile::TempDir,
    origin: std::path::PathBuf,
    clone: std::path::PathBuf,
    sha5: String,
}

/// A bare origin with main: initial, #5 adds `broken`, #6 adds `ok.txt`; and a clone of it.
fn repos() -> Repos {
    let tmp = tempfile::tempdir().unwrap();
    let origin = tmp.path().join("origin.git");
    let clone = tmp.path().join("clone");
    std::fs::create_dir(&origin).unwrap();
    git(&origin, &["init", "-q", "--bare", "-b", "main"]);
    let url = format!("file://{}", origin.display());
    Command::new("git")
        .args(["clone", "-q", &url])
        .arg(&clone)
        .output()
        .unwrap();
    git(&clone, &["checkout", "-q", "-B", "main"]);
    let mut sha5 = String::new();
    for (file, msg) in [
        ("init.txt", "initial"),
        ("broken", "PR 5"),
        ("ok.txt", "PR 6"),
    ] {
        std::fs::write(clone.join(file), "x\n").unwrap();
        git(&clone, &["add", "-A"]);
        git(&clone, &["commit", "-q", "-m", msg]);
        if file == "broken" {
            sha5 = git(&clone, &["rev-parse", "HEAD"]);
        }
    }
    git(&clone, &["push", "-q", "-u", "origin", "main"]);
    Repos {
        _tmp: tmp,
        origin,
        clone,
        sha5,
    }
}

fn run(fake: &FakeGithub, dir: &Path) -> (i32, Value) {
    let out = bosun()
        .env("BOSUN_GITHUB_API", fake.url())
        .env("GIT_AUTHOR_NAME", "t")
        .env("GIT_AUTHOR_EMAIL", "t@t")
        .env("GIT_COMMITTER_NAME", "t")
        .env("GIT_COMMITTER_EMAIL", "t@t")
        .arg("-C")
        .arg(dir)
        .args(["revert", "5"])
        .output()
        .unwrap();
    (out.status.code().unwrap(), one_object(&out.stdout))
}

fn creates(fake: &FakeGithub) -> usize {
    fake.requests()
        .iter()
        .filter(|r| r.method == "POST" && r.path.ends_with("/pulls"))
        .count()
}

#[test]
fn uc_m3_4_revert_opens_a_mainfix_pr_that_turns_main_green() {
    let r = repos();
    let main_before = git(&r.origin, &["rev-parse", "main"]);
    let fake = FakeGithub::start();
    fake.pull(5, "Add broken", "claude/five", "");
    fake.pull_merged(5, &r.sha5);
    fake.issue(9, "main is red", &["bosun:main-red"], "");
    let (code, v) = run(&fake, &r.clone);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["branch"], "bosun/revert-5");
    assert_eq!(v["data"]["main_red"], 9);
    assert_eq!(creates(&fake), 1);
    let rq = fake.requests();
    let create = rq
        .iter()
        .find(|q| q.method == "POST" && q.path.ends_with("/pulls"))
        .unwrap();
    let body: Value = serde_json::from_str(create.body.as_ref().unwrap()).unwrap();
    assert_eq!(body["head"], "bosun/revert-5");
    assert_eq!(body["title"], "Revert #5: Add broken");
    assert!(body["body"].as_str().unwrap().contains("main-red: #9"));
    assert!(
        body["body"]
            .as_str()
            .unwrap()
            .contains("<!-- bosun:revert 5 -->")
    );
    let label = rq
        .iter()
        .find(|q| q.method == "POST" && q.path.ends_with("/labels"))
        .expect("label call");
    assert!(label.body.as_ref().unwrap().contains("bosun:kind/mainfix"));
    // The new branch on origin is green: `broken` is gone, `ok.txt` stays.
    let files = git(&r.origin, &["ls-tree", "--name-only", "bosun/revert-5"]);
    assert!(!files.lines().any(|f| f == "broken"), "{files}");
    assert!(files.lines().any(|f| f == "ok.txt"), "{files}");
    // The default branch is untouched.
    assert_eq!(git(&r.origin, &["rev-parse", "main"]), main_before);
}

#[test]
fn revert_of_an_unmerged_pr_is_a_conflict_and_pushes_nothing() {
    let r = repos();
    let fake = FakeGithub::start();
    fake.pull(5, "Add broken", "claude/five", "");
    let (code, v) = run(&fake, &r.clone);
    assert_eq!(code, 3, "{v}");
    assert_eq!(v["error"]["fix"], "only a merged PR can be reverted");
    assert_eq!(creates(&fake), 0);
    let branches = git(&r.origin, &["branch", "--list"]);
    assert!(!branches.contains("revert"), "{branches}");
}
