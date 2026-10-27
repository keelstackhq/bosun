//! The edit lock keeps a session with a lease inside its own touch (spec §4 layer 3).

mod common;

use std::fs;
use std::path::Path;
use std::process::Command;

use common::fake::FakeGithub;
use common::{bosun, lease_note};
use serde_json::{Value, json};

const LIVE: &str = "2026-10-22T10:20:00Z";

fn git(dir: &Path, args: &[&str]) {
    let cfg = ["-c", "user.name=t", "-c", "user.email=t@example.com"];
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(cfg)
        .args(args)
        .output()
        .unwrap();
    assert!(out.status.success(), "git {args:?}: {out:?}");
}

/// A repo with one commit that `origin/main` points at, and a fake GitHub where `holder` has
/// leased `touch` (a glob, or several joined as `a","b`) on issue 5.
fn setup(holder: &str, touch: &str) -> (FakeGithub, tempfile::TempDir) {
    let (fake, tmp) = (FakeGithub::start(), tempfile::tempdir().unwrap());
    let d = tmp.path();
    git(d, &["init", "-q", "-b", "main"]);
    git(
        d,
        &[
            "remote",
            "add",
            "origin",
            "https://github.com/acme/widget.git",
        ],
    );
    for f in ["src/core/a.rs", "src/cli/b.rs", "Cargo.toml", "Cargo.lock"] {
        fs::create_dir_all(d.join(f).parent().unwrap()).unwrap();
        fs::write(d.join(f), "x").unwrap();
    }
    git(d, &["add", "-A"]);
    git(d, &["commit", "-q", "-m", "base"]);
    git(d, &["update-ref", "refs/remotes/origin/main", "HEAD"]);
    fake.issue(5, "mine", &["bosun:state/claimed"], "x");
    fake.comment(5, &lease_note(holder, LIVE, touch));
    (fake, tmp)
}

/// Run the hook as `session_me` for an Edit of `rel`; `Some(reason)` when it denies.
fn edit(fake: &FakeGithub, repo: &Path, rel: &str) -> Option<String> {
    let input = json!({"session_id": "s", "tool_name": "Edit", "cwd": repo.to_str().unwrap(),
        "tool_input": {"file_path": repo.join(rel).to_str().unwrap()}});
    let out = bosun()
        .current_dir(repo)
        .env("BOSUN_SESSION", "session_me")
        .env("BOSUN_NOW", "2026-10-22T10:00:00Z")
        .env("BOSUN_GITHUB_API", fake.url())
        .args(["hook", "pre-tool-use"])
        .write_stdin(input.to_string())
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&out.stdout);
    if stdout.is_empty() {
        assert_eq!(String::from_utf8_lossy(&out.stderr), "", "{rel}");
        return None;
    }
    let v: Value = serde_json::from_str(&stdout).expect("one JSON object");
    assert_eq!(v["hookSpecificOutput"]["permissionDecision"], "deny");
    Some(
        v["hookSpecificOutput"]["permissionDecisionReason"]
            .as_str()
            .unwrap()
            .into(),
    )
}

#[test]
fn edits_stay_inside_the_own_touch() {
    let (fake, tmp) = setup("session_me", "src/core/**\",\"Cargo.toml");
    let r = tmp.path();
    // Inside the touch, a lockfile companion, a new file, a fragment: allowed.
    for rel in [
        "src/core/a.rs",
        "Cargo.lock",
        "src/cli/new.rs",
        "changelog.d/x.md",
    ] {
        assert_eq!(edit(&fake, r, rel), None, "{rel}");
    }
    // Outside it, also spelled with `..`: denied, naming the globs and the fix.
    for rel in ["src/cli/b.rs", "src/core/../cli/b.rs"] {
        let why = edit(&fake, r, rel).expect(rel);
        assert!(
            why.starts_with("src/cli/b.rs is outside your lease (touch: src/core/**, Cargo.toml)")
                && why.contains("bosun claim 5 --touch src/cli/b.rs"),
            "{why}"
        );
    }
}

#[test]
fn only_a_lease_of_the_plain_session_restricts() {
    // No lease at all, and a wave orchestrator holding only per-task leases: free.
    let (fake, tmp) = setup("session_me/x", "src/core/**");
    assert_eq!(edit(&fake, tmp.path(), "src/cli/b.rs"), None);
    let (fake, tmp) = setup("session_other", "src/cli/**");
    assert!(
        edit(&fake, tmp.path(), "src/cli/b.rs")
            .unwrap()
            .contains("claimed")
    );
}

#[test]
fn a_stale_cache_is_rechecked_before_denying() {
    let (fake, tmp) = setup("session_me", "src/core/**");
    let r = tmp.path();
    assert!(edit(&fake, r, "src/cli/b.rs").is_some()); // fills the cache
    // The claim grows on GitHub; the cache still holds the old touch.
    fake.comment(
        5,
        &lease_note("session_me", LIVE, "src/core/**\",\"src/cli/b.rs"),
    );
    assert_eq!(edit(&fake, r, "src/cli/b.rs"), None);
    assert!(edit(&fake, r, "Cargo.toml").is_some()); // still outside the grown touch
}

#[test]
fn a_file_counts_as_new_unless_it_is_on_the_base_branch() {
    let (fake, tmp) = setup("session_me", "src/core/**");
    let r = tmp.path();
    // Written, then committed on a branch: not on origin/main, so still editable.
    fs::write(r.join("src/cli/new.rs"), "x").unwrap();
    assert_eq!(edit(&fake, r, "src/cli/new.rs"), None);
    git(r, &["checkout", "-q", "-b", "work"]);
    git(r, &["add", "-A"]);
    git(r, &["commit", "-q", "-m", "add new.rs"]);
    assert_eq!(edit(&fake, r, "src/cli/new.rs"), None);
    // Without any base ref nothing can be called existing (fail open).
    assert!(edit(&fake, r, "src/cli/b.rs").is_some());
    git(r, &["update-ref", "-d", "refs/remotes/origin/main"]);
    assert_eq!(edit(&fake, r, "src/cli/b.rs"), None);
}

#[cfg(unix)]
#[test]
fn paths_are_judged_by_their_real_target() {
    let (fake, tmp) = setup("session_me", "src/core/**");
    let r = tmp.path();
    // A link inside the touch to a file outside it: the real target decides.
    std::os::unix::fs::symlink(r.join("src/cli/b.rs"), r.join("src/core/l.rs")).unwrap();
    let why = edit(&fake, r, "src/core/l.rs").expect("link to a file outside the touch");
    assert!(
        why.starts_with("src/cli/b.rs is outside your lease"),
        "{why}"
    );
    // `link/../x`: `..` follows the real directory (src/cli), not the lexical one.
    std::os::unix::fs::symlink(r.join("src/cli"), r.join("src/core/link")).unwrap();
    let why = edit(&fake, r, "src/core/link/../cli/b.rs");
    assert!(why.is_some_and(|w| w.starts_with("src/cli/b.rs is outside")));
    assert_eq!(edit(&fake, r, "src/core/link/../core/a.rs"), None);
    // A link to a file outside the repo counts as outside: allowed.
    let out = tempfile::tempdir().unwrap();
    fs::write(out.path().join("o.rs"), "x").unwrap();
    std::os::unix::fs::symlink(out.path().join("o.rs"), r.join("src/cli/o.rs")).unwrap();
    assert_eq!(edit(&fake, r, "src/cli/o.rs"), None);
}
