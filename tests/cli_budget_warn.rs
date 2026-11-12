//! Modes PR 7e: the 80 % feature-budget warning in status, the digest's budget asks and the
//! wave tick. Gatekeeper output stays as it was.
mod common;

use common::fake::FakeGithub;
use serde_json::{Value, json};

const NOW: &str = "2026-11-12T06:00:00Z";
const EPIC: &str = r#"Epic<!-- bosun:epic {"item":"M1","plan":"plans/x.md"} -->"#;
const INDEX: &str =
    r#"{"bosun":"epic-index","session":"s","plan":"plans/x.md","item":"M1","epic":10}"#;

fn plan_text() -> String {
    "# Plan: widget\n\nproject: widget\n\n## M1: One\n\narea: core\nbudget_usd: 100\n\nWords.\n\n\
### Tasks\n- T1: a\n  touch: src/a/**\n\n### Done when\n- UC-M1-1: works\n"
        .into()
}

/// Epic #10 (budget 100, indexed on the registry) with `usd` spent and a ready task #11.
fn feature(mode: &str, usd: f64) -> FakeGithub {
    let fake = FakeGithub::start();
    if !mode.is_empty() {
        let t = format!("[mode]\nname = \"{mode}\"\n");
        fake.content(".bosun/bosun.toml", "main", &t);
        fake.content(".bosun/registry", "refs/heads/main", "1");
    }
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
    let spend =
        format!(r#"{{"bosun":"spend","session":"s","usd":{usd},"at":"2026-11-12T03:00:00Z"}}"#);
    fake.comment(12, &spend);
    fake
}

fn run_in(fake: &FakeGithub, args: &[&str]) -> (i32, Value) {
    let dir = tempfile::tempdir().unwrap();
    let mut a = vec!["-C", dir.path().to_str().unwrap()];
    a.extend_from_slice(args);
    common::run(fake, "session_test", NOW, &a)
}

#[test]
fn status_lists_an_85_percent_epic_under_steady_only() {
    let (code, v) = run_in(&feature("steady", 85.0), &["status"]);
    assert_eq!(code, 0, "{v}");
    let l = &v["data"]["budget_feature"];
    assert_eq!(l.as_array().unwrap().len(), 1, "{v}");
    assert_eq!(
        (l[0]["epic"].as_u64(), l[0]["level"].as_str()),
        (Some(10), Some("warn"))
    );
    assert_eq!(l[0]["spent_usd"].as_f64(), Some(85.0));
    assert_eq!(l[0]["budget_usd"].as_f64(), Some(100.0));
    let (_, v) = run_in(&feature("steady", 50.0), &["status"]);
    assert_eq!(v["data"]["budget_feature"], json!([]), "{v}");
    let (code, v) = run_in(&feature("", 85.0), &["status"]);
    assert_eq!(code, 0, "{v}");
    assert!(v["data"].get("budget_feature").is_none(), "{v}");
}

#[test]
fn digest_asks_for_a_120_percent_epic_under_booster_only() {
    let (code, v) = run_in(&feature("booster", 120.0), &["digest"]);
    assert_eq!(code, 0, "{v}");
    let asks = v["data"]["budget_asks"].as_array().unwrap();
    assert_eq!(asks.len(), 1, "{v}");
    assert_eq!(asks[0]["epic"], 10);
    assert_eq!(asks[0]["spent_usd"].as_f64(), Some(120.0));
    assert_eq!(asks[0]["budget_usd"].as_f64(), Some(100.0));
    let id = v["data"]["message_id"].as_str().unwrap();
    let msg = v["data"]["message"].as_str().unwrap();
    assert!(msg.contains("Budget asks (1):"), "{msg}");
    assert!(msg.contains(&format!("{id} raise 10 +45")), "{msg}");
    // Under 100 % and under steady nothing is asked, and the id is the same either way.
    let (_, w) = run_in(&feature("booster", 85.0), &["digest"]);
    assert_eq!(w["data"]["budget_asks"], json!([]), "{w}");
    assert_eq!(w["data"]["message_id"], v["data"]["message_id"]);
    let (_, s) = run_in(&feature("steady", 120.0), &["digest"]);
    assert_eq!(s["data"]["budget_asks"], json!([]), "{s}");
    let (_, g) = run_in(&feature("", 120.0), &["digest"]);
    assert_eq!(g["data"]["budget_asks"], json!([]), "{g}");
    let gm = g["data"]["message"].as_str().unwrap();
    assert!(gm.ends_with("Budget asks (0):"), "{gm}");
}

#[test]
fn a_wave_tick_copies_the_warning_into_the_spawned_entry() {
    let (code, v) = run_in(&feature("booster", 85.0), &["wave", "tick", "10"]);
    assert_eq!(code, 0, "{v}");
    let s = &v["data"]["spawned"];
    assert_eq!(s[0]["issue"], 11, "{v}");
    assert_eq!(s[0]["budget_feature"]["level"], "warn", "{v}");
    assert_eq!(s[0]["budget_feature"]["epic"], 10);
    let (_, v) = run_in(&feature("booster", 10.0), &["wave", "tick", "10"]);
    assert!(
        v["data"]["spawned"][0].get("budget_feature").is_none(),
        "{v}"
    );
}

const PUB: &str = "11qYAYKxCrfVS/7TyWQHOg7hcvPapiMlrwIaaPcHURo=";

fn enroll(fake: &FakeGithub) {
    let keys = format!("[[key]]\nid = \"k1\"\ned25519 = \"{PUB}\"\nrole = \"owner\"\n");
    fake.content(".bosun/approvers.toml", "refs/heads/main", &keys);
}

/// The record line of an owner-signed `raise 10 +<usd>` for the digest of `day`, posted on #10.
fn post_raise(fake: &FakeGithub, day: &str, usd: u64) {
    let dir = tempfile::tempdir().unwrap();
    let key = dir.path().join("k.pem");
    std::fs::write(&key, common::TEST_ED25519_PEM).unwrap();
    let id = bosun::core::approve::message_id(&[format!("day {day}")]);
    let head = format!("/bosun digest acme/widget {day} {id} ");
    let input = format!("{head}\n{id} raise 10 +{usd}\n");
    let at = format!("{day}T06:00:00Z");
    let out = common::bosun()
        .env("BOSUN_GITHUB_API", fake.url())
        .env("BOSUN_SESSION", "session_test")
        .env("BOSUN_NOW", &at)
        .env("BOSUN_REPO", "acme/widget")
        .args(["digest", "--answer", "--sign-key", key.to_str().unwrap()])
        .write_stdin(input)
        .output()
        .unwrap();
    let v = common::one_object(&out.stdout);
    let e = v["data"]["raises"][0].clone();
    assert!(e.is_object(), "{v}");
    let m = e["message"].as_str().unwrap();
    let digest = m.split(' ').nth(4).unwrap();
    let rec = json!({"bosun": "raise", "session": "s", "epic": 10, "usd": usd, "digest": digest,
        "day": day, "key_id": "k1", "message": m, "sig": e["sig"]});
    fake.comment_at(10, "octocat", &rec.to_string(), &format!("{day}T07:00:00Z"));
}

#[test]
fn the_reply_hint_is_capped_by_what_a_raise_may_still_add() {
    let fake = feature("booster", 250.0);
    enroll(&fake);
    let (_, v) = run_in(&fake, &["digest"]);
    let id = v["data"]["message_id"].as_str().unwrap();
    let msg = v["data"]["message"].as_str().unwrap();
    assert!(msg.contains(&format!("{id} raise 10 +100")), "{msg}");
    assert!(v["data"]["budget_asks"][0].get("needs").is_none(), "{v}");
    // +100 raised this week: base 100 leaves nothing, so no reply line and the plan PR is named.
    let fake = feature("booster", 230.0);
    enroll(&fake);
    post_raise(&fake, "2026-11-10", 100);
    let (_, v) = run_in(&fake, &["digest"]);
    let msg = v["data"]["message"].as_str().unwrap();
    assert!(!msg.contains(" raise 10 +"), "{msg}");
    let ask = &v["data"]["budget_asks"][0];
    assert_eq!(ask["epic"], 10, "{v}");
    assert_eq!(ask["budget_usd"].as_f64(), Some(200.0), "{v}");
    assert_eq!(ask["needs"], "R-gov plan PR", "{v}");
}

#[test]
fn closed_epics_are_not_listed_and_a_bad_epic_is_skipped() {
    let fake = feature("steady", 85.0);
    // #20 is indexed and closed; #30 is indexed and open, but its comments fail.
    let idx = |item: &str, epic: u64| {
        format!(
            r#"{{"bosun":"epic-index","session":"s","plan":"plans/x.md","item":"{item}","epic":{epic}}}"#
        )
    };
    fake.comment(1, &idx("M2", 20));
    fake.comment(1, &idx("M3", 30));
    let body = |i: &str| format!(r#"E<!-- bosun:epic {{"item":"{i}","plan":"plans/x.md"}} -->"#);
    fake.issue(20, "Closed", &["bosun:area/core"], &body("M2"));
    fake.close_issue(20, "2026-11-11T00:00:00Z");
    fake.issue(30, "Bad", &["bosun:area/core"], &body("M3"));
    fake.fail_path("/issues/30/comments", 500);
    let (code, v) = run_in(&fake, &["status"]);
    assert_eq!(code, 0, "{v}");
    let l = v["data"]["budget_feature"].as_array().unwrap();
    assert_eq!(l.len(), 1, "{v}");
    assert_eq!(l[0]["epic"], 10);
    let s = v["data"]["budget_skipped"].as_array().unwrap();
    assert_eq!(s.len(), 1, "{v}");
    assert_eq!(s[0]["epic"], 30);
    assert!(!s[0]["error"].as_str().unwrap().is_empty());
}

#[test]
fn an_epic_with_a_raise_is_measured_against_base_plus_raise() {
    let fake = feature("steady", 150.0);
    enroll(&fake);
    post_raise(&fake, "2026-11-12", 50);
    let (_, v) = run_in(&fake, &["status"]);
    let l = &v["data"]["budget_feature"][0];
    assert_eq!(l["budget_usd"].as_f64(), Some(150.0), "{v}");
    assert_eq!(l["level"], "over", "{v}");
    let fake = feature("steady", 140.0);
    enroll(&fake);
    post_raise(&fake, "2026-11-12", 50);
    let (_, v) = run_in(&fake, &["status"]);
    assert_eq!(v["data"]["budget_feature"][0]["level"], "warn", "{v}");
}

#[test]
fn digest_lists_85_percent_epics_as_warnings_without_a_reply_hint() {
    let (_, base) = run_in(&feature("steady", 10.0), &["digest"]);
    assert_eq!(base["data"]["budget_warnings"], json!([]), "{base}");
    assert!(
        !base["data"]["message"]
            .as_str()
            .unwrap()
            .contains("warnings")
    );
    let (code, v) = run_in(&feature("steady", 85.0), &["digest"]);
    assert_eq!(code, 0, "{v}");
    let w = &v["data"]["budget_warnings"];
    assert_eq!(w.as_array().unwrap().len(), 1, "{v}");
    assert_eq!(w[0]["epic"], 10);
    assert_eq!(w[0]["spent_usd"].as_f64(), Some(85.0));
    assert_eq!(w[0]["budget_usd"].as_f64(), Some(100.0));
    let msg = v["data"]["message"].as_str().unwrap();
    assert!(
        msg.contains("Budget warnings (1):\n- epic #10 $85.00 of $100.00 (85 %)"),
        "{msg}"
    );
    assert!(!msg.contains(" raise "), "{msg}");
    assert_eq!(v["data"]["message_id"], base["data"]["message_id"]);
    assert_eq!(v["data"]["budget_asks"], json!([]));
    // Over 100 % under warn is a warning too; under ask the 85 % epic is a warning, not an ask.
    let (_, o) = run_in(&feature("steady", 120.0), &["digest"]);
    assert_eq!(
        o["data"]["budget_warnings"][0]["spent_usd"].as_f64(),
        Some(120.0),
        "{o}"
    );
    let (_, a) = run_in(&feature("booster", 85.0), &["digest"]);
    assert_eq!(a["data"]["budget_warnings"][0]["epic"], 10, "{a}");
    assert_eq!(a["data"]["budget_asks"], json!([]));
    // Gatekeeper: no key, no section.
    let (_, g) = run_in(&feature("", 85.0), &["digest"]);
    assert!(g["data"].get("budget_warnings").is_none(), "{g}");
    assert!(
        g["data"]["message"]
            .as_str()
            .unwrap()
            .ends_with("Budget asks (0):")
    );
}

#[test]
fn a_gatekeeper_wave_tick_has_no_budget_feature_and_the_old_shape() {
    let fake = feature("", 85.0);
    let (code, v) = run_in(&fake, &["wave", "tick", "10"]);
    assert_eq!(code, 0, "{v}");
    assert!(!v.to_string().contains("budget_feature"), "{v}");
    let mut keys: Vec<&String> = v["data"].as_object().unwrap().keys().collect();
    keys.sort();
    let want = [
        "archived",
        "brake",
        "environment_id",
        "epic",
        "held_by",
        "mode",
        "spawn_auto",
    ];
    let want: Vec<&str> = want.iter().copied().chain(["spawned", "stopped"]).collect();
    assert_eq!(keys, want, "{v}");
}

#[test]
fn status_survives_a_failing_budget_read() {
    let fake = feature("steady", 85.0);
    fake.fail_path("contents/.bosun/bosun.toml", 500);
    let (code, v) = run_in(&fake, &["status"]);
    assert_eq!(code, 0, "{v}");
    let s = &v["data"]["budget_skipped"][0];
    assert!(s["epic"].is_null(), "{v}");
    assert!(!s["error"].as_str().unwrap().is_empty(), "{v}");
}

#[test]
fn the_hint_adds_one_task_estimate_and_the_raise_lifts_the_claim() {
    let fake = feature("booster", 130.0);
    enroll(&fake);
    let (_, v) = run_in(&fake, &["digest"]);
    let id = v["data"]["message_id"].as_str().unwrap();
    // Overspend 30 plus the 25 a task is estimated at.
    let hint = format!("{id} raise 10 +55");
    assert!(v["data"]["message"].as_str().unwrap().contains(&hint));
    let dir = tempfile::tempdir().unwrap();
    let key = dir.path().join("k.pem");
    std::fs::write(&key, common::TEST_ED25519_PEM).unwrap();
    let head = format!("/bosun digest acme/widget 2026-11-12 {id}");
    let args = ["digest", "--answer", "--sign-key", key.to_str().unwrap()];
    let out = common::bosun()
        .env("BOSUN_GITHUB_API", fake.url())
        .env("BOSUN_SESSION", "session_test")
        .env("BOSUN_NOW", NOW)
        .env("BOSUN_REPO", "acme/widget")
        .args(args)
        .write_stdin(format!("{head}\n{hint}\n"))
        .output()
        .unwrap();
    let signed = common::one_object(&out.stdout)["data"]["raises"].clone();
    assert_eq!(signed.as_array().map(Vec::len), Some(1));
    let out = common::bosun()
        .env("BOSUN_GITHUB_API", fake.url())
        .env("BOSUN_SESSION", "session_test")
        .env("BOSUN_NOW", NOW)
        .env("BOSUN_REPO", "acme/widget")
        .args(["budget", "raise", "--from", "-"])
        .write_stdin(signed.to_string())
        .output()
        .unwrap();
    let r = common::one_object(&out.stdout);
    assert_eq!(r["data"]["recorded"][0]["usd"], 55, "{r}");
    // The fake stamps every comment with a fixed time; GitHub would stamp `now`.
    for c in fake.state.lock().unwrap().comments.iter_mut() {
        if c.body.contains(r#""bosun":"raise""#) {
            c.created_at = NOW.into();
        }
    }
    let (code, v) = run_in(&fake, &["claim", "11"]);
    assert_eq!(code, 0, "{v}");
}

#[test]
fn no_bare_reply_line_when_no_ask_has_room() {
    let fake = feature("booster", 230.0);
    enroll(&fake);
    post_raise(&fake, "2026-11-10", 100);
    let (_, v) = run_in(&fake, &["digest"]);
    let msg = v["data"]["message"].as_str().unwrap();
    assert!(msg.contains("needs an R-gov plan PR"), "{msg}");
    assert!(!msg.contains("Reply with"), "{msg}");
}
