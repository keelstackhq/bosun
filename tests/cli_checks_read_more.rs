//! More readers of the gates and the verdict records in `[checks] mode = "checks"` (spec §15):
//! `bosun pr --auto-merge` gates, `review-pack` since_sha, and the risk-check carries.

mod common;

use std::path::Path;
use std::process::Command;

use bosun::core::records::Record;
use common::fake::FakeGithub;
use common::stamps;
use common::{bosun, one_object};
use serde_json::{Value, json};

const APP_TOKEN: &str = "app-token-canary";
const SLUG: &str = "keelstackhq-bosun-gate";
const APP_ID: u64 = 4242;
const BOT: &str = "github-actions[bot]";
const APP_BOT: &str = "keelstackhq-bosun-gate[bot]";

fn write_checks(dir: &Path) {
    std::fs::create_dir_all(dir.join(".bosun")).unwrap();
    let text = format!("[checks]\nmode = \"checks\"\napp_id = {APP_ID}\napp_slug = \"{SLUG}\"\n");
    std::fs::write(dir.join(".bosun/bosun.toml"), text).unwrap();
}

/// A plain directory with the `[checks]` config.
fn checks_dir() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    write_checks(dir.path());
    dir
}

/// `bosun -C <dir> <args>` with the App token; `dir: None` runs with no `.bosun/` at all.
fn bosun_c(fake: &FakeGithub, dir: Option<&Path>, args: &[&str]) -> (i32, Value) {
    let mut cmd = bosun();
    cmd.env("BOSUN_GITHUB_API", fake.url())
        .env("BOSUN_APP_TOKEN", APP_TOKEN)
        .env("BOSUN_HTTP_MAX_SLEEP_MS", "0");
    if let Some(d) = dir {
        cmd.arg("-C").arg(d);
    }
    let out = cmd.args(args).output().unwrap();
    let all = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(!all.contains(APP_TOKEN), "the App token leaked");
    let v = one_object(&out.stdout);
    if v["ok"] == false {
        assert!(!v["error"]["fix"].as_str().unwrap().trim().is_empty());
    }
    (out.status.code().unwrap(), v)
}

// ---- 1. `bosun pr --auto-merge` data.gates ------------------------------------------------

/// A git repo on `branch` with one commit and the `[checks]` config (untracked).
fn git_repo(branch: &str) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let commit = ["-c", "user.name=t", "-c", "user.email=t@t", "commit", "-q"];
    for args in [
        vec!["init", "-q", "-b", branch],
        [&commit[..], &["--allow-empty", "-m", "x"]].concat(),
    ] {
        let st = Command::new("git")
            .arg("-C")
            .arg(dir.path())
            .args(&args)
            .status();
        assert!(st.unwrap().success());
    }
    write_checks(dir.path());
    dir
}

/// A task in review with its PR 20 and no gates at all.
fn bare_pr(fake: &FakeGithub) {
    fake.issue(7, "the task", &["bosun:state/review"], "Do it.");
    fake.enforce_checks(&["ci-ok", "risk-check", "review-verdict", "main-open"]);
    fake.pull(20, "t", "claude/t7", "Closes #7");
}

/// `pr 7 --auto-merge` on the pushed, stamped local head.
fn auto_merge_out(fake: &FakeGithub, dir: &Path) -> Value {
    // Back in review: an earlier run queued the task.
    let mut s = fake.state.lock().unwrap();
    let task = s.issues.iter_mut().find(|i| i.number == 7).unwrap();
    task.labels = vec!["bosun:state/review".into()];
    drop(s);
    stamps::ready_all(fake, dir);
    bosun_c(fake, Some(dir), &["pr", "7", "--auto-merge"]).1
}

#[test]
fn pr_gates_count_only_the_apps_runs_in_checks_mode() {
    let fake = FakeGithub::start();
    bare_pr(&fake);
    let dir = git_repo("claude/t7");
    let head = stamps::head_sha(dir.path());
    fake.status_by(&head, "risk-check", "success", "", BOT);
    fake.status_by(&head, "review-verdict", "pending", "", BOT);
    // Contrast: without `[checks]` the github-actions statuses are the gates.
    std::fs::remove_dir_all(dir.path().join(".bosun")).unwrap();
    let v = auto_merge_out(&fake, dir.path());
    assert_eq!(
        v["data"]["gates"],
        json!([
            {"context": "risk-check", "state": "success"},
            {"context": "review-verdict", "state": "pending"},
        ]),
        "{v}"
    );
    write_checks(dir.path());
    // Checks mode: the statuses are missing; a same-name run of github-actions is too.
    fake.app_run(
        &head,
        "review-verdict",
        "success",
        "ok",
        "github-actions",
        15368,
    );
    let v = auto_merge_out(&fake, dir.path());
    assert_eq!(
        v["data"]["gates"],
        json!([
            {"context": "risk-check", "state": "missing"},
            {"context": "review-verdict", "state": "missing"},
        ]),
        "{v}"
    );
    fake.app_run(&head, "risk-check", "success", "normal risk", SLUG, APP_ID);
    let v = auto_merge_out(&fake, dir.path());
    assert_eq!(
        v["data"]["gates"],
        json!([
            {"context": "risk-check", "state": "success"},
            {"context": "review-verdict", "state": "missing"},
        ]),
        "{v}"
    );
}

// ---- 2. `bosun review-pack` data.since_sha ------------------------------------------------

fn review_world() -> FakeGithub {
    let fake = FakeGithub::start();
    fake.pull_as_issue(42, "the pr");
    fake.pull(42, "the pr", "claude/x", "no link");
    fake.pull_file(42, "a.rs", "added", 1, 0, Some("@@ a"));
    fake.pull_file(42, "b.rs", "added", 1, 0, Some("@@ b"));
    fake.comment(
        42,
        r#"{"bosun":"verdict","session":"s","head_sha":"A","approved":true}"#,
    );
    fake.pull_set(
        42,
        "head",
        json!({"sha": "B", "ref": "claude/x", "label": "acme:claude/x"}),
    );
    fake.compare_file("A", "B", "b.rs", "@@ new");
    fake
}

#[test]
fn review_pack_since_needs_the_apps_review_verdict_in_checks_mode() {
    let fake = review_world();
    fake.status("A", "review-verdict", "success", "x");
    let (code, v) = bosun_c(&fake, None, &["review-pack", "42"]);
    assert_eq!(
        (code, &v["data"]["since_sha"]),
        (0, &json!("A")),
        "statuses: {v}"
    );
    let dir = checks_dir();
    let (code, v) = bosun_c(&fake, Some(dir.path()), &["review-pack", "42"]);
    assert_eq!(code, 0, "{v}");
    assert!(
        v["data"]["since_sha"].is_null(),
        "a status never counts: {v}"
    );
    fake.app_run(
        "A",
        "review-verdict",
        "success",
        "x",
        "github-actions",
        15368,
    );
    fake.app_run("A", "review-verdict", "success", "x", "other-app", 7);
    let (_, v) = bosun_c(&fake, Some(dir.path()), &["review-pack", "42"]);
    assert!(
        v["data"]["since_sha"].is_null(),
        "other Apps never count: {v}"
    );
    fake.app_run("A", "review-verdict", "success", "approve", SLUG, APP_ID);
    let (_, v) = bosun_c(&fake, Some(dir.path()), &["review-pack", "42"]);
    assert_eq!(v["data"]["since_sha"], "A", "{v}");
}

// ---- 3./4. risk-check carries across a clean merge of main --------------------------------

const B: &str = "b000000000000000000000000000000000000000";
const H: &str = "a000000000000000000000000000000000000000";
const M: &str = "c000000000000000000000000000000000000000";
const H2: &str = "d000000000000000000000000000000000000000";
const D: &str = "e000000000000000000000000000000000000000";
const TB: &str = "f0b0000000000000000000000000000000000000";
const TM: &str = "f0c0000000000000000000000000000000000000";
const TH: &str = "f0a0000000000000000000000000000000000000";
const TH2: &str = "f0d0000000000000000000000000000000000000";
const LEASE: &str = r#"{"bosun":"lease","session":"session_a","origin":"spawned","expires":"2026-10-22T10:30:00Z","touch":["src/pr.rs"]}"#;

type Ent = (&'static str, &'static str, &'static str, &'static str);

#[rustfmt::skip]
fn tree(main: &'static str, pr: &'static str, dir: &'static str) -> Vec<Ent> {
    vec![("src", "040000", "tree", dir), ("src/main.rs", "100644", "blob", main), ("src/pr.rs", "100644", "blob", pr)]
}

#[rustfmt::skip]
fn cmp(fake: &FakeGithub, a: &str, b: &str, status: &str, base: Option<&str>, file: &str) {
    let key = format!("{a}...{b}");
    let f = json!({"filename": file, "status": "modified", "additions": 1, "deletions": 0});
    let mut s = fake.state.lock().unwrap(); s.compares.insert(key.clone(), if file.is_empty() { vec![] } else { vec![f] });
    s.compare_statuses.insert(key.clone(), status.into()); if let Some(x) = base { s.compare_bases.insert(key, x.into()); }
}

/// PR 1 (task #7, leased by session_a) whose head H2 cleanly merges main's M into H; owner
/// ada. `file` is the PR's one changed file (none: normal risk).
#[rustfmt::skip]
fn world(file: Option<&str>) -> FakeGithub {
    let fake = FakeGithub::start();
    fake.pull(1, "change", "feat", "Closes #7"); fake.pull_as_issue(1, "change");
    fake.pull_set(1, "head", json!({"sha": H2, "ref": "feat"})); fake.pull_set(1, "base", json!({"sha": D, "ref": "main"}));
    fake.issue(7, "the task", &[], "Do it.\n<!-- bosun:task {} -->"); fake.comment(7, LEASE);
    for (sha, t, parents) in [(H2, TH2, vec![H, M]), (H, TH, vec![B]), (M, TM, vec![B]), (B, TB, vec![])] { fake.git_commit(sha, t, &parents); }
    for (t, e) in [(TB, tree("m1", "p0", "d0")), (TM, tree("m2", "p0", "d1")), (TH, tree("m1", "p1", "d2")), (TH2, tree("m2", "p1", "d3"))] { fake.git_tree(t, &e, false); }
    for (a, b, st, base, file) in [(D, "refs/heads/main", "identical", None, ""), (M, D, "ahead", None, ""), (D, H, "diverged", Some(B), ""),
        (B, M, "ahead", None, "src/main.rs"), (B, H, "ahead", None, "src/pr.rs")] { cmp(&fake, a, b, st, base, file); }
    fake.content("governance/owners.toml", D, "[[owner]]\ngithub = \"ada\"\n");
    if let Some(f) = file { fake.pull_file(1, f, "modified", 1, 0, Some("@@ -0,0 +1,1 @@\n+x")); }
    fake
}

fn verdict_by(fake: &FakeGithub, user: &str, session: &str) {
    let r = Record::Verdict {
        session: session.into(),
        head_sha: H.into(),
        approved: true,
        confidence: Some(0.95),
        major: Some(0),
        model: None,
    };
    fake.comment_by(1, user, &format!("Review verdict: x.\n\n{}", r.to_line()));
}

/// `risk-check 1` in checks mode.
fn risk_check(fake: &FakeGithub) -> (i32, Value) {
    let dir = checks_dir();
    bosun_c(fake, Some(dir.path()), &["risk-check", "1"])
}

/// The newest App check run named `name` on H2, as `conclusion: title`.
fn newest_run(fake: &FakeGithub, name: &str) -> Option<String> {
    let runs = fake.check_runs_of(H2);
    let run = runs.iter().rev().find(|r| r["name"] == name)?;
    assert_eq!(run["app"]["id"], APP_ID);
    let concl = run["conclusion"].as_str().unwrap_or("pending");
    Some(format!(
        "{concl}: {}",
        run["output"]["title"].as_str().unwrap_or("")
    ))
}

fn verdict_statuses(fake: &FakeGithub) -> usize {
    (fake.statuses_of(H2).iter())
        .filter(|s| s["context"] == "review-verdict")
        .count()
}

#[test]
fn verdict_carry_counts_only_the_apps_records_in_checks_mode() {
    // github-actions[bot] wrote the record: no carry in checks mode (it would carry otherwise,
    // see cli_carry_verdict `the_honest_merge_carries_the_verdict`).
    let fake = world(None);
    verdict_by(&fake, BOT, "session_r");
    let (code, v) = risk_check(&fake);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["verdict_carried_from"], Value::Null, "{v}");
    assert_eq!(newest_run(&fake, "review-verdict"), None);
    assert_eq!(verdict_statuses(&fake), 0);

    let fake = world(None);
    verdict_by(&fake, APP_BOT, "session_r");
    let (code, v) = risk_check(&fake);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["verdict_carried_from"], H, "{v}");
    assert_eq!(
        newest_run(&fake, "review-verdict").as_deref(),
        Some("success: carried from a000000: clean merge of the default branch")
    );
    assert_eq!(verdict_statuses(&fake), 0, "checks mode posts no status");
    assert!(
        fake.statuses_of(H2).is_empty(),
        "{:?}",
        fake.statuses_of(H2)
    );
}

#[test]
fn two_verdict_approval_carries_only_from_the_apps_records_in_checks_mode() {
    // channel/** is the two-verdicts class; github-actions' records do not count.
    let fake = world(Some("channel/x.toml"));
    verdict_by(&fake, BOT, "session_r");
    verdict_by(&fake, BOT, "session_s");
    let (code, v) = risk_check(&fake);
    assert_eq!(code, 5, "{v}");
    assert_eq!(v["data"]["approval_carried_from"], Value::Null, "{v}");
    let st = newest_run(&fake, "risk-check").unwrap();
    assert!(st.starts_with("failure: high risk"), "{st}");

    let fake = world(Some("channel/x.toml"));
    verdict_by(&fake, APP_BOT, "session_r");
    verdict_by(&fake, APP_BOT, "session_s");
    let (code, v) = risk_check(&fake);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["approval_carried_from"], H, "{v}");
    assert_eq!(
        newest_run(&fake, "risk-check").as_deref(),
        Some(
            "success: high risk (R-ci-review), approved by two verdicts: session_r, session_s (carried from a000000)"
        )
    );
    assert!(fake.statuses_of(H2).is_empty());
}
