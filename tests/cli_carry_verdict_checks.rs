//! risk-check's verdict carry-over in `[checks] mode = "checks"`: `settle()` reads the gate history
//! from the App's check runs and re-posts a raced verdict as a check run, never as a status
//! (spec §15; design 2026-10-27 §3 C7).

mod common;

use bosun::core::records::Record;
use common::bosun;
use common::fake::FakeGithub;
use common::one_object;
use serde_json::{Value, json};

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
const SLUG: &str = "keelstackhq-bosun-gate";
const APP_ID: u64 = 4242;
const APP_BOT: &str = "keelstackhq-bosun-gate[bot]";
const CARRIED: &str = "carried from a000000: clean merge of the default branch";

fn cmp(fake: &FakeGithub, a: &str, b: &str, status: &str, base: Option<&str>, file: &str) {
    let key = format!("{a}...{b}");
    let f = json!({"filename": file, "status": "modified", "additions": 1, "deletions": 0});
    let mut s = fake.state.lock().unwrap();
    s.compares
        .insert(key.clone(), if file.is_empty() { vec![] } else { vec![f] });
    s.compare_statuses.insert(key.clone(), status.into());
    if let Some(x) = base {
        s.compare_bases.insert(key, x.into());
    }
}

/// PR 1 (task #7, leased by session_a) whose head H2 merges main's M into H; H has an approving
/// verdict record by the App from another session; H2 has no verdict.
fn world() -> FakeGithub {
    let fake = FakeGithub::start();
    fake.pull(1, "change", "feat", "Closes #7");
    fake.pull_as_issue(1, "change");
    fake.pull_set(1, "head", json!({"sha": H2, "ref": "feat"}));
    fake.pull_set(1, "base", json!({"sha": D, "ref": "main"}));
    fake.issue(7, "the task", &[], "Do it.\n<!-- bosun:task {} -->");
    fake.comment(7, LEASE);
    for (sha, t, parents) in [
        (H2, TH2, vec![H, M]),
        (H, TH, vec![B]),
        (M, TM, vec![B]),
        (B, TB, vec![]),
    ] {
        fake.git_commit(sha, t, &parents);
    }
    let tree = |m, p, d| {
        vec![
            ("src", "040000", "tree", d),
            ("src/main.rs", "100644", "blob", m),
            ("src/pr.rs", "100644", "blob", p),
        ]
    };
    for (t, e) in [
        (TB, tree("m1", "p0", "d0")),
        (TM, tree("m2", "p0", "d1")),
        (TH, tree("m1", "p1", "d2")),
        (TH2, tree("m2", "p1", "d3")),
    ] {
        fake.git_tree(t, &e, false);
    }
    for (a, b, st, base, file) in [
        (D, "refs/heads/main", "identical", None, ""),
        (M, D, "ahead", None, ""),
        (D, H, "diverged", Some(B), ""),
        (B, M, "ahead", None, "src/main.rs"),
        (B, H, "ahead", None, "src/pr.rs"),
    ] {
        cmp(&fake, a, b, st, base, file);
    }
    fake.check_runs_as_app(SLUG, APP_ID);
    fake.app_run(H, "review-verdict", "success", "approved", SLUG, APP_ID);
    let r = Record::Verdict {
        session: "session_r".into(),
        head_sha: H.into(),
        approved: true,
        confidence: Some(0.95),
        major: Some(0),
        model: None,
    };
    fake.comment_by(
        1,
        APP_BOT,
        &format!("Review verdict: x.\n\n{}", r.to_line()),
    );
    fake
}

fn go(fake: &FakeGithub) -> Value {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join(".bosun")).unwrap();
    let toml = format!("[checks]\nmode = \"checks\"\napp_id = {APP_ID}\napp_slug = \"{SLUG}\"\n");
    std::fs::write(dir.path().join(".bosun/bosun.toml"), toml).unwrap();
    let out = bosun()
        .env("BOSUN_GITHUB_API", fake.url())
        .env("BOSUN_APP_TOKEN", "app-token-canary")
        .env("BOSUN_SESSION", "session_test")
        .env("BOSUN_NOW", "2026-10-22T10:00:00Z")
        .env("BOSUN_HTTP_MAX_SLEEP_MS", "0")
        .arg("-C")
        .arg(dir.path())
        .args(["risk-check", "1"])
        .output()
        .unwrap();
    let v = one_object(&out.stdout);
    assert_eq!(out.status.code(), Some(0), "{v}");
    v
}

fn verdict_runs(fake: &FakeGithub) -> Vec<Value> {
    let all = fake.check_runs_of(H2);
    all.into_iter()
        .filter(|r| r["name"] == "review-verdict")
        .collect()
}

#[test]
fn a_settled_carry_is_a_check_run_and_no_status() {
    let fake = world();
    let v = go(&fake);
    assert_eq!(v["data"]["verdict_carried_from"], H, "{v}");
    let runs = verdict_runs(&fake);
    assert_eq!(runs.len(), 1, "{runs:?}");
    assert_eq!(runs[0]["conclusion"], "success");
    assert_eq!(runs[0]["output"]["title"], CARRIED);
    assert!(
        fake.statuses_of(H2)
            .iter()
            .all(|s| s["context"] != "review-verdict")
    );
}

#[test]
fn a_raced_verdict_is_posted_again_as_a_check_run_and_the_carry_is_dropped() {
    let fake = world();
    // A real verdict run, then our carry on top of it (a run cancelled mid-post): settle re-posts
    // the real one so it stands newest, and the carry is not kept.
    fake.app_run(H2, "review-verdict", "failure", "needs work", SLUG, APP_ID);
    fake.app_run(H2, "review-verdict", "success", CARRIED, SLUG, APP_ID);
    let v = go(&fake);
    assert_eq!(v["data"]["verdict_carried_from"], Value::Null, "{v}");
    let runs = verdict_runs(&fake);
    assert_eq!(runs.len(), 3, "{runs:?}");
    let last = runs.last().unwrap();
    assert_eq!(last["conclusion"], "failure");
    assert_eq!(last["output"]["title"], "needs work");
    assert!(
        fake.statuses_of(H2)
            .iter()
            .all(|s| s["context"] != "review-verdict")
    );
}
