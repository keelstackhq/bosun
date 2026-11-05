mod common;

use std::path::Path;
use std::process::Command;

use common::fake::FakeGithub;
use common::{TOKEN_CANARY, bosun, lease_note};
use serde_json::{Value, json};

const T0: &str = "2026-10-22T10:00:00Z";
const T1: &str = "2026-10-22T10:00:30Z";
const T2: &str = "2026-10-22T10:02:00Z";
const LIVE: &str = "2026-10-22T10:20:00Z";

fn git(dir: &Path, args: &[&str]) {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .output()
        .expect("git");
    assert!(out.status.success(), "git {args:?}: {out:?}");
}

fn repo() -> tempfile::TempDir {
    let tmp = tempfile::tempdir().unwrap();
    git(tmp.path(), &["init", "-q", "-b", "main"]);
    let url = "https://github.com/acme/widget.git";
    git(tmp.path(), &["remote", "add", "origin", url]);
    tmp
}

/// Issue 4 holds a live lease of `session_other` on `src/core/**`; issue 5 is `session_me`'s.
fn leased(fake: &FakeGithub) {
    fake.issue(4, "other", &["bosun:state/claimed"], "x");
    fake.comment(4, &lease_note("session_other", LIVE, "src/core/**"));
    fake.issue(5, "mine", &["bosun:state/claimed"], "x");
    fake.comment(5, &lease_note("session_me", LIVE, "docs/**"));
}

struct Out {
    code: i32,
    stdout: String,
    stderr: String,
}

fn hook(fake: Option<&FakeGithub>, repo: &Path, now: &str, sub: &str, input: &Value) -> Out {
    hook_as("session_me", fake, repo, now, sub, input)
}

fn hook_as(
    session: &str,
    fake: Option<&FakeGithub>,
    repo: &Path,
    now: &str,
    sub: &str,
    input: &Value,
) -> Out {
    let mut cmd = bosun();
    cmd.current_dir(repo)
        .env("BOSUN_SESSION", session)
        .env("BOSUN_NOW", now)
        .args(["hook", sub])
        .write_stdin(input.to_string());
    if let Some(f) = fake {
        cmd.env("BOSUN_GITHUB_API", f.url());
    }
    let out = cmd.output().unwrap();
    let o = Out {
        code: out.status.code().unwrap(),
        stdout: String::from_utf8_lossy(&out.stdout).into(),
        stderr: String::from_utf8_lossy(&out.stderr).into(),
    };
    assert!(!format!("{}{}", o.stdout, o.stderr).contains(TOKEN_CANARY));
    o
}

fn call(repo: &Path, tool: &str, input: Value) -> Value {
    json!({"session_id":"s","tool_name":tool,"tool_input":input,"cwd":repo.to_str().unwrap()})
}

fn pre(fake: &FakeGithub, repo: &Path, tool: &str, input: Value) -> Out {
    hook(
        Some(fake),
        repo,
        T0,
        "pre-tool-use",
        &call(repo, tool, input),
    )
}

fn denied(o: &Out) -> String {
    assert_eq!(o.code, 0, "{}", o.stderr);
    let v: Value = serde_json::from_str(&o.stdout).expect("one JSON object");
    assert_eq!(v.as_object().unwrap().len(), 1);
    let h = &v["hookSpecificOutput"];
    assert_eq!(h.as_object().unwrap().len(), 3);
    assert_eq!(h["hookEventName"], "PreToolUse");
    assert_eq!(h["permissionDecision"], "deny");
    let reason = h["permissionDecisionReason"].as_str().unwrap();
    assert!(!reason.is_empty());
    reason.to_string()
}

fn allowed(o: &Out) {
    assert_eq!(o.code, 0, "{}", o.stderr);
    assert_eq!(o.stdout, "", "allow prints nothing");
}

#[test]
fn edit_in_another_sessions_lease_is_denied() {
    let (fake, tmp) = (FakeGithub::start(), repo());
    leased(&fake);
    let abs = tmp.path().join("src/core/guard.rs");
    let input = json!({"file_path": abs.to_str().unwrap()});
    let reason = denied(&pre(&fake, tmp.path(), "Edit", input));
    assert!(
        reason.contains("session_other") && reason.contains("#4"),
        "{reason}"
    );
    let input = json!({"file_path": "src/core/x.rs"});
    denied(&pre(&fake, tmp.path(), "Write", input));
}

#[test]
fn own_lease_unrelated_and_outside_paths_are_allowed() {
    let (fake, tmp) = (FakeGithub::start(), repo());
    leased(&fake);
    for p in ["docs/spec.md", "src/cli/mod.rs", "/etc/hosts"] {
        allowed(&pre(&fake, tmp.path(), "Edit", json!({"file_path": p})));
    }
    let read = json!({"file_path": "src/core/a.rs"});
    allowed(&pre(&fake, tmp.path(), "Read", read));
}

#[test]
fn per_task_lease_of_my_own_session_is_mine_but_blocks_other_tasks() {
    let (fake, tmp) = (FakeGithub::start(), repo());
    fake.issue(19, "task", &["bosun:state/claimed"], "x");
    fake.comment(19, &lease_note("session_me/19", LIVE, "crates/**"));
    let edit = json!({"file_path": "crates/cli/src/run.rs"});
    allowed(&pre(&fake, tmp.path(), "Edit", edit.clone()));
    // Another task of the same orchestrator is a different session and still conflicts.
    let input = call(tmp.path(), "Edit", edit);
    let other = hook_as(
        "session_me/20",
        Some(&fake),
        tmp.path(),
        T0,
        "pre-tool-use",
        &input,
    );
    denied(&other);
}

#[test]
fn read_only_gh_tools_may_filter_on_a_state_label() {
    let (fake, tmp) = (FakeGithub::start(), repo());
    let filter = json!({"labels": ["bosun:state/ready"]});
    allowed(&pre(
        &fake,
        tmp.path(),
        "mcp__github__list_issues",
        filter.clone(),
    ));
    denied(&pre(&fake, tmp.path(), "mcp__github__issue_write", filter));
    let doc = "cat >> docs/x.md <<EOF\nThe bosun:state/ready label (see `bosun status`).\nEOF";
    let bash = |c: &str| pre(&fake, tmp.path(), "Bash", json!({"command": c}));
    allowed(&bash(doc));
    denied(&bash(
        "cat <<EOF\n`gh issue edit 4 --add-label bosun:state/merged`\nEOF",
    ));
}

#[test]
fn label_and_merge_locks_deny_without_the_network() {
    let (fake, tmp) = (FakeGithub::start(), repo());
    let bash = |c: &str| pre(&fake, tmp.path(), "Bash", json!({"command": c}));
    denied(&bash("gh issue edit 4 --add-label bosun:state/merged"));
    denied(&bash("gh pr merge 3"));
    denied(&bash("git push --force"));
    allowed(&bash("bosun release 4"));
    allowed(&bash("ls"));
    let merge = pre(
        &fake,
        tmp.path(),
        "mcp__github__merge_pull_request",
        json!({"pullNumber": 3}),
    );
    denied(&merge);
    let label = json!({"labels": ["bosun:state/merged"]});
    denied(&pre(&fake, tmp.path(), "mcp__github__issue_write", label));
    assert!(
        fake.requests().is_empty(),
        "label and merge rules skip GitHub"
    );
}

#[test]
fn second_call_within_the_cache_age_makes_no_request() {
    let (fake, tmp) = (FakeGithub::start(), repo());
    leased(&fake);
    let edit = json!({"file_path": "src/core/a.rs"});
    denied(&pre(&fake, tmp.path(), "Edit", edit.clone()));
    let first = fake.requests().len();
    assert!(first > 0);
    let c = call(tmp.path(), "Edit", edit);
    denied(&hook(Some(&fake), tmp.path(), T1, "pre-tool-use", &c));
    assert_eq!(fake.requests().len(), first, "served from the cache");
    hook(Some(&fake), tmp.path(), T2, "pre-tool-use", &c);
    assert!(fake.requests().len() > first, "older than 60 s: refreshed");
    assert!(tmp.path().join(".git/bosun/leases.json").exists());
}

#[test]
fn github_down_without_a_cache_allows_with_a_warning() {
    let tmp = repo();
    let c = call(tmp.path(), "Edit", json!({"file_path": "src/core/a.rs"}));
    let o = hook(None, tmp.path(), T0, "pre-tool-use", &c);
    allowed(&o);
    assert!(o.stderr.contains("warning"), "{}", o.stderr);
}

#[test]
fn stale_cache_is_used_while_github_is_down() {
    let (fake, tmp) = (FakeGithub::start(), repo());
    leased(&fake);
    let edit = json!({"file_path": "src/core/a.rs"});
    denied(&pre(&fake, tmp.path(), "Edit", edit.clone()));
    let c = call(tmp.path(), "Edit", edit);
    let o = hook(None, tmp.path(), T2, "pre-tool-use", &c);
    denied(&o);
    assert!(o.stderr.contains("warning"));
}

#[test]
fn malformed_stdin_allows() {
    let tmp = repo();
    let out = bosun()
        .current_dir(tmp.path())
        .args(["hook", "pre-tool-use"])
        .write_stdin("not json")
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(0));
    assert!(out.stdout.is_empty());
    assert!(String::from_utf8_lossy(&out.stderr).contains("warning"));
}

#[test]
fn session_start_joins_and_prints_the_context() {
    let (fake, tmp) = (FakeGithub::start(), repo());
    leased(&fake);
    fake.variable("BOSUN_HALT", "0");
    fake.issue(9, "red", &["bosun:main-red"], "x");
    let input = json!({"hook_event_name":"SessionStart","cwd":tmp.path().to_str().unwrap()});
    let o = hook(Some(&fake), tmp.path(), T0, "session-start", &input);
    assert_eq!(o.code, 0, "{}", o.stderr);
    let v: Value = serde_json::from_str(&o.stdout).unwrap();
    assert_eq!(v["hookSpecificOutput"]["hookEventName"], "SessionStart");
    let text = v["hookSpecificOutput"]["additionalContext"]
        .as_str()
        .unwrap();
    for needle in ["session_me", "local", "#5", "#9", "bosun status"] {
        assert!(text.contains(needle), "{needle} missing: {text}");
    }
    assert!(!text.contains("#4"), "only own leases: {text}");
    let s = fake.state.lock().unwrap();
    let registry = s
        .issues
        .iter()
        .find(|i| i.labels.iter().any(|l| l == "bosun:registry"))
        .expect("registry issue");
    let joined = s
        .comments
        .iter()
        .any(|c| c.issue == registry.number && c.body.contains("session_me"));
    assert!(joined, "join comment posted");
    drop(s);
    assert!(tmp.path().join(".git/bosun/leases.json").exists());
}

#[test]
fn session_start_explains_when_github_is_unreachable() {
    let tmp = repo();
    let input = json!({"cwd": tmp.path().to_str().unwrap()});
    let o = hook(None, tmp.path(), T0, "session-start", &input);
    assert_eq!(o.code, 0);
    let v: Value = serde_json::from_str(&o.stdout).unwrap();
    let text = v["hookSpecificOutput"]["additionalContext"]
        .as_str()
        .unwrap();
    assert!(text.contains("could not reach GitHub"), "{text}");
    assert!(text.contains("GH_TOKEN"), "{text}");
}

#[test]
fn merge_lock_sees_a_merge_nested_in_a_bosun_command() {
    let (fake, tmp) = (FakeGithub::start(), repo());
    let bash = |c: &str| pre(&fake, tmp.path(), "Bash", json!({"command": c}));
    denied(&bash("bosun status $(gh pr merge 1)"));
    denied(&bash("sh -c \"gh pr merge 1\""));
    allowed(&bash("bosun status $(git rev-parse HEAD)"));
}

#[test]
fn edit_on_a_bracket_path_in_another_sessions_lease_is_denied() {
    let (fake, tmp) = (FakeGithub::start(), repo());
    fake.issue(4, "other", &["bosun:state/claimed"], "x");
    fake.comment(4, &lease_note("session_other", LIVE, "app/**"));
    let input = json!({"file_path": "app/[id]/page.tsx"});
    denied(&pre(&fake, tmp.path(), "Edit", input));
}

#[test]
fn commit_in_a_feature_worktree_is_allowed_from_a_main_checkout() {
    let (fake, tmp) = (FakeGithub::start(), repo());
    let main = tmp.path().join("main");
    std::fs::create_dir(&main).unwrap();
    git(&main, &["init", "-q", "-b", "main"]);
    let id = ["-c", "user.name=t", "-c", "user.email=t@t"];
    git(
        &main,
        &[&id[..], &["commit", "-q", "--allow-empty", "-m", "x"]].concat(),
    );
    let wt = tmp.path().join("wt");
    let w = wt.to_str().unwrap();
    git(&main, &["worktree", "add", "-q", "-b", "feature", w]);
    let bash = |c: &str| pre(&fake, &main, "Bash", json!({"command": c}));
    allowed(&bash(&format!("cd {w} && git commit -m x")));
    allowed(&bash(&format!("git -C {w} commit -m x")));
    allowed(&bash(&format!(
        "git --git-dir={w}/.git --work-tree={w} commit -m x"
    )));
    // The main checkout itself, and a directory the hook cannot read, stay refused.
    denied(&bash("git commit -m x"));
    denied(&bash("cd $WT && git commit -m x"));
}

fn rules_start_text(
    fake: Option<&FakeGithub>,
    repo: &Path,
    env: &[(&str, &str)],
) -> (String, String) {
    let mut cmd = bosun();
    cmd.current_dir(repo)
        .env("BOSUN_SESSION", "session_me")
        .env("BOSUN_NOW", T0)
        .args(["hook", "session-start"])
        .write_stdin(json!({"cwd": repo.to_str().unwrap()}).to_string());
    for (k, v) in env {
        cmd.env(k, v);
    }
    if let Some(f) = fake {
        cmd.env("BOSUN_GITHUB_API", f.url());
    }
    let out = cmd.output().unwrap();
    assert_eq!(out.status.code(), Some(0));
    let v: Value = serde_json::from_slice(&out.stdout).unwrap();
    let text = v["hookSpecificOutput"]["additionalContext"].as_str();
    (
        text.unwrap().to_string(),
        String::from_utf8_lossy(&out.stderr).into(),
    )
}

#[test]
fn session_start_rules_follow_the_origin() {
    let (fake, tmp) = (FakeGithub::start(), repo());
    let (local, _) = rules_start_text(Some(&fake), tmp.path(), &[]);
    assert!(
        local.contains("Session rules (orchestrator): Split work"),
        "{local}"
    );
    assert!(local.contains("Caveman style everywhere"), "{local}");
    let (spawned, _) = rules_start_text(
        Some(&fake),
        tmp.path(),
        &[("CLAUDE_CODE_CHILD_SESSION", "1")],
    );
    assert!(
        spawned.contains("Session rules (worker): Use superpowers"),
        "{spawned}"
    );
    let (bad, _) = rules_start_text(Some(&fake), tmp.path(), &[("BOSUN_ORIGIN", "bogus")]);
    assert!(bad.contains("Session rules (orchestrator)"), "{bad}");
    let (bad_child, _) = rules_start_text(
        Some(&fake),
        tmp.path(),
        &[
            ("BOSUN_ORIGIN", "bogus"),
            ("CLAUDE_CODE_CHILD_SESSION", "1"),
        ],
    );
    assert!(bad_child.contains("Session rules (worker)"), "{bad_child}");
}

#[test]
fn session_start_rules_come_from_the_file_and_survive_no_github() {
    let tmp = repo();
    std::fs::create_dir_all(tmp.path().join(".bosun")).unwrap();
    std::fs::write(
        tmp.path().join(".bosun/session-rules.toml"),
        "worker = \"custom worker rule\"\n",
    )
    .unwrap();
    let env = [("BOSUN_ORIGIN", "spawned")];
    let (text, _) = rules_start_text(None, tmp.path(), &env);
    assert!(text.contains("could not reach GitHub"), "{text}");
    assert!(
        text.contains("Session rules (worker): custom worker rule Caveman"),
        "{text}"
    );
    std::fs::write(tmp.path().join(".bosun/session-rules.toml"), "bogus = 1\n").unwrap();
    let (text, err) = rules_start_text(None, tmp.path(), &env);
    assert!(text.contains("Use superpowers"), "{text}");
    assert!(text.contains(".bosun/session-rules.toml ignored"), "{text}");
    assert!(err.contains("session-rules.toml"), "{err}");
}

const LESSONS: &str = "[[lesson]]\nid = \"L1\"\nrule = \"keep it small\"\nevidence = [\"e\"]\n\n\
    [[lesson]]\nid = \"L2\"\nrule = \"scoped rule\"\nscope = [\"src/**\"]\nevidence = [\"e\"]\n";

fn start_text(fake: Option<&FakeGithub>, tmp: &Path) -> String {
    let input = json!({"cwd": tmp.to_str().unwrap()});
    let o = hook(fake, tmp, T0, "session-start", &input);
    let v: Value = serde_json::from_str(&o.stdout).unwrap();
    v["hookSpecificOutput"]["additionalContext"]
        .as_str()
        .unwrap()
        .to_string()
}

#[test]
fn session_start_prints_the_global_lessons_in_both_texts() {
    let (fake, tmp) = (FakeGithub::start(), repo());
    std::fs::create_dir(tmp.path().join("docs")).unwrap();
    std::fs::write(tmp.path().join("docs/lessons.toml"), LESSONS).unwrap();
    for fake in [None, Some(&fake)] {
        let text = start_text(fake, tmp.path());
        assert!(text.contains("\nLessons:\nL1 * keep it small"), "{text}");
        assert!(!text.contains("scoped rule"), "{text}");
    }
}

#[test]
fn session_start_without_usable_lessons_prints_no_block() {
    let tmp = repo();
    assert!(!start_text(None, tmp.path()).contains("Lessons:"));
    std::fs::create_dir(tmp.path().join("docs")).unwrap();
    std::fs::write(tmp.path().join("docs/lessons.toml"), "[[lesson\n").unwrap();
    assert!(!start_text(None, tmp.path()).contains("Lessons:"));
}

#[test]
fn session_start_prints_at_most_eight_lessons() {
    let tmp = repo();
    let many: String = (1..=10)
        .map(|i| format!("[[lesson]]\nid = \"L{i}\"\nrule = \"rule {i}\"\nevidence = [\"e\"]\n\n"))
        .collect();
    std::fs::create_dir(tmp.path().join("docs")).unwrap();
    std::fs::write(tmp.path().join("docs/lessons.toml"), many).unwrap();
    let text = start_text(None, tmp.path());
    let after = text.split("Lessons:\n").nth(1).expect("a block");
    // The session rules follow the lessons block on their own line.
    let lessons = after
        .lines()
        .take_while(|l| !l.starts_with("Session rules"));
    assert_eq!(lessons.count(), 8, "{text}");
}

#[test]
fn session_start_from_a_subdirectory_reads_the_repo_root_lessons() {
    let tmp = repo();
    std::fs::create_dir(tmp.path().join("docs")).unwrap();
    std::fs::write(tmp.path().join("docs/lessons.toml"), LESSONS).unwrap();
    let sub = tmp.path().join("src/deep");
    std::fs::create_dir_all(&sub).unwrap();
    let text = start_text(None, &sub);
    assert!(text.contains("\nLessons:\nL1 * keep it small"), "{text}");
}

fn bash_in(repo: &Path, session: &str, cmd: &str) -> Out {
    let mut input = call(repo, "Bash", json!({ "command": cmd }));
    input["session_id"] = json!(session);
    hook(None, repo, T0, "pre-tool-use", &input)
}

#[test]
fn repeat_lock_warns_at_three_and_denies_at_five() {
    let tmp = repo();
    let r = tmp.path();
    for _ in 0..2 {
        allowed(&bash_in(r, "s1", "cargo test"));
    }
    let third = bash_in(r, "s1", " cargo test ");
    assert_eq!(third.code, 0, "{}", third.stderr);
    let v: Value = serde_json::from_str(&third.stdout).unwrap();
    let h = &v["hookSpecificOutput"];
    assert_eq!(h["hookEventName"], "PreToolUse");
    assert!(h.get("permissionDecision").is_none());
    let note = h["additionalContext"].as_str().unwrap();
    assert!(note.starts_with("repeat-lock"), "{note}");
    allowed(&bash_in(r, "s1", "cargo test"));
    let why = denied(&bash_in(r, "s1", "cargo test"));
    assert!(why.starts_with("repeat-lock"), "{why}");
}

#[test]
fn repeat_lock_resets_on_another_command_or_session() {
    let tmp = repo();
    let r = tmp.path();
    for _ in 0..2 {
        allowed(&bash_in(r, "s1", "cargo test"));
    }
    allowed(&bash_in(r, "s1", "cargo build"));
    allowed(&bash_in(r, "s1", "cargo test"));
    allowed(&bash_in(r, "s1", "cargo test"));
    allowed(&bash_in(r, "s2", "cargo test"));
    allowed(&bash_in(r, "s2", "cargo test"));
}

#[test]
fn destructive_lock_denies_through_the_hook() {
    let tmp = repo();
    let why = denied(&bash_in(tmp.path(), "s1", "sudo rm -rf /"));
    assert!(why.starts_with("destructive-lock"), "{why}");
}

#[test]
fn repeat_lock_counts_each_session_on_its_own() {
    let tmp = repo();
    let r = tmp.path();
    for _ in 0..2 {
        allowed(&bash_in(r, "s1", "cargo test"));
        allowed(&bash_in(r, "s2", "cargo build"));
    }
    let v: Value = serde_json::from_str(&bash_in(r, "s1", "cargo test").stdout).unwrap();
    assert!(
        v["hookSpecificOutput"]["additionalContext"].is_string(),
        "{v}"
    );
}

#[test]
fn repeat_lock_restarts_after_an_edit() {
    let tmp = repo();
    let r = tmp.path();
    for _ in 0..2 {
        allowed(&bash_in(r, "s1", "cargo test"));
    }
    let mut edit = call(
        r,
        "Edit",
        json!({"file_path": r.join("a.rs").to_str().unwrap()}),
    );
    edit["session_id"] = json!("s1");
    hook(None, r, T0, "pre-tool-use", &edit);
    for _ in 0..2 {
        allowed(&bash_in(r, "s1", "cargo test"));
    }
}

#[test]
fn repeat_lock_needs_a_session_id() {
    let tmp = repo();
    let r = tmp.path();
    let input = json!({"tool_name": "Bash", "tool_input": {"command": "ls"}});
    for _ in 0..5 {
        let mut cmd = bosun();
        cmd.current_dir(r)
            .env_remove("BOSUN_SESSION")
            .env("BOSUN_NOW", T0)
            .args(["hook", "pre-tool-use"])
            .write_stdin(input.to_string());
        let out = cmd.output().unwrap();
        assert_eq!(String::from_utf8_lossy(&out.stdout), "", "{out:?}");
    }
}

#[test]
fn repeat_lock_fails_open_when_it_cannot_write() {
    let tmp = repo();
    let r = tmp.path();
    std::fs::create_dir_all(r.join(".git/bosun")).unwrap();
    std::fs::write(r.join(".git/bosun/repeat"), "not a dir").unwrap();
    for _ in 0..5 {
        let out = bash_in(r, "s1", "cargo test");
        allowed(&out);
        assert!(out.stderr.contains("repeat-lock skipped"), "{}", out.stderr);
    }
}
