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
    assert!(msg.contains(&format!("{id} raise 10 +20")), "{msg}");
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
