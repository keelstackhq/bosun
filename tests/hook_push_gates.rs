//! `push_gates = "push"` (spec §7): the PreToolUse hook refuses a push whose tree lacks a
//! passing stamp for a ready gate; `ready` (and no `[mode]`) pushes as today.

mod common;

use std::fs;
use std::path::Path;
use std::process::Command;

use common::bosun;
use common::fake::FakeGithub;
use common::stamps;
use serde_json::{Value, json};

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

/// A repo on branch `feat` with one commit holding `.bosun/bosun.toml` = `config`.
fn repo(config: Option<&str>) -> tempfile::TempDir {
    let tmp = tempfile::tempdir().unwrap();
    let d = tmp.path();
    git(d, &["init", "-q", "-b", "main"]);
    std::fs::create_dir_all(d.join(".bosun")).unwrap();
    std::fs::write(d.join(".bosun/version"), "0.9.0\n").unwrap();
    git(
        d,
        &[
            "remote",
            "add",
            "origin",
            "https://github.com/acme/widget.git",
        ],
    );
    fs::write(d.join("a.txt"), "x").unwrap();
    git(d, &["add", "-A"]);
    git(d, &["commit", "-q", "-m", "base"]);
    git(d, &["update-ref", "refs/remotes/origin/main", "HEAD"]);
    git(d, &["checkout", "-q", "-b", "feat"]);
    if let Some(c) = config {
        fs::create_dir_all(d.join(".bosun")).unwrap();
        fs::write(d.join(".bosun/bosun.toml"), c).unwrap();
    }
    fs::write(d.join("a.txt"), "y").unwrap();
    git(d, &["add", "-A"]);
    git(d, &["commit", "-q", "-m", "work"]);
    tmp
}

/// The hook's deny reason for a Bash `command` run in `cwd`, `None` when it allows it.
fn hook(cwd: &Path, command: &str) -> Option<String> {
    let fake = FakeGithub::start();
    let input = json!({"session_id": "s", "tool_name": "Bash", "cwd": cwd.to_str().unwrap(),
        "tool_input": {"command": command}});
    let out = bosun()
        .current_dir(cwd)
        .env("BOSUN_SESSION", "session_me")
        .env("BOSUN_NOW", "2026-11-12T10:00:00Z")
        .env("BOSUN_GITHUB_API", fake.url())
        .args(["hook", "pre-tool-use"])
        .write_stdin(input.to_string())
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(0));
    let v: Value = serde_json::from_str(&String::from_utf8_lossy(&out.stdout)).ok()?;
    (v["hookSpecificOutput"]["permissionDecision"] == "deny").then(|| {
        v["hookSpecificOutput"]["permissionDecisionReason"]
            .as_str()
            .unwrap()
            .into()
    })
}

const STEADY: &str = "project = \"widget\"\n\n[mode]\nname = \"steady\"\n";
const PUSH: &str = "git push -u origin feat";

#[test]
fn steady_refuses_a_push_without_stamps_and_names_the_gate_loop() {
    let r = repo(Some(STEADY));
    let why = hook(r.path(), PUSH).expect("denied");
    assert!(why.contains("push-gates"), "{why}");
    for g in stamps::READY_GATES {
        assert!(why.contains(g), "{why}");
    }
    assert!(why.contains("bosun gate $g"), "{why}");
}

#[test]
fn steady_allows_a_push_once_every_ready_gate_is_stamped() {
    let r = repo(Some(STEADY));
    stamps::stamp(r.path(), &["fmt", "lint"]);
    let why = hook(r.path(), PUSH).expect("still missing gates");
    assert!(
        why.contains("test-integrity") && !why.contains("lint,"),
        "{why}"
    );
    stamps::stamp_all(r.path());
    assert_eq!(hook(r.path(), PUSH), None);
    // Redirections are not pushed refs.
    assert_eq!(hook(r.path(), &format!("{PUSH} 2>&1 | tail -3")), None);
    assert_eq!(hook(r.path(), "git push -u origin feat:feat > log"), None);
}

#[test]
fn without_mode_or_with_ready_a_push_needs_no_stamps() {
    for config in [
        None,
        Some("project = \"widget\"\n"),
        Some("project = \"widget\"\n\n[mode]\nname = \"steady\"\npush_gates = \"ready\"\n"),
    ] {
        let r = repo(config);
        assert_eq!(hook(r.path(), PUSH), None, "{config:?}");
    }
}

#[test]
fn the_pushed_tree_is_read_where_git_runs_and_from_the_pushed_ref() {
    let r = repo(Some(STEADY));
    let outside = tempfile::tempdir().unwrap();
    let at = r.path().to_str().unwrap();
    for cmd in [
        format!("cd {at} && git push origin feat"),
        format!("git -C {at} push origin feat"),
    ] {
        assert!(hook(outside.path(), &cmd).is_some(), "{cmd}");
    }
    // Stamps for HEAD do not vouch for another pushed branch.
    stamps::stamp_all(r.path());
    git(r.path(), &["branch", "old", "main"]);
    assert_eq!(hook(r.path(), "git push origin feat"), None);
    let why = hook(r.path(), "git push origin old").expect("old is not stamped");
    assert!(why.contains("`old`"), "{why}");
    // A script that pushes counts too.
    fs::write(r.path().join("ship.sh"), "git push origin old\n").unwrap();
    assert!(hook(r.path(), "bash ship.sh").is_some());
}

#[test]
fn a_push_whose_checkout_cannot_be_told_is_refused_in_push_mode() {
    let r = repo(Some(STEADY));
    stamps::stamp_all(r.path());
    for cmd in [
        "python3 -c \"import subprocess; subprocess.run(['git', 'push', 'origin', 'feat:feat'])\"",
        "git --git-dir=../w/.git push origin feat",
    ] {
        let why = hook(r.path(), cmd).expect(cmd);
        assert!(why.contains("cannot tell which checkout"), "{why}");
    }
    let plain = repo(None);
    let py =
        "python3 -c \"import subprocess; subprocess.run(['git', 'push', 'origin', 'feat:feat'])\"";
    assert_eq!(hook(plain.path(), py), None);
}

/// Commit `files` (path, body) on the current branch.
fn commit(dir: &Path, files: &[(&str, &str)]) {
    for (p, body) in files {
        let f = dir.join(p);
        fs::create_dir_all(f.parent().unwrap()).unwrap();
        fs::write(f, body).unwrap();
    }
    git(dir, &["add", "-A"]);
    git(dir, &["commit", "-q", "-m", "c"]);
}

#[test]
fn stamps_carry_over_a_docs_only_diff_since_the_last_stamped_tree() {
    let r = repo(Some(STEADY));
    stamps::stamp_all(r.path());
    commit(
        r.path(),
        &[
            ("docs/guide.md", "a"),
            ("changelog.d/feat.md", "c"),
            ("docs/progress.d/feat.md", "d"),
        ],
    );
    assert_eq!(hook(r.path(), PUSH), None);
    commit(r.path(), &[("docs/more.md", "f")]);
    assert_eq!(hook(r.path(), PUSH), None);
    // Code after the stamp, even under a later docs commit, needs gates again.
    commit(r.path(), &[("src/lib.rs", "fn x() {}")]);
    assert!(hook(r.path(), PUSH).is_some());
    commit(r.path(), &[("docs/after.md", "g")]);
    let why = hook(r.path(), PUSH).expect("code since the stamp");
    assert!(why.contains("push-gates"), "{why}");
    // A non-docs file outside docs/ (even text) is no docs-only change.
    let r = repo(Some(STEADY));
    stamps::stamp_all(r.path());
    // Markdown the tests read (the spec, READMEs, root files, plans) is no docs-only change.
    for f in [
        "docs/notes.txt",
        "README.md",
        "src/a.md",
        "docs/spec/x.md",
        "docs/spec.md",
        "changelog.d/README.md",
        "CHANGELOG.md",
        "plans/p.md",
    ] {
        commit(r.path(), &[(f, "x")]);
        assert!(hook(r.path(), PUSH).is_some(), "{f}");
    }
    // `diff.relative` in a subdirectory push hides no path.
    let r = repo(Some(STEADY));
    stamps::stamp_all(r.path());
    commit(
        r.path(),
        &[("src/lib.rs", "fn x() {}"), ("docs/docs/a.md", "x")],
    );
    git(r.path(), &["config", "diff.relative", "true"]);
    assert!(hook(&r.path().join("docs"), PUSH).is_some());
}

/// The hook's whole output for a push in `cwd` against `api`; `Value::Null` when it prints none.
/// Each call is its own session, so repeat-lock never counts these pushes.
fn hook_at(cwd: &Path, command: &str, api: &str) -> Value {
    static CALLS: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    let n = CALLS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let input = json!({"session_id": format!("s{n}"), "tool_name": "Bash", "cwd": cwd.to_str().unwrap(),
        "tool_input": {"command": command}});
    let out = bosun()
        .current_dir(cwd)
        .env("BOSUN_SESSION", "session_me")
        .env("BOSUN_NOW", "2026-11-12T10:00:00Z")
        .env("BOSUN_GITHUB_API", api)
        .args(["hook", "pre-tool-use"])
        .write_stdin(input.to_string())
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(0));
    serde_json::from_slice(&out.stdout).unwrap_or(Value::Null)
}

fn head_of(dir: &Path) -> String {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(["rev-parse", "HEAD"])
        .output()
        .unwrap();
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

fn verdict_comment(pr: u64, head: &str) -> String {
    format!(
        "Review verdict.\n\n<!-- bosun:verdict -->\n```json\n{{\"pr\":{pr},\"head_sha\":\"{head}\",\
         \"verdict\":\"approve\",\"confidence\":0.9,\"findings\":[]}}\n```"
    )
}

const OLD: &str = "0123456789abcdef0123456789abcdef01234567";

#[test]
fn a_push_past_the_open_prs_verdict_head_warns_without_denying() {
    let r = repo(Some(STEADY));
    stamps::stamp_all(r.path());
    let fake = FakeGithub::start();
    fake.pull(7, "work", "feat", "");
    fake.comment(7, &verdict_comment(7, &"f".repeat(40)));
    fake.comment(7, &verdict_comment(7, OLD));
    let new = head_of(r.path());
    let v = hook_at(r.path(), PUSH, &fake.url());
    let out = &v["hookSpecificOutput"];
    assert_eq!(out["permissionDecision"], Value::Null, "{v}");
    let ctx = out["additionalContext"].as_str().expect("a warning");
    assert!(ctx.contains("PR #7"), "{ctx}");
    assert!(ctx.contains(&OLD[..7]) && ctx.contains(&new[..7]), "{ctx}");
    assert!(ctx.contains("bosun review-loop 7"), "{ctx}");
    // `HEAD` names the current branch's PR too.
    let v = hook_at(r.path(), "git push origin HEAD", &fake.url());
    assert!(
        v["hookSpecificOutput"]["additionalContext"].is_string(),
        "{v}"
    );
    // A deny of an earlier rule still wins.
    commit(r.path(), &[("src/lib.rs", "fn x() {}")]);
    let v = hook_at(r.path(), PUSH, &fake.url());
    assert_eq!(v["hookSpecificOutput"]["permissionDecision"], "deny", "{v}");
}

#[test]
fn a_verdict_at_the_pushed_head_no_verdict_no_pr_or_no_github_warn_nothing() {
    let r = repo(Some(STEADY));
    stamps::stamp_all(r.path());
    let head = head_of(r.path());
    let fake = FakeGithub::start();
    fake.pull(7, "work", "feat", "");
    fake.comment(7, &verdict_comment(7, OLD));
    fake.comment(7, &verdict_comment(7, &head));
    assert_eq!(hook_at(r.path(), PUSH, &fake.url()), Value::Null);
    // Another branch's PR, a PR without a verdict, no GitHub: nothing.
    let fake = FakeGithub::start();
    fake.pull(8, "other", "other", "");
    fake.comment(8, &verdict_comment(8, OLD));
    assert_eq!(hook_at(r.path(), PUSH, &fake.url()), Value::Null);
    let fake = FakeGithub::start();
    fake.pull(7, "work", "feat", "");
    fake.comment(7, "looks fine");
    assert_eq!(hook_at(r.path(), PUSH, &fake.url()), Value::Null);
    assert_eq!(hook_at(r.path(), PUSH, "http://127.0.0.1:9"), Value::Null);
}
