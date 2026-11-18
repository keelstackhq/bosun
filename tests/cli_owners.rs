//! `bosun owners set`: an owner PR that rewrites governance/owners.toml (spec §16).
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
        .args(["-c", "user.name=t", "-c", "user.email=t@t"])
        .args(args)
        .output()
        .unwrap();
    assert!(out.status.success(), "git {args:?}: {out:?}");
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

const OWNERS: &str = "governance/owners.toml";

struct Env {
    work: tempfile::TempDir,
    origin: tempfile::TempDir,
}

fn env() -> Env {
    let work = tempfile::tempdir().unwrap();
    let origin = tempfile::tempdir().unwrap();
    git(origin.path(), &["init", "-q", "--bare", "-b", "main"]);
    git(work.path(), &["init", "-q", "-b", "main"]);
    std::fs::create_dir_all(work.path().join("governance")).unwrap();
    std::fs::write(
        work.path().join(OWNERS),
        "[[owner]]\ngithub = \"ada\"\nslack = \"U1\"\n",
    )
    .unwrap();
    git(work.path(), &["add", "-A"]);
    git(work.path(), &["commit", "-q", "-m", "x"]);
    git(
        work.path(),
        &["remote", "add", "origin", origin.path().to_str().unwrap()],
    );
    git(work.path(), &["push", "-q", "origin", "main"]);
    Env { work, origin }
}

fn owners(e: &Env, fake: &FakeGithub, args: &[&str]) -> (i32, Value) {
    let out = bosun()
        .env("BOSUN_GITHUB_API", fake.url())
        .arg("-C")
        .arg(e.work.path())
        .args(["owners", "set"])
        .args(args)
        .output()
        .unwrap();
    let v = one_object(&out.stdout);
    if v["ok"] == false {
        assert!(!v["error"]["fix"].as_str().unwrap().trim().is_empty());
    }
    (out.status.code().unwrap(), v)
}

#[test]
fn set_opens_a_draft_owner_pr_with_users_and_a_team() {
    let fake = FakeGithub::start();
    let e = env();
    let (code, v) = owners(&e, &fake, &["Ada", "bo", "acme/maintainers"]);
    assert_eq!(code, 0, "{v}");
    let d = &v["data"];
    let n = d["pr"]["number"].as_u64().unwrap();
    assert!(fake.pull_is_draft(n));
    let branch = d["branch"].as_str().unwrap();
    assert!(branch.starts_with("bosun/owners"), "{branch}");
    let text = git(e.origin.path(), &["show", &format!("{branch}:{OWNERS}")]);
    // A known login keeps its Slack ID; the team is its own entry.
    assert!(text.contains("github = \"Ada\"\nslack = \"U1\""), "{text}");
    assert!(text.contains("github = \"bo\""), "{text}");
    assert!(text.contains("team = \"acme/maintainers\""), "{text}");
    assert!(
        d["then"]
            .to_string()
            .contains(&format!("bosun pr --pr {n}"))
    );
    assert!(fake.requests().iter().all(|r| !r.path.ends_with("/merge")));
}

#[test]
fn bad_entries_are_usage_errors_and_write_nothing() {
    for args in [
        vec!["other/team"],
        vec!["acme/maintainers"],
        vec!["ada", "x y"],
        vec!["ada", "acme/a/b"],
    ] {
        let fake = FakeGithub::start();
        let e = env();
        let (code, v) = owners(&e, &fake, &args);
        assert_eq!(code, 2, "{args:?}: {v}");
        assert!(
            fake.requests().iter().all(|r| r.method == "GET"),
            "{args:?}"
        );
    }
}

#[test]
fn the_same_owners_change_nothing() {
    let fake = FakeGithub::start();
    let e = env();
    let (code, v) = owners(&e, &fake, &["ada"]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["changed"], false);
    assert!(fake.requests().iter().all(|r| r.method == "GET"));
}
