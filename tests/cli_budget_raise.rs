//! Modes PR 7c: signed budget raises (`bosun budget raise`) lift the feature-budget ask.
mod common;

use common::fake::FakeGithub;
use common::{bosun, one_object};
use serde_json::{Value, json};

const NOW: &str = "2026-11-12T06:00:00Z";
const PUB: &str = "11qYAYKxCrfVS/7TyWQHOg7hcvPapiMlrwIaaPcHURo=";
const EPIC: &str = r#"Epic<!-- bosun:epic {"item":"M1","plan":"plans/x.md"} -->"#;
const INDEX: &str =
    r#"{"bosun":"epic-index","session":"s","plan":"plans/x.md","item":"M1","epic":10}"#;

fn plan_text() -> String {
    "# Plan: widget\n\nproject: widget\n\n## M1: One\n\narea: core\nbudget_usd: 100\n\nWords.\n\n\
### Tasks\n- T1: a\n  touch: src/a/**\n\n### Done when\n- UC-M1-1: works\n"
        .into()
}

fn enroll(fake: &FakeGithub, role: &str) {
    let keys = format!("[[key]]\nid = \"k1\"\ned25519 = \"{PUB}\"\nrole = \"{role}\"\n");
    fake.content(".bosun/approvers.toml", "refs/heads/main", &keys);
}

/// Epic #10 (budget 100, index on the registry) with 100 spent, and a ready task #11.
fn feature(mode: &str) -> FakeGithub {
    let fake = FakeGithub::start();
    fake.content(".bosun/registry", "refs/heads/main", "1");
    fake.content(
        ".bosun/bosun.toml",
        "main",
        &format!("[mode]\nname = \"{mode}\"\n"),
    );
    fake.issue(1, "bosun registry", &["bosun:registry"], "");
    let join = r#"{"bosun":"join","session":"session_test","origin":"spawned","depth":1,"expires":"2026-11-12T06:20:00Z"}"#;
    fake.comment(1, join);
    fake.comment(1, INDEX);
    fake.content("plans/x.md", "main", &plan_text());
    fake.issue(10, "Epic", &["bosun:state/ready", "bosun:area/core"], EPIC);
    let t = |task: &str| {
        format!(
            r#"<!-- bosun:task {{"item":"M1","epic":10,"task":"{task}","touch":["src/a/**"]}} -->"#
        )
    };
    fake.issue(11, "T1", &["bosun:state/ready"], &t("T1"));
    fake.issue(12, "T3", &["bosun:state/merged"], &t("T3"));
    let spend = r#"{"bosun":"spend","session":"s","usd":100.0,"at":"2026-11-12T03:00:00Z"}"#;
    fake.comment(12, spend);
    enroll(&fake, "owner");
    fake
}

fn run_env(fake: &FakeGithub, now: &str, repo: &str, args: &[&str], stdin: &str) -> (i32, Value) {
    let out = bosun()
        .env("BOSUN_GITHUB_API", fake.url())
        .env("BOSUN_HTTP_MAX_SLEEP_MS", "0")
        .env("BOSUN_SESSION", "session_test")
        .env("BOSUN_NOW", now)
        .env("BOSUN_REPO", repo)
        .args(args)
        .write_stdin(stdin.to_string())
        .output()
        .unwrap();
    (out.status.code().unwrap(), one_object(&out.stdout))
}

/// The owner's signed answer of `raise <epic> +<usd>` to a digest made at `now` in `repo`.
fn signed(fake: &FakeGithub, now: &str, repo: &str, reply: &str) -> String {
    signed_v(fake, now, repo, reply, 0)
}

/// As `signed`, for the digest variant `variant`: another shown pair, so another digest id on
/// the same day.
fn signed_v(fake: &FakeGithub, now: &str, repo: &str, reply: &str, variant: u32) -> String {
    let dir = tempfile::tempdir().unwrap();
    let key = dir.path().join("k.pem");
    std::fs::write(&key, common::TEST_ED25519_PEM).unwrap();
    let day = &now[..10];
    let pairs: Vec<String> = (1..=variant).map(|n| format!("{n}@{:040x}", n)).collect();
    let mut lines = pairs.clone();
    lines.push(format!("day {day}"));
    let id = bosun::core::approve::message_id(&lines);
    let head = format!("/bosun digest {repo} {day} {id} {}", pairs.join(" "));
    let input = format!("{head}\n{id} {reply}\n");
    let args = ["digest", "--answer", "--sign-key", key.to_str().unwrap()];
    let (code, v) = run_env(fake, now, repo, &args, &input);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["refused"], json!([]), "{v}");
    v.to_string()
}

fn raise(fake: &FakeGithub, now: &str, input: &str) -> (i32, Value) {
    let args = ["budget", "raise", "--from", "-"];
    let r = run_env(fake, now, "acme/widget", &args, input);
    // The fake stamps every comment with a fixed time; GitHub would stamp `now`.
    for c in fake.state.lock().unwrap().comments.iter_mut() {
        if c.body.contains(r#""bosun":"raise""#) && c.created_at.starts_with("2026-10-22T10:00:") {
            c.created_at = now.into();
        }
    }
    r
}

fn claim(fake: &FakeGithub) -> (i32, Value) {
    run_env(fake, NOW, "acme/widget", &["claim", "11"], "")
}

fn raises_on(fake: &FakeGithub) -> usize {
    fake.comments_of(10)
        .iter()
        .filter(|c| c.body.contains(r#""bosun":"raise""#))
        .count()
}

#[test]
fn a_valid_raise_is_recorded_and_lifts_the_ask() {
    let fake = feature("booster");
    assert_eq!(claim(&fake).0, 5);
    let input = signed(&fake, NOW, "acme/widget", "raise 10 +50");
    let (code, v) = raise(&fake, NOW, &input);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["recorded"][0]["epic"], 10, "{v}");
    assert_eq!(v["data"]["refused"], json!([]));
    assert_eq!(raises_on(&fake), 1);
    let (code, v) = claim(&fake);
    assert_eq!(code, 0, "{v}");
    // The bare array works too, and the same digest twice counts once.
    let arr = serde_json::from_str::<Value>(&input).unwrap()["data"]["raises"].to_string();
    let (code, v) = raise(&fake, NOW, &arr);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["recorded"], json!([]));
    assert!(
        v["data"]["refused"][0]["why"]
            .as_str()
            .unwrap()
            .contains("already"),
        "{v}"
    );
    assert_eq!(raises_on(&fake), 1);
}

fn refused_with(fake: &FakeGithub, now: &str, input: &str, why: &str) {
    let (code, v) = raise(fake, now, input);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["recorded"], json!([]), "{v}");
    let got = v["data"]["refused"][0]["why"].as_str().unwrap();
    assert!(got.contains(why), "{got} vs {why}");
    assert_eq!(raises_on(fake), 0);
}

#[test]
fn stale_future_other_repo_and_non_owner_raises_are_refused() {
    let fake = feature("booster");
    let input = signed(&fake, NOW, "acme/widget", "raise 10 +50");
    refused_with(&fake, "2026-11-15T06:00:00Z", &input, "stale");
    refused_with(&fake, "2026-11-10T06:00:00Z", &input, "future");
    let other = signed(&fake, NOW, "acme/other", "raise 10 +50");
    refused_with(&fake, NOW, &other, "repo");
    enroll(&fake, "delegate");
    refused_with(&fake, NOW, &input, "owner");
}

#[test]
fn an_unindexed_epic_an_over_cap_amount_and_a_bad_signature_are_refused() {
    let fake = feature("booster");
    let input = signed(&fake, NOW, "acme/widget", "raise 99 +50");
    refused_with(&fake, NOW, &input, "index");
    let input = signed(&fake, NOW, "acme/widget", "raise 10 +101");
    refused_with(&fake, NOW, &input, "cap");
    let input = signed(&fake, NOW, "acme/widget", "raise 10 +50");
    let bad = input.replace(r#""usd":50"#, r#""usd":60"#);
    refused_with(&fake, NOW, &bad, "match");
    let (_, v) = serde_json::from_str::<Value>(&input)
        .map(|v| (0, v["data"]["raises"][0].clone()))
        .unwrap();
    let mut forged = v.clone();
    forged["sig"] = json!("AAAA");
    refused_with(&fake, NOW, &json!([forged]).to_string(), "signature");
}

#[test]
fn a_forged_raise_comment_adds_nothing_and_gatekeeper_is_untouched() {
    let fake = feature("booster");
    let forged = r#"{"bosun":"raise","session":"s","epic":10,"usd":50,"digest":"d","day":"2026-11-12","key_id":"k1","message":"bosun-raise v1 acme/widget digest d day 2026-11-12 reply x epic 10 usd +50","sig":"AAAA"}"#;
    fake.comment(10, forged);
    assert_eq!(claim(&fake).0, 5);
    let fake = feature("gatekeeper");
    let (code, v) = claim(&fake);
    assert_eq!(code, 0, "{v}");
}

/// The record line of the first signed raise of `input`, as `bosun budget raise` would write it.
fn record_of(input: &str) -> String {
    record_on(input, "2026-11-12")
}

fn record_on(input: &str, day: &str) -> String {
    let e = serde_json::from_str::<Value>(input).unwrap()["data"]["raises"][0].clone();
    let m = e["message"].as_str().unwrap();
    let digest = m.split(' ').nth(4).unwrap();
    let usd = e["usd"].as_u64().unwrap();
    json!({"bosun": "raise", "session": "s", "epic": 10, "usd": usd, "digest": digest,
        "day": day, "key_id": "k1", "message": m, "sig": e["sig"]})
    .to_string()
}

/// A feature with 160 spent: +50 makes the budget 150 (over), a double count would make it 200.
fn spent_160() -> FakeGithub {
    spent_more(60)
}

fn spent_more(extra: u64) -> FakeGithub {
    let fake = feature("booster");
    let spend =
        format!(r#"{{"bosun":"spend","session":"s","usd":{extra}.0,"at":"2026-11-12T03:00:00Z"}}"#);
    fake.comment(12, &spend);
    fake
}

#[test]
fn a_raise_posted_late_or_with_an_unreadable_time_adds_nothing() {
    for at in ["2026-11-15T09:00:00Z", "not a time"] {
        let fake = feature("booster");
        let input = signed(&fake, NOW, "acme/widget", "raise 10 +50");
        fake.comment_at(10, "octocat", &record_of(&input), at);
        assert_eq!(claim(&fake).0, 5, "{at}");
    }
}

#[test]
fn the_same_raise_posted_twice_counts_once() {
    let fake = spent_160();
    let input = signed(&fake, NOW, "acme/widget", "raise 10 +50");
    for _ in 0..2 {
        fake.comment_at(10, "octocat", &record_of(&input), "2026-11-12T07:00:00Z");
    }
    assert_eq!(claim(&fake).0, 5, "counted once: 160 of 150");
    let fake = spent_160();
    fake.comment_at(10, "octocat", &record_of(&input), "2026-11-12T07:00:00Z");
    assert_eq!(claim(&fake).0, 5);
}

#[test]
fn a_recorded_raise_still_counts_after_its_day_has_passed() {
    let fake = feature("booster");
    let input = signed(&fake, NOW, "acme/widget", "raise 10 +50");
    assert_eq!(raise(&fake, NOW, &input).0, 0);
    let later = "2026-11-17T06:00:00Z";
    let (code, v) = run_env(&fake, later, "acme/widget", &["claim", "11"], "");
    assert_eq!(code, 0, "{v}");
}

#[test]
fn one_raise_a_day_at_record_and_read_time() {
    let fake = spent_more(40);
    let a = signed_v(&fake, NOW, "acme/widget", "raise 10 +10", 1);
    let b = signed_v(&fake, NOW, "acme/widget", "raise 10 +10", 2);
    assert_eq!(raise(&fake, NOW, &a).0, 0);
    let (code, v) = raise(&fake, NOW, &b);
    assert_eq!(code, 0, "{v}");
    assert!(
        v["data"]["refused"][0]["why"]
            .as_str()
            .unwrap()
            .contains("day"),
        "{v}"
    );
    assert_eq!(raises_on(&fake), 1);
    // Posted by hand, the second one is ignored: 140 spent of 110, not of 120.
    let fake = spent_more(45);
    let t = "2026-11-12T07:00:00Z";
    fake.comment_at(10, "octocat", &record_of(&a), t);
    fake.comment_at(10, "octocat", &record_of(&b), t);
    assert_eq!(claim(&fake).0, 5);
}

#[test]
fn raises_within_seven_days_are_capped_at_the_base_budget() {
    // Days 1 and 3 each 0.6 of the base: the second is refused at record time.
    let (d1, d3) = ("2026-11-10T06:00:00Z", "2026-11-12T06:00:00Z");
    let fake = feature("booster");
    let a = signed(&fake, d1, "acme/widget", "raise 10 +60");
    let b = signed(&fake, d3, "acme/widget", "raise 10 +60");
    assert_eq!(raise(&fake, d1, &a).0, 0);
    let (_, v) = raise(&fake, d3, &b);
    assert!(
        v["data"]["refused"][0]["why"]
            .as_str()
            .unwrap()
            .contains("cap"),
        "{v}"
    );
    // By hand the second adds nothing: 170 spent of 160, not of 220.
    let fake = spent_more(70);
    fake.comment_at(10, "octocat", &record_on(&a, "2026-11-10"), d1);
    fake.comment_at(10, "octocat", &record_on(&b, "2026-11-12"), d3);
    assert_eq!(claim(&fake).0, 5);
    // Day 1 and day 8, each the whole base: both count (260 spent of 300).
    let (d1, d8) = ("2026-11-05T06:00:00Z", "2026-11-12T06:00:00Z");
    let fake = spent_more(160);
    let a = signed(&fake, d1, "acme/widget", "raise 10 +100");
    let b = signed(&fake, d8, "acme/widget", "raise 10 +100");
    for (i, at) in [(&a, d1), (&b, d8)] {
        let (_, v) = raise(&fake, at, i);
        assert_eq!(v["data"]["refused"], json!([]), "{v}");
    }
    let (code, v) = claim(&fake);
    assert_eq!(code, 0, "{v}");
}

#[test]
fn raises_are_capped_at_twice_the_base_budget_in_all() {
    // Days 1, 8 and 15, each the whole base: the third is refused (lifetime 2x base).
    let days = [
        "2026-10-29T06:00:00Z",
        "2026-11-05T06:00:00Z",
        "2026-11-12T06:00:00Z",
    ];
    let fake = spent_more(210);
    let mut refused = vec![];
    for at in days {
        let input = signed(&fake, at, "acme/widget", "raise 10 +100");
        let (_, v) = raise(&fake, at, &input);
        refused.push(v["data"]["refused"].clone());
    }
    assert_eq!(refused[1], json!([]));
    let why = refused[2][0]["why"]
        .as_str()
        .unwrap_or_default()
        .to_string();
    assert!(why.contains("in all"), "{refused:?}");
    // 310 spent of 300 (base 100 + 200): still over, the third adds nothing.
    assert_eq!(claim(&fake).0, 5);
}

#[test]
fn a_github_error_exits_1_and_a_rerun_is_safe_and_bad_input_is_refused() {
    let fake = feature("booster");
    let input = signed(&fake, NOW, "acme/widget", "raise 10 +50");
    fake.fail_path("issues/1/comments", 500);
    let (code, v) = raise(&fake, NOW, &input);
    assert_eq!(code, 1, "{v}");
    assert_eq!(raises_on(&fake), 0);
    fake.state.lock().unwrap().fail_paths.clear();
    assert_eq!(raise(&fake, NOW, &input).0, 0);
    let (_, again) = raise(&fake, NOW, &input);
    assert_eq!(raises_on(&fake), 1, "{again}");
    let bad = r#"[{"key_id":"k1","message":"x","sig":"y"}]"#;
    let (code, v) = raise(&fake, NOW, bad);
    assert_eq!(code, 0, "{v}");
    let r = &v["data"]["refused"][0];
    assert_eq!((&r["epic"], &r["usd"]), (&Value::Null, &Value::Null));
    assert_eq!(raise(&fake, NOW, r#"{"data":{}}"#).0, 2);
}
