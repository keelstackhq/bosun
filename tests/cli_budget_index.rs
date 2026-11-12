//! Modes PR 7d: the registry's epic index names the canonical epic of a plan item.
mod common;

use common::fake::FakeGithub;
use serde_json::Value;
use std::path::Path;

const NOW: &str = "2026-10-22T10:00:00Z";
const LATER: &str = "2026-10-22T10:20:00Z";
const EPIC: &str = r#"Epic<!-- bosun:epic {"item":"M1","plan":"plans/x.md"} -->"#;
const INDEX: &str =
    r#"{"bosun":"epic-index","session":"s","plan":"plans/x.md","item":"M1","epic":10}"#;
const FORGED: &str =
    r#"{"bosun":"epic-index","session":"s","plan":"plan.md","item":"M1","epic":99}"#;

fn plan_text() -> String {
    "# Plan: widget\n\nproject: widget\n\n## M1: One\n\narea: core\nbudget_usd: 100\n\nWords.\n\n\
### Tasks\n- T1: a\n  touch: src/a/**\n\n### Done when\n- UC-M1-1: works\n"
        .into()
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

/// A registry issue with this session joined, and the mode on main.
fn repo(mode: &str) -> FakeGithub {
    let fake = FakeGithub::start();
    fake.content(
        ".bosun/bosun.toml",
        "main",
        &format!("[mode]\nname = \"{mode}\"\n"),
    );
    fake.issue(1, "bosun registry", &["bosun:registry"], "");
    fake.comment(
        1,
        &format!(
            r#"{{"bosun":"join","session":"session_test","origin":"spawned","depth":1,"expires":"{LATER}"}}"#
        ),
    );
    fake
}

fn indexes(fake: &FakeGithub) -> usize {
    fake.comments_of(1)
        .iter()
        .filter(|c| c.body.contains(r#""bosun":"epic-index""#))
        .count()
}

fn plan_cmd(fake: &FakeGithub, dir: &Path, cmd: &str) -> (i32, Value) {
    let file = dir.join("plan.md");
    std::fs::write(&file, plan_text()).unwrap();
    run_in(fake, dir, &["plan", cmd, file.to_str().unwrap()])
}

#[test]
fn import_writes_one_index_per_item_never_twice_and_none_in_gatekeeper() {
    let (dir, fake) = (cfg_dir(), repo("gatekeeper"));
    assert_eq!(plan_cmd(&fake, dir.path(), "import").0, 0);
    assert_eq!(indexes(&fake), 0);
    let (dir, fake) = (cfg_dir(), repo("steady"));
    assert_eq!(plan_cmd(&fake, dir.path(), "import").0, 0);
    assert_eq!(indexes(&fake), 1);
    assert_eq!(plan_cmd(&fake, dir.path(), "amend").0, 0);
    assert_eq!(indexes(&fake), 1, "a second run adds nothing");
}

#[test]
fn a_later_forged_index_is_ignored_and_a_forged_first_one_stops_the_import() {
    let (dir, fake) = (cfg_dir(), repo("steady"));
    assert_eq!(plan_cmd(&fake, dir.path(), "import").0, 0);
    fake.comment(1, FORGED);
    assert_eq!(plan_cmd(&fake, dir.path(), "amend").0, 0);
    let (dir, fake) = (cfg_dir(), repo("steady"));
    fake.comment(1, FORGED);
    let (code, v) = plan_cmd(&fake, dir.path(), "import");
    assert_eq!(code, 5, "{v}");
    let fix = v["error"]["fix"].as_str().unwrap();
    assert!(fix.contains("index comment"), "{v}");
}

fn rival() -> (tempfile::TempDir, FakeGithub) {
    let (dir, fake) = (cfg_dir(), repo("booster"));
    fake.content("plans/x.md", "main", &plan_text());
    fake.comment(1, INDEX);
    fake.issue(10, "Epic", &["bosun:state/ready", "bosun:area/core"], EPIC);
    // #20 copies the marker; #10 may have lost its marker and area label.
    fake.issue(20, "Rival", &["bosun:state/ready"], EPIC);
    let t = |e: u64| {
        format!(
            r#"<!-- bosun:task {{"item":"M1","epic":{e},"task":"T1","touch":["src/a/**"]}} -->"#
        )
    };
    fake.issue(21, "T1", &["bosun:state/ready"], &t(20));
    fake.issue(11, "T1", &["bosun:state/ready"], &t(10));
    (dir, fake)
}

#[test]
fn a_rival_epic_cannot_hide_the_real_one_by_deleting_its_marker_and_label() {
    let (dir, fake) = rival();
    {
        let mut s = fake.state.lock().unwrap();
        let e = s.issues.iter_mut().find(|i| i.number == 10).unwrap();
        e.body = "Epic".into();
        e.labels.retain(|l| l != "bosun:area/core");
    }
    let (code, v) = run_in(&fake, dir.path(), &["claim", "21"]);
    assert_eq!(code, 5, "{v}");
    assert_eq!(v["data"]["check"], "budget_feature");
    assert!(
        v["error"]["fix"]
            .as_str()
            .unwrap()
            .contains("work under #10"),
        "{v}"
    );
}

#[test]
fn no_index_refuses_with_the_amend_fix_and_the_canonical_epic_claims() {
    let (dir, fake) = rival();
    {
        let mut s = fake.state.lock().unwrap();
        s.comments.retain(|c| !c.body.contains("epic-index"));
    }
    let (code, v) = run_in(&fake, dir.path(), &["claim", "11"]);
    assert_eq!(code, 5, "{v}");
    assert!(
        v["error"]["fix"]
            .as_str()
            .unwrap()
            .contains("bosun plan amend"),
        "{v}"
    );
    let (dir, fake) = rival();
    let (code, v) = run_in(&fake, dir.path(), &["claim", "11"]);
    assert_eq!(code, 0, "{v}");
}
