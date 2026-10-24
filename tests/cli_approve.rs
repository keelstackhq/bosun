mod common;

use common::fake::FakeGithub;
use common::{bosun, one_object};
use serde_json::{Value, json};

const OWNER_TOKEN: &str = "canary-owner-token-7f3a";
const WAITING: &str = "high risk, owner approval needed: R-gov, R-ci";

/// Run `bosun approve <args>` with the owner token; asserts one envelope, a fix on every error
/// and that neither stdout nor stderr carries a token.
fn approve(fake: &FakeGithub, args: &[&str]) -> (i32, Value) {
    let out = bosun()
        .env("BOSUN_GITHUB_API", fake.url())
        .env("BOSUN_OWNER_TOKEN", OWNER_TOKEN)
        .arg("approve")
        .args(args)
        .output()
        .unwrap();
    let all = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(!all.contains(OWNER_TOKEN), "owner token leaked: {all}");
    assert!(!all.contains(common::TOKEN_CANARY), "token leaked: {all}");
    let v = one_object(&out.stdout);
    if v["ok"] == false {
        assert!(!v["error"]["fix"].as_str().unwrap().trim().is_empty());
    }
    (out.status.code().unwrap(), v)
}

/// A PR whose risk-check waits for an owner, with review-verdict and ci-ok green.
fn ready(fake: &FakeGithub, n: u64, title: &str) {
    waiting(fake, n, title);
    let sha = format!("sha{n}");
    fake.status(&sha, "review-verdict", "success", "approved");
    fake.check_run(&sha, "ci-ok", "completed", Some("success"));
}

fn waiting(fake: &FakeGithub, n: u64, title: &str) {
    fake.pull(n, title, &format!("b{n}"), "");
    fake.pull_as_issue(n, title);
    fake.status(&format!("sha{n}"), "risk-check", "failure", WAITING);
    fake.pull_file(n, &format!("governance/f{n}.toml"), "modified", 1, 0, None);
}

fn comment_posts(fake: &FakeGithub) -> Vec<(String, Option<String>, Option<String>)> {
    fake.requests()
        .into_iter()
        .filter(|r| r.method == "POST" && r.path.ends_with("/comments"))
        .map(|r| (r.path, r.body, r.headers[0].clone()))
        .collect()
}

fn records(fake: &FakeGithub) -> Vec<Value> {
    let s = fake.state.lock().unwrap();
    s.comments
        .iter()
        .filter(|c| c.body.contains("\"bosun\":\"approve\""))
        .map(|c| serde_json::from_str(&c.body).unwrap())
        .collect()
}

fn board() -> FakeGithub {
    let fake = FakeGithub::start();
    ready(&fake, 3, "gov change");
    ready(&fake, 1, "ci change");
    waiting(&fake, 2, "no verdict");
    waiting(&fake, 4, "red ci");
    fake.status("sha4", "review-verdict", "success", "approved");
    fake.check_run("sha4", "ci-ok", "completed", Some("failure"));
    // Normal risk, and a closed PR: neither is listed.
    fake.pull(5, "small", "b5", "");
    fake.status("sha5", "risk-check", "success", "normal risk");
    waiting(&fake, 6, "closed");
    fake.pull_set(6, "state", json!("closed"));
    fake
}

#[test]
fn list_shows_ready_and_waiting_sorted() {
    let fake = board();
    let (code, v) = approve(&fake, &["--list"]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["command"], "approve");
    let ready = v["data"]["ready"].as_array().unwrap();
    assert_eq!(ready.len(), 2);
    assert_eq!(ready[0]["pr"], 1);
    assert_eq!(ready[0]["title"], "ci change");
    assert_eq!(ready[0]["head_sha"], "sha1");
    assert_eq!(ready[0]["risk"], json!(["R-gov", "R-ci"]));
    assert_eq!(ready[0]["files"], json!(["governance/f1.toml"]));
    assert_eq!(ready[1]["pr"], 3);
    assert_eq!(
        v["data"]["waiting"],
        json!([
            {"pr": 2, "why": "review-verdict not green"},
            {"pr": 4, "why": "ci-ok not green"}
        ])
    );
    assert!(comment_posts(&fake).is_empty(), "--list writes nothing");
}

#[test]
fn ci_ok_commit_status_counts_too() {
    let fake = FakeGithub::start();
    waiting(&fake, 1, "x");
    fake.status("sha1", "review-verdict", "success", "approved");
    fake.status("sha1", "ci-ok", "success", "");
    let (code, v) = approve(&fake, &["--list"]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["ready"][0]["pr"], 1);
}

#[test]
fn approve_posts_the_comment_with_the_owner_token_and_records_it() {
    let fake = board();
    let (code, v) = approve(&fake, &["1"]);
    assert_eq!(code, 0, "{v}");
    let posts = comment_posts(&fake);
    let on_pr: Vec<_> = posts
        .iter()
        .filter(|p| p.0.ends_with("/issues/1/comments"))
        .collect();
    assert_eq!(on_pr.len(), 1, "{posts:?}");
    let body: Value = serde_json::from_str(on_pr[0].1.as_ref().unwrap()).unwrap();
    assert_eq!(body["body"], "/bosun approve sha1");
    assert_eq!(
        on_pr[0].2.as_deref(),
        Some("Bearer canary-owner-token-7f3a")
    );
    let cid = v["data"]["approved"][0]["comment_id"].as_u64().unwrap();
    assert!(cid > 0);
    assert_eq!(v["data"]["approved"][0]["pr"], 1);
    assert_eq!(v["data"]["approved"][0]["head_sha"], "sha1");
    // Every request used the owner token, never GITHUB_TOKEN.
    for r in fake.requests() {
        assert_eq!(
            r.headers[0].as_deref(),
            Some("Bearer canary-owner-token-7f3a"),
            "{} {}",
            r.method,
            r.path
        );
    }
    let recs = records(&fake);
    assert_eq!(recs.len(), 1);
    assert_eq!(
        recs[0],
        json!({"bosun": "approve", "prs": [{"pr": 1, "head_sha": "sha1"}],
            "at": "2026-10-22T10:00:00Z"})
    );
}

#[test]
fn one_pr_not_ready_posts_nothing_and_is_a_conflict() {
    let fake = board();
    fake.pull(7, "plain", "b7", "");
    fake.status("sha7", "risk-check", "success", "normal risk");
    let (code, v) = approve(&fake, &["1", "2", "4", "5", "6", "7", "99"]);
    assert_eq!(code, 3, "{v}");
    assert!(comment_posts(&fake).is_empty());
    assert!(records(&fake).is_empty());
    assert_eq!(
        v["data"]["not_ready"],
        json!([
            {"pr": 2, "why": "review-verdict not green"},
            {"pr": 4, "why": "ci-ok not green"},
            {"pr": 5, "why": "not waiting for an owner"},
            {"pr": 6, "why": "closed"},
            {"pr": 7, "why": "not waiting for an owner"},
            {"pr": 99, "why": "not found"}
        ])
    );
}

#[test]
fn all_ready_approves_the_whole_list_in_one_record() {
    let fake = board();
    let (code, v) = approve(&fake, &["--all-ready"]);
    assert_eq!(code, 0, "{v}");
    let prs: Vec<u64> = v["data"]["approved"]
        .as_array()
        .unwrap()
        .iter()
        .map(|a| a["pr"].as_u64().unwrap())
        .collect();
    assert_eq!(prs, [1, 3]);
    let recs = records(&fake);
    assert_eq!(recs.len(), 1);
    assert_eq!(recs[0]["prs"].as_array().unwrap().len(), 2);
}

#[test]
fn all_ready_with_nothing_ready_is_ok_and_writes_no_record() {
    let fake = FakeGithub::start();
    waiting(&fake, 2, "no verdict");
    let (code, v) = approve(&fake, &["--all-ready"]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["approved"], json!([]));
    assert!(comment_posts(&fake).is_empty());
    assert!(records(&fake).is_empty());
}

#[test]
fn halted_posts_nothing() {
    let fake = board();
    fake.variable("BOSUN_HALT", "1");
    for args in [&["1"][..], &["--all-ready"], &["--main-open", "1"]] {
        let (code, v) = approve(&fake, args);
        assert_eq!(code, 4, "{args:?} {v}");
    }
    assert!(comment_posts(&fake).is_empty());
    assert!(
        fake.statuses_of("sha1")
            .iter()
            .all(|s| s["context"] != "main-open")
    );
}

#[test]
fn missing_or_blank_owner_token_is_needs_human_and_never_falls_back() {
    let fake = board();
    for token in [None, Some("  ")] {
        let mut cmd = bosun();
        cmd.env("BOSUN_GITHUB_API", fake.url())
            .args(["approve", "--list"]);
        match token {
            Some(t) => cmd.env("BOSUN_OWNER_TOKEN", t),
            None => cmd.env_remove("BOSUN_OWNER_TOKEN"),
        };
        let out = cmd.output().unwrap();
        assert_eq!(out.status.code(), Some(5));
        let v = one_object(&out.stdout);
        assert!(v["error"]["message"].as_str().unwrap().contains("owner"));
        let fix = v["error"]["fix"].as_str().unwrap();
        assert!(fix.contains("BOSUN_OWNER_TOKEN") && fix.contains("Claude Code"));
    }
    assert!(fake.requests().is_empty(), "no request without the token");
}

#[test]
fn usage_errors_exit_2() {
    let fake = board();
    for args in [
        &[][..],
        &["--list", "1"],
        &["--all-ready", "1"],
        &["--list", "--all-ready"],
        &["--main-open", "1", "--list"],
        &["--main-open", "1", "2"],
        &["--main-open", "1", "--all-ready"],
    ] {
        let (code, v) = approve(&fake, args);
        assert_eq!(code, 2, "{args:?} {v}");
    }
    assert!(fake.requests().is_empty());
}

#[test]
fn main_open_posts_the_status_and_records_it() {
    let fake = board();
    let (code, v) = approve(&fake, &["--main-open", "1"]);
    assert_eq!(code, 0, "{v}");
    let s = fake
        .statuses_of("sha1")
        .into_iter()
        .find(|s| s["context"] == "main-open")
        .expect("a main-open status");
    assert_eq!(s["state"], "success");
    assert!(
        s["description"]
            .as_str()
            .unwrap()
            .contains("bosun approve --main-open")
    );
    let recs = records(&fake);
    assert_eq!(recs.len(), 1);
    assert_eq!(recs[0]["prs"], json!([]));
    assert_eq!(recs[0]["main_open"], json!([{"pr": 1, "head_sha": "sha1"}]));
    for r in fake.requests() {
        assert_eq!(
            r.headers[0].as_deref(),
            Some("Bearer canary-owner-token-7f3a")
        );
    }
}

#[test]
fn main_open_follows_the_gate_when_main_is_red() {
    let fake = board();
    fake.issue(50, "main is red", &["bosun:main-red"], "");
    let (code, _) = approve(&fake, &["--main-open", "1"]);
    assert_eq!(code, 4);
    let s = fake
        .statuses_of("sha1")
        .into_iter()
        .find(|s| s["context"] == "main-open")
        .unwrap();
    assert_eq!(s["state"], "failure");
    assert!(records(&fake).is_empty());
    // A mainfix PR passes.
    fake.pull_set(3, "labels", json!([{"name": "bosun:kind/mainfix"}]));
    let (code, v) = approve(&fake, &["--main-open", "3"]);
    assert_eq!(code, 0, "{v}");
}

#[test]
fn main_open_needs_an_open_pr() {
    let fake = board();
    for pr in ["6", "99"] {
        let (code, _) = approve(&fake, &["--main-open", pr]);
        assert_eq!(code, 3, "PR {pr}");
    }
    assert!(
        fake.statuses_of("sha6")
            .iter()
            .all(|s| s["context"] != "main-open")
    );
}

#[test]
fn the_owner_token_never_reaches_stdout_or_stderr() {
    // `approve()` asserts this on every run; this walks every mode, including a GitHub error.
    let fake = board();
    for args in [
        &["--list"][..],
        &["1"],
        &["2"],
        &["--all-ready"],
        &["--main-open", "3"],
        &["--main-open", "99"],
        &["99"],
    ] {
        approve(&fake, args);
    }
    fake.fail_path("/pulls", 403);
    let (code, v) = approve(&fake, &["--list"]);
    assert_eq!(code, 1, "{v}");
}
