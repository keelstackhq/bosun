mod common;

use common::fake::FakeGithub;
use common::{bosun, one_object};
use serde_json::{Value, json};

const WAITING: &str = "high risk, owner approval needed: R-gov, R-ci";
/// `BOSUN_NOW` of `bosun()`, 2026-10-22T10:00:00Z, in unix seconds.
const UNIX: i64 = 1_792_663_200;

fn head(prefix: &str) -> String {
    format!("{prefix:0<40}")
}

/// PR `n` at `head`, ready for an owner.
fn ready(fake: &FakeGithub, n: u64, title: &str, head: &str) {
    fake.pull(n, title, &format!("b{n}"), "");
    fake.pull_as_issue(n, title);
    fake.pull_set(
        n,
        "head",
        json!({"sha": head, "ref": format!("b{n}"), "label": format!("acme:b{n}")}),
    );
    fake.status(head, "risk-check", "failure", WAITING);
    fake.status(head, "review-verdict", "success", "approved by session_rev");
    fake.check_run(head, "ci-ok", "completed", Some("success"));
    fake.compare_file("base0000", head, &format!("governance/f{n}.toml"), "@@");
}

fn list(fake: &FakeGithub, origin: Option<&str>) -> Value {
    let mut cmd = bosun();
    cmd.env("BOSUN_GITHUB_API", fake.url())
        .env("BOSUN_OWNER_TOKEN", "canary-owner-token-7f3a")
        .env("BOSUN_HTTP_MAX_SLEEP_MS", "0")
        .args(["approve", "--list"]);
    if let Some(o) = origin {
        cmd.env("BOSUN_ORIGIN", o);
    }
    let out = cmd.output().unwrap();
    assert!(!String::from_utf8_lossy(&out.stdout).contains("canary-owner-token"));
    let v = one_object(&out.stdout);
    assert_eq!(out.status.code(), Some(0), "{v}");
    v
}

#[test]
fn list_one_ready_has_ask_and_next() {
    let fake = FakeGithub::start();
    let h = head("abc1234");
    ready(&fake, 12, "gov change", &h);
    let v = list(&fake, None);
    let qid = format!("12-abc1234-{UNIX}");
    let ask = &v["data"]["ready"][0]["ask"];
    assert_eq!(ask["question_id"], json!(qid));
    assert_eq!(ask["options"], json!(["Approve abc1234", "Hold"]));
    let q = ask["question"].as_str().unwrap();
    for part in [
        "acme/widget#12",
        "`gov change`",
        "R-gov, R-ci",
        "abc1234",
        &h,
        "approved by session_rev",
        &format!("[bosun:{qid}]"),
    ] {
        assert!(q.contains(part), "{part} not in {q}");
    }
    let next = v["next"].as_array().unwrap();
    assert_eq!(next.len(), 1, "{v}");
    assert_eq!(next[0]["tool"], json!("AskUserQuestion"));
    let q0 = &next[0]["args"]["questions"][0];
    assert_eq!(q0["header"], json!("Approve"));
    assert_eq!(q0["multiSelect"], json!(false));
    assert_eq!(q0["question"], json!(q));
    let labels: Vec<_> = q0["options"]
        .as_array()
        .unwrap()
        .iter()
        .map(|o| o["label"].clone())
        .collect();
    assert_eq!(labels, vec![json!("Approve #12 abc1234"), json!("Hold")]);
    assert!(
        q0["options"]
            .as_array()
            .unwrap()
            .iter()
            .all(|o| o["description"].as_str().is_some_and(|d| !d.is_empty()))
    );
}

#[test]
fn list_two_ready_multiselect() {
    let fake = FakeGithub::start();
    ready(&fake, 15, "second", &head("def5678"));
    ready(&fake, 12, "first", &head("abc1234"));
    let v = list(&fake, None);
    let next = v["next"].as_array().unwrap();
    assert_eq!(next.len(), 1);
    let q0 = &next[0]["args"]["questions"][0];
    assert_eq!(q0["multiSelect"], json!(true));
    let labels: Vec<_> = q0["options"]
        .as_array()
        .unwrap()
        .iter()
        .map(|o| o["label"].clone())
        .collect();
    assert_eq!(
        labels,
        vec![
            json!("Approve #12 abc1234"),
            json!("Approve #15 def5678"),
            json!("Hold")
        ]
    );
    let q = q0["question"].as_str().unwrap();
    assert!(q.contains(&format!("[bosun:12-abc1234-{UNIX}]")), "{q}");
    assert!(q.contains(&format!("[bosun:15-def5678-{UNIX}]")), "{q}");
    assert!(q.contains("acme/widget#15"), "{q}");
}

#[test]
fn list_four_ready_asks_three_and_says_so() {
    // AskUserQuestion takes at most 4 options: 3 PRs + Hold; the rest wait for the next list.
    let fake = FakeGithub::start();
    for (n, p) in [(1, "a"), (2, "b"), (3, "c"), (4, "d")] {
        ready(&fake, n, "t", &head(p));
    }
    let v = list(&fake, None);
    let opts = v["next"][0]["args"]["questions"][0]["options"]
        .as_array()
        .unwrap()
        .len();
    assert_eq!(opts, 4);
    assert_eq!(v["data"]["ask_more"], json!(1));
    assert!(v["data"]["ready"][3]["ask"].is_object());
}

#[test]
fn list_slack_origin_next_is_slack_post() {
    let fake = FakeGithub::start();
    ready(&fake, 12, "gov change", &head("abc1234"));
    let v = list(&fake, Some("slack"));
    let next = v["next"].as_array().unwrap();
    assert_eq!(next.len(), 1, "{v}");
    assert_eq!(next[0]["tool"], json!("slack_post"));
    let text = next[0]["args"]["text"].as_str().unwrap();
    assert!(text.contains("acme/widget#12"), "{text}");
    assert!(text.ends_with("answer \"approve abc1234\""), "{text}");
}

#[test]
fn list_none_ready_no_next() {
    let fake = FakeGithub::start();
    let v = list(&fake, None);
    assert_eq!(v["next"], json!([]));
    assert_eq!(v["data"]["ready"], json!([]));
}

#[test]
fn question_ids_stable_per_now() {
    let fake = FakeGithub::start();
    ready(&fake, 12, "gov change", &head("abc1234"));
    let a = list(&fake, None);
    let b = list(&fake, None);
    assert_eq!(a["next"], b["next"]);
    assert_eq!(a["data"]["ready"][0]["ask"], b["data"]["ready"][0]["ask"]);
}
