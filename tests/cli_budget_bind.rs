//! Modes PR 7b: trusted writer bindings close the body-edit misses of the feature budget.
mod common;

use common::fake::FakeGithub;
use serde_json::Value;
use std::path::Path;

const NOW: &str = "2026-10-22T10:00:00Z";
const LATER: &str = "2026-10-22T10:20:00Z";
const EPIC: &str = r#"Epic<!-- bosun:epic {"item":"M1","plan":"plans/x.md"} -->"#;

fn plan_text(budget: &str) -> String {
    format!(
        "# Plan: widget\n\nproject: widget\n\n## M1: One\n\narea: core\nbudget_usd: {budget}\n\nWords.\n\n\
### Tasks\n- T1: a\n  touch: src/a/**\n- T2: b\n  touch: src/b/**\n\n### Done when\n- UC-M1-1: works\n"
    )
}

fn cfg_dir() -> tempfile::TempDir {
    let d = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(d.path().join(".bosun")).unwrap();
    d
}

fn run_in(fake: &FakeGithub, dir: &Path, args: &[&str]) -> (i32, Value) {
    let mut a = vec!["-C", dir.to_str().unwrap()];
    a.extend_from_slice(args);
    common::run(fake, "session_test", NOW, &a)
}

fn mode(fake: &FakeGithub, name: &str) {
    let t = format!("[mode]\nname = \"{name}\"\n");
    fake.content(".bosun/bosun.toml", "main", &t);
}

fn bind(plan: &str, item: &str, rest: &str) -> String {
    format!(r#"{{"bosun":"bind","session":"s","plan":"{plan}","item":"{item}"{rest}}}"#)
}

fn task_body(epic: &str) -> String {
    let e = if epic.is_empty() {
        String::new()
    } else {
        format!(r#""epic":{epic},"#)
    };
    format!(r#"<!-- bosun:task {{"item":"M1",{e}"task":"T1","touch":["src/a/**"]}} -->"#)
}

/// Epic #10 (budget 100), ready task #11 and merged task #12 that spent `usd`. With `bound`
/// the writer's binds are on all three; the bodies of #11 and #12 have lost their `epic`.
fn feature(bound: bool, usd: f64) -> FakeGithub {
    let fake = FakeGithub::start();
    mode(&fake, "booster");
    fake.issue(1, "bosun registry", &["bosun:registry"], "");
    let join = format!(
        r#"{{"bosun":"join","session":"session_test","origin":"spawned","depth":1,"expires":"{LATER}"}}"#
    );
    fake.comment(1, &join);
    fake.content("plans/x.md", "main", &plan_text("100"));
    fake.content("plans/y.md", "main", &plan_text("9999"));
    fake.issue(10, "Epic", &["bosun:state/ready", "bosun:area/core"], EPIC);
    let body = |e: &str| if bound { task_body("") } else { task_body(e) };
    fake.issue(11, "T1", &["bosun:state/ready"], &body("10"));
    fake.issue(
        12,
        "T3",
        &["bosun:state/merged"],
        &body("10").replace("T1", "T3"),
    );
    fake.comment(
        12,
        &format!(r#"{{"bosun":"spend","session":"s","usd":{usd},"at":"2026-10-22T03:00:00Z"}}"#),
    );
    if bound {
        let t = |n: u64, t: &str| {
            let rest = format!(r#","task":"{t}","epic":10"#);
            fake.comment(n, &bind("plans/x.md", "M1", &rest));
        };
        t(11, "T1");
        t(12, "T3");
        fake.comment(10, &bind("plans/x.md", "M1", r#","tasks":[11,12]"#));
    }
    fake
}

#[test]
fn a_bound_task_without_its_body_epic_is_still_guarded_and_its_spend_counts() {
    let (dir, fake) = (cfg_dir(), feature(true, 100.0));
    let (code, v) = run_in(&fake, dir.path(), &["claim", "11"]);
    assert_eq!(code, 5, "{v}");
    assert_eq!(v["data"]["check"], "budget_feature");
    assert_eq!(v["data"]["spent_usd"], 100.0);
    // Unbound, the body epic still guards.
    let (dir, fake) = (cfg_dir(), feature(false, 100.0));
    let (code, v) = run_in(&fake, dir.path(), &["claim", "11"]);
    assert_eq!(code, 5, "{v}");
}

#[test]
fn a_repointed_marker_and_a_later_forged_bind_change_nothing() {
    let (dir, fake) = (cfg_dir(), feature(true, 100.0));
    let big = EPIC.replace("plans/x.md", "plans/y.md");
    fake.issue(10, "Epic", &["bosun:state/ready", "bosun:area/core"], &big);
    assert_eq!(run_in(&fake, dir.path(), &["claim", "11"]).0, 5);
    let (dir, fake) = (cfg_dir(), feature(true, 100.0));
    fake.comment(10, &bind("plans/y.md", "M1", r#","tasks":[]"#));
    assert_eq!(run_in(&fake, dir.path(), &["claim", "11"]).0, 5);
}

#[test]
fn a_bound_epic_has_no_mainfix_label_exemption_and_an_unbound_one_keeps_it() {
    let (dir, fake) = (cfg_dir(), feature(true, 100.0));
    fake.label(10, "bosun:kind/mainfix");
    let (code, v) = run_in(&fake, dir.path(), &["spawn", "10"]);
    assert_eq!(code, 5, "{v}");
    let (dir, fake) = (cfg_dir(), feature(false, 100.0));
    fake.label(10, "bosun:kind/mainfix");
    let (code, v) = run_in(&fake, dir.path(), &["spawn", "10"]);
    assert_eq!(code, 0, "{v}");
}

fn binds(fake: &FakeGithub) -> usize {
    (1..=12)
        .flat_map(|n| fake.comments_of(n))
        .filter(|c| c.body.contains(r#""bosun":"bind""#))
        .count()
}

fn import(fake: &FakeGithub, dir: &Path) {
    let file = dir.join("plan.md");
    std::fs::write(&file, plan_text("100")).unwrap();
    let (code, v) = run_in(fake, dir, &["plan", "import", file.to_str().unwrap()]);
    assert_eq!(code, 0, "{v}");
}

#[test]
fn import_writes_binds_only_when_the_knob_is_on_and_never_twice() {
    let (dir, fake) = (cfg_dir(), FakeGithub::start());
    import(&fake, dir.path());
    assert_eq!(binds(&fake), 0, "gatekeeper writes no bind");
    let (dir, fake) = (cfg_dir(), FakeGithub::start());
    mode(&fake, "steady");
    import(&fake, dir.path());
    // Two tasks and the epic.
    assert_eq!(binds(&fake), 3);
    import(&fake, dir.path());
    assert_eq!(binds(&fake), 3, "a second import adds nothing");
}
