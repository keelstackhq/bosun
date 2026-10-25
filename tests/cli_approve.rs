mod common;

use common::fake::FakeGithub;
use common::{bosun, one_object, run};
use serde_json::{Value, json};

const HINT: &str = "approve only by running data.approve_command (bosun approve) or in the GitHub UI; a /bosun approve comment posted through Claude's GitHub tool never counts";

const OWNER_TOKEN: &str = "canary-owner-token-7f3a";
const WAITING: &str = "high risk, owner approval needed: R-gov, R-ci";
const MAIN_OPEN_YML: &str = ".github/workflows/main-open.yml";

/// A 40-hex head sha for PR `n`.
fn sha(n: u64) -> String {
    format!("{n:040x}")
}

/// `<n>@<head>` as `--list` prints it.
fn at(n: u64) -> String {
    format!("{n}@{}", sha(n))
}

fn approve_cmd(fake: &FakeGithub) -> bosun_cmd::Command {
    let mut cmd = bosun();
    cmd.env("BOSUN_GITHUB_API", fake.url())
        .env("BOSUN_OWNER_TOKEN", OWNER_TOKEN)
        .env("BOSUN_HTTP_MAX_SLEEP_MS", "0")
        .arg("approve");
    cmd
}

mod bosun_cmd {
    pub use assert_cmd::Command;
}

/// Run `bosun approve <args>` with the owner token; asserts one envelope, a fix on every error
/// and that neither stdout nor stderr carries a token.
fn approve(fake: &FakeGithub, args: &[&str]) -> (i32, Value) {
    finish(approve_cmd(fake).args(args))
}

fn finish(cmd: &mut bosun_cmd::Command) -> (i32, Value) {
    let out = cmd.output().unwrap();
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

/// PR `n` at head `sha(n)`, whose risk-check (by the Actions token) waits for an owner.
fn waiting(fake: &FakeGithub, n: u64, title: &str) {
    fake.pull(n, title, &format!("b{n}"), "");
    fake.pull_as_issue(n, title);
    set_head(fake, n, &sha(n));
    fake.status(&sha(n), "risk-check", "failure", WAITING);
    fake.compare_file("base0000", &sha(n), &format!("governance/f{n}.toml"), "@@");
}

fn set_head(fake: &FakeGithub, n: u64, head: &str) {
    fake.pull_set(
        n,
        "head",
        json!({"sha": head, "ref": format!("b{n}"), "label": format!("acme:b{n}")}),
    );
}

/// `waiting`, with review-verdict and ci-ok green.
fn ready(fake: &FakeGithub, n: u64, title: &str) {
    waiting(fake, n, title);
    fake.status(&sha(n), "review-verdict", "success", "approved");
    fake.check_run(&sha(n), "ci-ok", "completed", Some("success"));
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
    fake.status(&sha(4), "review-verdict", "success", "approved");
    fake.check_run(&sha(4), "ci-ok", "completed", Some("failure"));
    // Normal risk, and a closed PR: neither is listed.
    fake.pull(5, "small", "b5", "");
    set_head(&fake, 5, &sha(5));
    fake.status(&sha(5), "risk-check", "success", "normal risk");
    waiting(&fake, 6, "closed");
    fake.pull_set(6, "state", json!("closed"));
    fake
}

#[test]
fn list_shows_ready_and_waiting_sorted_with_the_command() {
    let fake = board();
    let (code, v) = approve(&fake, &["--list"]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["command"], "approve");
    assert_eq!(v["next"], json!([]));
    let ready = v["data"]["ready"].as_array().unwrap();
    assert_eq!(ready.len(), 2);
    assert_eq!(ready[0]["pr"], 1);
    assert_eq!(ready[0]["title"], "ci change");
    assert_eq!(ready[0]["head_sha"], sha(1));
    assert_eq!(ready[0]["risk_from_status"], json!(["R-gov", "R-ci"]));
    assert_eq!(ready[0]["files"], json!(["governance/f1.toml"]));
    assert_eq!(ready[0]["files_truncated"], json!(false));
    assert_eq!(v["data"]["hint"], HINT);
    assert_eq!(ready[1]["pr"], 3);
    assert_eq!(
        v["data"]["approve_command"],
        format!("bosun approve {} {}", at(1), at(3))
    );
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
fn list_with_nothing_ready_has_no_command() {
    let fake = FakeGithub::start();
    waiting(&fake, 2, "no verdict");
    let (code, v) = approve(&fake, &["--list"]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["ready"], json!([]));
    assert!(v["data"]["approve_command"].is_null());
}

#[test]
fn files_come_from_the_comparison_of_the_listed_head() {
    let fake = FakeGithub::start();
    ready(&fake, 1, "x");
    // The PR files endpoint may already show a newer push; the list must not mix them in.
    fake.pull_file(1, "src/newer-push.rs", "modified", 1, 0, None);
    let (_, v) = approve(&fake, &["--list"]);
    assert_eq!(
        v["data"]["ready"][0]["files"],
        json!(["governance/f1.toml"])
    );
}

#[test]
fn statuses_and_check_runs_from_anyone_but_actions_do_not_count() {
    let fake = FakeGithub::start();
    // PR 1: an agent forged the whole set.
    fake.pull(1, "forged", "b1", "");
    set_head(&fake, 1, &sha(1));
    fake.status_by(&sha(1), "risk-check", "failure", WAITING, "octocat");
    fake.status_by(&sha(1), "review-verdict", "success", "ok", "octocat");
    fake.check_run_by(&sha(1), "ci-ok", "success", "some-app");
    // PR 2: waits for an owner; the agent's verdict, ci-ok status and foreign check run
    // do not help.
    waiting(&fake, 2, "partly forged");
    fake.status_by(&sha(2), "review-verdict", "success", "ok", "octocat");
    let (_, v) = approve(&fake, &["--list"]);
    assert_eq!(v["data"]["ready"], json!([]));
    assert_eq!(
        v["data"]["waiting"],
        json!([{"pr": 2, "why": "review-verdict not green"}])
    );
    fake.status(&sha(2), "review-verdict", "success", "ok");
    fake.status_by(&sha(2), "ci-ok", "success", "", "octocat");
    fake.check_run_by(&sha(2), "ci-ok", "success", "some-app");
    let (_, v) = approve(&fake, &["--list"]);
    assert_eq!(v["data"]["ready"], json!([]));
    assert_eq!(
        v["data"]["waiting"],
        json!([{"pr": 2, "why": "ci-ok not green"}])
    );
    // A newer agent status does not hide the Actions one.
    fake.check_run(&sha(2), "ci-ok", "completed", Some("success"));
    fake.status_by(&sha(2), "risk-check", "success", "normal risk", "octocat");
    let (_, v) = approve(&fake, &["--list"]);
    assert_eq!(v["data"]["ready"][0]["pr"], 2);
}

#[test]
fn approve_posts_the_comment_with_the_owner_token_and_records_it() {
    let fake = board();
    let (code, v) = approve(&fake, &[&at(1)]);
    assert_eq!(code, 0, "{v}");
    let posts = comment_posts(&fake);
    let on_pr: Vec<_> = posts
        .iter()
        .filter(|p| p.0.ends_with("/issues/1/comments"))
        .collect();
    assert_eq!(on_pr.len(), 1, "{posts:?}");
    let body: Value = serde_json::from_str(on_pr[0].1.as_ref().unwrap()).unwrap();
    assert_eq!(body["body"], format!("/bosun approve {}", sha(1)));
    assert_eq!(
        on_pr[0].2.as_deref(),
        Some("Bearer canary-owner-token-7f3a")
    );
    assert!(v["data"]["approved"][0]["comment_id"].as_u64().unwrap() > 0);
    assert_eq!(v["data"]["approved"][0]["pr"], 1);
    assert_eq!(v["data"]["approved"][0]["head_sha"], sha(1));
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
        json!({"bosun": "approve", "prs": [{"pr": 1, "head_sha": sha(1)}],
            "at": "2026-10-22T10:00:00Z"})
    );
}

#[test]
fn approving_what_the_list_printed_posts_every_comment_and_one_record() {
    let fake = board();
    let (_, v) = approve(&fake, &["--list"]);
    let cmd = v["data"]["approve_command"].as_str().unwrap().to_string();
    let args: Vec<&str> = cmd.split(' ').skip(2).collect();
    let (code, v) = approve(&fake, &args);
    assert_eq!(code, 0, "{v}");
    let prs: Vec<u64> = v["data"]["approved"]
        .as_array()
        .unwrap()
        .iter()
        .map(|a| a["pr"].as_u64().unwrap())
        .collect();
    assert_eq!(prs, [1, 3]);
    assert_eq!(records(&fake).len(), 1);
}

#[test]
fn a_bare_pr_number_or_a_bad_sha_is_a_usage_error() {
    let fake = board();
    for arg in [
        "1",
        "1@abc",
        &format!("x@{}", sha(1)),
        &format!("1@{}0", sha(1)),
    ] {
        let (code, v) = approve(&fake, &[arg]);
        assert_eq!(code, 2, "{arg} {v}");
        assert!(v["error"]["fix"].as_str().unwrap().contains("--list"));
    }
    assert!(fake.requests().is_empty());
}

#[test]
fn a_head_that_moved_after_the_list_posts_nothing() {
    let fake = board();
    let (_, v) = approve(&fake, &["--list"]);
    assert_eq!(v["data"]["ready"][0]["head_sha"], sha(1));
    // An agent pushes: PR 1 now has another head, with the same statuses copied by CI.
    let pushed = format!("{:040x}", 0xfeed);
    set_head(&fake, 1, &pushed);
    fake.status(&pushed, "risk-check", "failure", WAITING);
    fake.status(&pushed, "review-verdict", "success", "approved");
    fake.check_run(&pushed, "ci-ok", "completed", Some("success"));
    fake.compare_file("base0000", &pushed, "src/evil.rs", "@@");
    let (code, v) = approve(&fake, &[&at(3), &at(1)]);
    assert_eq!(code, 3, "{v}");
    assert!(comment_posts(&fake).is_empty());
    assert!(records(&fake).is_empty());
    assert_eq!(
        v["data"]["not_ready"],
        json!([{"pr": 1, "why": format!("head changed (now {pushed})")}])
    );
}

#[test]
fn one_pr_not_ready_posts_nothing_and_is_a_conflict() {
    let fake = board();
    fake.pull(7, "plain", "b7", "");
    set_head(&fake, 7, &sha(7));
    fake.status(&sha(7), "risk-check", "success", "normal risk");
    let args: Vec<String> = [1, 2, 4, 5, 6, 7, 99].iter().map(|n| at(*n)).collect();
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    let (code, v) = approve(&fake, &args);
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
fn a_failed_comment_still_records_the_ones_posted() {
    let fake = board();
    // Every comment on PR 3 fails, the one on PR 1 goes through (it comes first).
    fake.after_requests("/issues/1/comments", 0, |s| {
        s.fail_paths.push(("/issues/3/comments".into(), 422));
    });
    let (code, v) = approve(&fake, &[&at(1), &at(3)]);
    assert_eq!(code, 1, "{v}");
    assert_eq!(v["data"]["approved"].as_array().unwrap().len(), 1);
    assert_eq!(v["data"]["approved"][0]["pr"], 1);
    let recs = records(&fake);
    assert_eq!(recs.len(), 1);
    assert_eq!(recs[0]["prs"], json!([{"pr": 1, "head_sha": sha(1)}]));
}

#[test]
fn halted_posts_nothing() {
    let fake = board();
    fake.variable("BOSUN_HALT", "1");
    let one = at(1);
    for args in [&[one.as_str()][..], &["--main-open", "1"]] {
        let (code, v) = approve(&fake, args);
        assert_eq!(code, 4, "{args:?} {v}");
    }
    assert!(comment_posts(&fake).is_empty());
    assert!(
        fake.statuses_of(&sha(1))
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
        assert!(
            fix.contains("BOSUN_OWNER_TOKEN") && fix.contains("own machine"),
            "{fix}"
        );
        // A claude.ai cloud environment is never the place (#211).
        assert!(!fix.contains("environment"), "{fix}");
    }
    assert!(fake.requests().is_empty(), "no request without the token");
}

#[test]
fn the_owner_token_only_goes_to_https_or_loopback() {
    let fake = board();
    for api in ["http://api.github.com", "http://127.0.0.1.evil.example"] {
        let (code, v) = finish(
            approve_cmd(&fake)
                .env("BOSUN_GITHUB_API", api)
                .arg("--list"),
        );
        assert_eq!(code, 1, "{api} {v}");
        assert!(
            v["error"]["fix"]
                .as_str()
                .unwrap()
                .contains("BOSUN_GITHUB_API")
        );
    }
    assert!(fake.requests().is_empty());
}

#[test]
fn usage_errors_exit_2() {
    let fake = board();
    for args in [
        &[][..],
        &["--list", &at(1)],
        &["--list", "--all-ready"],
        &["--main-open", "1", "--list"],
        &["--main-open", "1", &at(2)],
    ] {
        let (code, v) = approve(&fake, args);
        assert_eq!(code, 2, "{args:?} {v}");
    }
    assert!(fake.requests().is_empty());
}

/// PR `n` changes the main-open workflow, as the bootstrap case does.
fn changes_main_open(fake: &FakeGithub, n: u64) {
    fake.pull_file(n, MAIN_OPEN_YML, "modified", 1, 0, None);
}

#[test]
fn main_open_posts_the_status_and_records_it() {
    let fake = board();
    changes_main_open(&fake, 1);
    let (code, v) = approve(&fake, &["--main-open", "1"]);
    assert_eq!(code, 0, "{v}");
    let s = fake
        .statuses_of(&sha(1))
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
    assert_eq!(recs[0]["main_open"], json!([{"pr": 1, "head_sha": sha(1)}]));
    for r in fake.requests() {
        assert_eq!(
            r.headers[0].as_deref(),
            Some("Bearer canary-owner-token-7f3a")
        );
    }
}

#[test]
fn main_open_only_for_a_pr_that_changes_the_workflow() {
    let fake = board();
    let (code, v) = approve(&fake, &["--main-open", "1"]);
    assert_eq!(code, 3, "{v}");
    assert!(
        v["error"]["fix"]
            .as_str()
            .unwrap()
            .contains("main-open workflow posts this status")
    );
    assert!(
        fake.statuses_of(&sha(1))
            .iter()
            .all(|s| s["context"] != "main-open")
    );
}

#[test]
fn main_open_follows_the_gate_when_main_is_red() {
    let fake = board();
    changes_main_open(&fake, 1);
    changes_main_open(&fake, 3);
    fake.issue(50, "main is red", &["bosun:main-red"], "");
    let (code, _) = approve(&fake, &["--main-open", "1"]);
    assert_eq!(code, 4);
    let s = fake
        .statuses_of(&sha(1))
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
    changes_main_open(&fake, 6);
    for pr in ["6", "99"] {
        let (code, _) = approve(&fake, &["--main-open", pr]);
        assert_eq!(code, 3, "PR {pr}");
    }
    assert!(
        fake.statuses_of(&sha(6))
            .iter()
            .all(|s| s["context"] != "main-open")
    );
}

#[test]
fn the_comment_counts_for_risk_check_as_the_owners_approval() {
    let fake = FakeGithub::start();
    ready(&fake, 1, "gov change");
    fake.content(
        "governance/owners.toml",
        "base0000",
        "[[owner]]\ngithub = \"octocat\"\n",
    );
    fake.pull_file(
        1,
        "governance/f1.toml",
        "modified",
        1,
        0,
        Some("@@ -0,0 +1,1 @@\n+x = 1"),
    );
    let before = |f: &FakeGithub| {
        run(
            f,
            "session_test",
            "2026-10-22T10:00:00Z",
            &["risk-check", "1"],
        )
    };
    let (code, v) = before(&fake);
    assert_eq!(code, 5, "not approved yet: {v}");
    let (code, v) = approve(&fake, &[&at(1)]);
    assert_eq!(code, 0, "{v}");
    let (code, v) = before(&fake);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["approved_by"], "octocat");
}

#[test]
fn the_owner_token_never_reaches_stdout_or_stderr() {
    // `finish()` asserts this on every run; this walks every mode, including GitHub errors.
    let fake = board();
    changes_main_open(&fake, 3);
    let args = [at(1), at(2), at(99)];
    for a in [
        &["--list"][..],
        &[&args[0]],
        &[&args[1]],
        &[&args[2]],
        &["--main-open", "3"],
        &["--main-open", "99"],
    ] {
        approve(&fake, a);
    }
    fake.fail_path("/pulls", 403);
    let (code, v) = approve(&fake, &["--list"]);
    assert_eq!(code, 1, "{v}");
}

#[test]
fn a_server_that_echoes_the_token_in_an_error_does_not_leak_it() {
    let fake = board();
    fake.echo_auth_on_fail();
    fake.fail_path("/pulls", 403);
    let (code, v) = approve(&fake, &["--list"]);
    assert_eq!(code, 1, "{v}");
    // The fake did put the header into GitHub's message; bosun redacted it.
    assert!(
        v["error"]["message"]
            .as_str()
            .unwrap()
            .contains("[redacted]")
    );
}

fn owners(fake: &FakeGithub) {
    fake.content(
        "governance/owners.toml",
        "base0000",
        "[[owner]]\ngithub = \"octocat\"\n",
    );
}

#[test]
fn a_claude_cloud_session_cannot_post_an_approval() {
    for (var, val) in [
        ("CCR_AGENT_PROXY_ENABLED", "1"),
        ("CCR_AGENT_PROXY_ENABLED", "true"),
        ("CLAUDE_CODE_REMOTE_ENVIRONMENT_TYPE", "cloud_default"),
    ] {
        let fake = board();
        changes_main_open(&fake, 1);
        let one = at(1);
        for args in [&[one.as_str()][..], &["--main-open", "1"]] {
            let (code, v) = finish(approve_cmd(&fake).env(var, val).args(args));
            assert_eq!(code, 5, "{var}={val} {args:?} {v}");
            assert!(v["error"]["message"].as_str().unwrap().contains("cloud"));
            assert!(v["error"]["fix"].as_str().unwrap().contains("own machine"));
        }
        assert!(fake.requests().is_empty(), "{var}={val}: no request at all");
    }
}

#[test]
fn list_works_in_a_cloud_session_and_says_so() {
    let fake = board();
    let (code, v) = finish(
        approve_cmd(&fake)
            .env("CCR_AGENT_PROXY_ENABLED", "1")
            .arg("--list"),
    );
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["cloud"], true);
    assert!(v["data"]["hint"].as_str().unwrap().contains("own machine"));
    let (_, v) = approve(&fake, &["--list"]);
    assert_eq!(v["data"]["cloud"], false);
    let (_, v) = finish(
        approve_cmd(&fake)
            .env("CCR_AGENT_PROXY_ENABLED", "0")
            .arg("--list"),
    );
    assert_eq!(v["data"]["cloud"], false);
}

const WITHDRAWN: &str = "an owner's review requests changes on this head; that owner approves with a review or dismisses it, or another owner approves with a GitHub review";

fn repost_of(v: &Value, i: usize) -> Option<&Value> {
    v["data"]["approved"][i].get("repost")
}

#[test]
fn a_counting_comment_is_posted_again_to_rerun_risk() {
    let fake = board();
    owners(&fake);
    let existing = fake.comment(1, &format!("/bosun approve {}", sha(1)));
    let (code, v) = approve(&fake, &[&at(1), &at(3)]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(repost_of(&v, 0), Some(&json!(true)));
    assert_ne!(v["data"]["approved"][0]["comment_id"], existing);
    assert!(repost_of(&v, 1).is_none());
    let posts = comment_posts(&fake);
    let on = |n: u64| {
        posts
            .iter()
            .filter(|p| p.0.ends_with(&format!("/issues/{n}/comments")))
            .count()
    };
    assert_eq!((on(1), on(3)), (1, 1));
    assert_eq!(records(&fake)[0]["prs"].as_array().unwrap().len(), 2);
}

#[test]
fn an_owner_login_in_another_case_still_counts_as_existing() {
    let fake = board();
    fake.content(
        "governance/owners.toml",
        "base0000",
        "[[owner]]\ngithub = \"OctoCat\"\n",
    );
    fake.comment(1, &format!("/bosun approve {}", sha(1)));
    let (_, v) = approve(&fake, &[&at(1)]);
    assert_eq!(repost_of(&v, 0), Some(&json!(true)));
}

#[test]
fn without_owners_no_comment_counts_as_existing() {
    let fake = board();
    fake.comment(1, &format!("/bosun approve {}", sha(1)));
    let (code, v) = approve(&fake, &[&at(1)]);
    assert_eq!(code, 0, "{v}");
    assert!(repost_of(&v, 0).is_none());
    assert_eq!(comment_posts(&fake).len(), 2, "the comment and the record");
}

#[test]
fn look_alike_comments_are_not_reposts() {
    let fake = board();
    owners(&fake);
    // PR 1: edited since it was written. PR 3: written through an app.
    fake.comment(1, &format!("/bosun approve {}", sha(1)));
    fake.last_comment(|c| c.updated_at = "2026-10-22T11:00:00Z".into());
    fake.comment(3, &format!("/bosun approve {}", sha(3)));
    fake.last_comment(|c| c.via_app = Some("claude".into()));
    let (code, v) = approve(&fake, &[&at(1), &at(3)]);
    assert_eq!(code, 0, "{v}");
    assert!(repost_of(&v, 0).is_none() && repost_of(&v, 1).is_none());
    let fake = board();
    owners(&fake);
    fake.comment_by(1, "mallory", &format!("/bosun approve {}", sha(1)));
    fake.comment(3, &format!("/bosun approve {}", sha(1)));
    let (_, v) = approve(&fake, &[&at(1), &at(3)]);
    assert!(repost_of(&v, 0).is_none() && repost_of(&v, 1).is_none());
}

#[test]
fn an_owner_review_requesting_changes_withdraws_the_approval() {
    let fake = board();
    owners(&fake);
    fake.comment(1, &format!("/bosun approve {}", sha(1)));
    fake.review(1, "octocat", "CHANGES_REQUESTED", &sha(1));
    let (_, v) = approve(&fake, &["--list"]);
    let ready: Vec<u64> = v["data"]["ready"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["pr"].as_u64().unwrap())
        .collect();
    assert_eq!(ready, [3]);
    assert!(
        v["data"]["waiting"]
            .as_array()
            .unwrap()
            .contains(&json!({"pr": 1, "why": WITHDRAWN}))
    );
    let before = comment_posts(&fake).len();
    let (code, v) = approve(&fake, &[&at(1), &at(3)]);
    assert_eq!(code, 3, "{v}");
    assert_eq!(v["data"]["not_ready"], json!([{"pr": 1, "why": WITHDRAWN}]));
    assert_eq!(comment_posts(&fake).len(), before, "nothing posted");
    // A later approving review, or a review of an older head, does not withdraw it.
    fake.review(1, "octocat", "APPROVED", &sha(1));
    let (_, v) = approve(&fake, &["--list"]);
    assert_eq!(v["data"]["ready"][0]["pr"], 1);
}
