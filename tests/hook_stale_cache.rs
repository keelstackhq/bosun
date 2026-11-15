//! The edit-lock fails closed for paths held in a lease cache older than the lease TTL when
//! GitHub is unreachable (spec §4).
mod common;

use std::path::Path;
use std::process::Command;

use common::fake::FakeGithub;
use common::{bosun, lease_note};
use serde_json::{Value, json};

const T0: &str = "2026-10-22T10:00:00Z";
const LATER: &str = "2026-10-22T14:00:00Z";
const LIVE: &str = "2026-10-22T10:20:00Z";

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
    tmp
}

fn pre(api: &str, repo: &Path, now: &str, file: &str) -> (String, String) {
    let input = json!({"session_id":"s","tool_name":"Edit","tool_input":{"file_path":file},
        "cwd":repo.to_str().unwrap()});
    let out = bosun()
        .current_dir(repo)
        .env("BOSUN_SESSION", "session_me")
        .env("BOSUN_NOW", now)
        .env("BOSUN_GITHUB_API", api)
        .args(["hook", "pre-tool-use"])
        .write_stdin(input.to_string())
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(0));
    (
        String::from_utf8_lossy(&out.stdout).into(),
        String::from_utf8_lossy(&out.stderr).into(),
    )
}

#[test]
fn a_cache_older_than_the_ttl_still_denies_held_paths_and_frees_the_rest() {
    let (fake, tmp) = (FakeGithub::start(), repo());
    fake.issue(4, "other", &["bosun:state/claimed"], "x");
    fake.comment(4, &lease_note("session_other", LIVE, "src/core/**"));
    let (out, _) = pre(&fake.url(), tmp.path(), T0, "src/core/a.rs");
    assert!(out.contains("deny"), "{out}");
    // Hours later (cache and lease both past the TTL), GitHub down.
    let (out, err) = pre("http://127.0.0.1:9", tmp.path(), LATER, "src/core/a.rs");
    let v: Value = common::one_object(out.as_bytes());
    assert_eq!(
        v["hookSpecificOutput"]["permissionDecision"], "deny",
        "{err}"
    );
    assert!(err.contains("warning"), "{err}");
    let (out, _) = pre("http://127.0.0.1:9", tmp.path(), LATER, "docs/x.md");
    assert_eq!(out, "", "paths the cache does not hold stay free");
}
