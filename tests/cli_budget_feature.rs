//! Modes item 6: the per-feature budget (`budget_usd:` on a plan item, warn/ask at 80/100 %).
mod common;

use bosun::core::budget::{Feature, feature_estimate, feature_level};
use bosun::core::meta::epic_budget;
use bosun::core::plan::parse;
use common::fake::FakeGithub;
use serde_json::Value;
use std::path::Path;

const NOW: &str = "2026-10-22T10:00:00Z";
const LATER: &str = "2026-10-22T10:20:00Z";

fn plan_text(field: &str) -> String {
    format!(
        "# Plan: widget\n\nproject: widget\n\n## M1: One\n\narea: core\n{field}\n\nWords.\n\n\
### Tasks\n- T1: a\n  touch: src/a/**\n- T2: b\n  touch: src/b/**\n\n### Done when\n- UC-M1-1: works\n"
    )
}

#[test]
fn plan_parses_budget_usd_once_and_positive() {
    assert_eq!(parse(&plan_text("")).unwrap().items[0].budget_usd, None);
    let p = parse(&plan_text("budget_usd: 120.5")).unwrap();
    assert_eq!(p.items[0].budget_usd, Some(120.5));
    for bad in [
        "budget_usd: 0",
        "budget_usd: -3",
        "budget_usd: abc",
        "budget_usd: inf",
    ] {
        let errs = parse(&plan_text(bad)).unwrap_err();
        assert!(errs[0].message.contains("budget_usd"), "{bad}: {errs:?}");
        assert!(!errs[0].fix.is_empty());
    }
    let twice = parse(&plan_text("budget_usd: 5\nbudget_usd: 6")).unwrap_err();
    assert!(twice[0].message.contains("given twice"), "{twice:?}");
}

#[test]
fn levels_and_estimate() {
    assert_eq!(feature_estimate(4, 25.0), 100.0);
    assert_eq!(feature_level(79.9, 100.0), Feature::Under);
    assert_eq!(feature_level(80.0, 100.0), Feature::Warn);
    assert_eq!(feature_level(99.99, 100.0), Feature::Warn);
    assert_eq!(feature_level(100.0, 100.0), Feature::Over);
    for b in [0.0, -1.0, f64::NAN, f64::INFINITY] {
        assert_eq!(feature_level(500.0, b), Feature::Under, "{b}");
    }
}

#[test]
fn epic_budget_reads_the_last_json_marker() {
    let one = r#"x <!-- bosun:epic {"item":"M1","budget_usd":90.0} -->"#;
    assert_eq!(epic_budget(one), Some(90.0));
    let two = "<!-- bosun:epic {\"budget_usd\":1} -->\n<!-- bosun:epic {\"budget_usd\":2.5} -->";
    assert_eq!(epic_budget(two), Some(2.5));
    assert_eq!(epic_budget(r#"<!-- bosun:epic {"item":"M1"} -->"#), None);
    assert_eq!(epic_budget("<!-- bosun:epic not json -->"), None);
    assert_eq!(epic_budget("no marker"), None);
}

fn cfg_dir(mode: &str) -> tempfile::TempDir {
    let d = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(d.path().join(".bosun")).unwrap();
    if !mode.is_empty() {
        let t = format!("[mode]\nname = \"{mode}\"\n");
        std::fs::write(d.path().join(".bosun/bosun.toml"), t).unwrap();
    }
    d
}

/// The mode on the default branch, where claim, spawn and import read it.
fn on_main(fake: &FakeGithub, mode: &str) {
    if !mode.is_empty() {
        let t = format!("[mode]\nname = \"{mode}\"\n");
        fake.content(".bosun/bosun.toml", "main", &t);
    }
}

fn run_in(fake: &FakeGithub, dir: &Path, args: &[&str]) -> (i32, Value) {
    let mut a = vec!["-C", dir.to_str().unwrap()];
    a.extend_from_slice(args);
    common::run(fake, "session_test", NOW, &a)
}

fn import(mode: &str, field: &str) -> String {
    let (dir, fake) = (cfg_dir(""), FakeGithub::start());
    on_main(&fake, mode);
    let file = dir.path().join("plan.md");
    std::fs::write(&file, plan_text(field)).unwrap();
    let f = file.to_str().unwrap();
    let (code, v) = run_in(&fake, dir.path(), &["plan", "import", f]);
    assert_eq!(code, 0, "{v}");
    let s = fake.state.lock().unwrap();
    let marker = "<!-- bosun:plan M1 -->";
    let epic = s.issues.iter().find(|i| i.body.starts_with(marker));
    epic.unwrap().body.clone()
}

#[test]
fn gatekeeper_epic_body_is_unchanged_without_a_budget() {
    let body = import("", "");
    assert!(!body.contains("budget"), "{body}");
    let meta = body.rsplit("<!-- bosun:epic ").next().unwrap();
    let v: Value = serde_json::from_str(meta.trim_end_matches(" -->")).unwrap();
    let mut keys: Vec<&String> = v.as_object().unwrap().keys().collect();
    keys.sort();
    assert_eq!(keys, ["area", "depends", "item", "plan", "shape", "ucs"]);
}

#[test]
fn epic_meta_carries_the_plan_budget_or_the_per_task_estimate() {
    let plan = import("", "budget_usd: 77");
    assert!(plan.contains(r#""budget_usd":77.0"#), "{plan}");
    assert!(plan.contains(r#""budget_source":"plan""#), "{plan}");
    // Steady, no field: 2 tasks x per_task_usd (25 by default).
    let est = import("steady", "");
    assert!(est.contains(r#""budget_usd":50.0"#), "{est}");
    assert!(est.contains(r#""budget_source":"per_task""#), "{est}");
    // An explicit field wins over the estimate.
    let both = import("steady", "budget_usd: 77");
    assert!(both.contains(r#""budget_usd":77.0"#), "{both}");
    assert!(both.contains(r#""budget_source":"plan""#), "{both}");
}

#[test]
fn plan_check_prints_budget_only_when_set_or_knob_on() {
    let item = |mode: &str, field: &str| {
        let (dir, fake) = (cfg_dir(mode), FakeGithub::start());
        let file = dir.path().join("plan.md");
        std::fs::write(&file, plan_text(field)).unwrap();
        let f = file.to_str().unwrap();
        let (code, v) = run_in(&fake, dir.path(), &["plan", "check", f]);
        assert_eq!(code, 0, "{v}");
        v["data"]["items"][0].clone()
    };
    assert!(item("", "").get("budget_usd").is_none());
    assert_eq!(item("", "budget_usd: 9")["budget_usd"], 9.0);
    assert_eq!(item("steady", "")["budget_usd"], 50.0);
}

fn spend(usd: f64) -> String {
    format!(r#"{{"bosun":"spend","session":"s","usd":{usd},"at":"2026-10-22T03:00:00Z"}}"#)
}

/// Epic #10 with budget 100, a ready task #11 and a merged task #12 that spent `usd`.
fn feature(mode: &str, usd: f64) -> FakeGithub {
    let fake = FakeGithub::start();
    on_main(&fake, mode);
    fake.issue(1, "bosun registry", &["bosun:registry"], "");
    let join = format!(
        r#"{{"bosun":"join","session":"session_test","origin":"spawned","depth":1,"expires":"{LATER}"}}"#
    );
    fake.comment(1, &join);
    let epic = r#"Epic<!-- bosun:epic {"item":"M1","plan":"plans/x.md","budget_usd":100.0,"budget_source":"plan"} -->"#;
    fake.issue(10, "Epic", &["bosun:state/ready", "bosun:area/core"], epic);
    let t = |n: u64| {
        format!(
            r#"<!-- bosun:task {{"item":"M1","task":"T{n}","epic":10,"touch":["src/a{n}/**"]}} -->"#
        )
    };
    fake.content("plans/x.md", "main", &plan_text(""));
    fake.issue(11, "T1", &["bosun:state/ready"], &t(1));
    fake.issue(12, "T2", &["bosun:state/merged"], &t(2));
    fake.comment(12, &spend(usd));
    fake
}

#[test]
fn claim_warns_at_80_percent_in_steady() {
    let (dir, fake) = (cfg_dir(""), feature("steady", 80.0));
    let (code, v) = run_in(&fake, dir.path(), &["claim", "11"]);
    assert_eq!(code, 0, "{v}");
    let w = &v["data"]["budget_feature"];
    assert_eq!(w["epic"], 10, "{v}");
    assert_eq!(w["level"], "warn");
    assert_eq!(
        (w["spent_usd"].as_f64(), w["budget_usd"].as_f64()),
        (Some(80.0), Some(100.0))
    );
}

#[test]
fn claim_asks_the_owner_at_100_percent_in_booster() {
    let (dir, fake) = (cfg_dir(""), feature("booster", 100.0));
    let before = fake.comments_of(11).len();
    let (code, v) = run_in(&fake, dir.path(), &["claim", "11"]);
    assert_eq!(code, 5, "{v}");
    assert_eq!(v["data"]["check"], "budget_feature", "{v}");
    assert!(v["error"]["fix"].as_str().unwrap().contains("epic #10"));
    assert_eq!(fake.comments_of(11).len(), before, "nothing is written");
}

#[test]
fn claim_below_the_warning_and_in_gatekeeper_is_unchanged() {
    let (dir, fake) = (cfg_dir(""), feature("steady", 10.0));
    let (code, v) = run_in(&fake, dir.path(), &["claim", "11"]);
    assert_eq!(code, 0, "{v}");
    assert!(v["data"].get("budget_feature").is_none());
    let (dir, fake) = (cfg_dir(""), feature("", 200.0));
    let (code, v) = run_in(&fake, dir.path(), &["claim", "11"]);
    assert_eq!(code, 0, "{v}");
    assert!(v["data"].get("budget_feature").is_none());
}

#[test]
fn spawn_asks_the_owner_at_100_percent_and_warns_at_80() {
    let (dir, fake) = (cfg_dir(""), feature("booster", 100.0));
    let before = fake.comments_of(10).len();
    let (code, v) = run_in(&fake, dir.path(), &["spawn", "10"]);
    assert_eq!(code, 5, "{v}");
    assert_eq!(
        fake.comments_of(10).len(),
        before,
        "a refusal claims nothing"
    );
    let (dir, fake) = (cfg_dir(""), feature("steady", 85.0));
    let (code, v) = run_in(&fake, dir.path(), &["spawn", "10"]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["budget_feature"]["level"], "warn");
    let (dir, fake) = (cfg_dir(""), feature("", 200.0));
    let (code, v) = run_in(&fake, dir.path(), &["spawn", "10"]);
    assert_eq!(code, 0, "{v}");
    assert!(v["data"].get("budget_feature").is_none());
}

#[test]
fn the_default_branch_decides_not_the_working_tree() {
    // The working tree says gatekeeper, the default branch says booster.
    let (dir, fake) = (cfg_dir("gatekeeper"), feature("booster", 100.0));
    let (code, v) = run_in(&fake, dir.path(), &["claim", "11"]);
    assert_eq!(code, 5, "{v}");
    // And the other way round: a booster working tree does not matter in gatekeeper.
    let (dir, fake) = (cfg_dir("booster"), feature("", 100.0));
    assert_eq!(run_in(&fake, dir.path(), &["claim", "11"]).0, 0);
}

#[test]
fn the_warn_knob_only_warns_at_100_percent() {
    let (dir, fake) = (cfg_dir(""), feature("booster", 100.0));
    fake.content(
        ".bosun/bosun.toml",
        "main",
        "[mode]\nname = \"booster\"\nbudget_feature = \"warn\"\n",
    );
    let (code, v) = run_in(&fake, dir.path(), &["claim", "11"]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["budget_feature"]["level"], "over");
}

#[test]
fn a_mainfix_task_is_never_refused() {
    let (dir, fake) = (cfg_dir(""), feature("booster", 100.0));
    let mf = r#"<!-- bosun:task {"item":"M1","task":"T1","epic":10,"touch":["src/a1/**"],"kind":"mainfix"} -->"#;
    fake.issue(13, "Fix", &["bosun:state/ready", "bosun:kind/mainfix"], mf);
    let (code, v) = run_in(&fake, dir.path(), &["claim", "13"]);
    assert_eq!(code, 0, "{v}");
    assert!(v["data"].get("budget_feature").is_none());
}

#[test]
fn spend_on_the_epic_issue_counts_and_the_fix_names_what_works() {
    let (dir, fake) = (cfg_dir(""), feature("booster", 0.0));
    fake.comment(10, &spend(100.0));
    let (code, v) = run_in(&fake, dir.path(), &["claim", "11"]);
    assert_eq!(code, 5, "{v}");
    let fix = v["error"]["fix"].as_str().unwrap();
    assert!(
        fix.contains("`budget_usd`") && fix.contains("bosun:epic"),
        "{fix}"
    );
}

#[test]
fn a_wave_tick_guards_the_epic_of_the_task() {
    let (dir, fake) = (cfg_dir(""), feature("booster", 100.0));
    let (code, v) = run_in(&fake, dir.path(), &["wave", "tick", "10"]);
    assert_eq!(code, 0, "{v}");
    let stop = &v["data"]["stopped"];
    assert_eq!(stop["check"], "budget_feature", "{v}");
    assert_eq!(stop["issue"], 11);
    assert!(v["next"].as_array().unwrap().is_empty());
}
