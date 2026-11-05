//! A wildcard `--touch` inside a bundle is resolved against the default branch's tree.

mod common;

use common::fake::FakeGithub;
use common::run;

const NOW: &str = "2026-10-22T10:00:00Z";
const PLAN: &str = "# Plan: X\n\nproject: x\n\n## M1: One\n\narea: core\ndepends: seed\n\nWords.\n\n\
### Tasks\n- T1: a\n  touch: src/a.rs\n### Done when\n- UC-M1-1: works\n";

fn world(tree: Option<(&str, bool)>) -> FakeGithub {
    let fake = FakeGithub::start();
    let epic = r#"Epic<!-- bosun:epic {"item":"M1","plan":"plans/x.md","shape":"single"} -->"#;
    fake.issue(1, "bosun registry", &["bosun:registry"], "");
    fake.issue(10, "Epic", &["bosun:state/claimed"], epic);
    fake.content("plans/x.md", "main", PLAN);
    let body = r#"<!-- bosun:task {"item":"M1","task":"T1","epic":10,"touch":["src/a.rs"]} -->"#;
    fake.issue(11, "Task 11", &["bosun:state/ready"], body);
    match tree {
        Some((file, cut)) => fake.git_tree("main", &[(file, "100644", "blob", "s")], cut),
        None => fake.no_main_tree(),
    }
    fake
}

fn claim(fake: &FakeGithub) -> (i32, serde_json::Value) {
    let args = ["claim", "11", "--touch", "packages/**"];
    run(fake, "session_b", NOW, &args)
}

#[test]
fn a_subtree_touch_holding_a_gated_file_is_refused() {
    let (code, v) = claim(&world(Some(("packages/a/hoist.toml", false))));
    assert_eq!(code, 3, "{v}");
    let m = v["error"]["message"].as_str().unwrap();
    assert!(m.contains("--touch packages/** is a gated path"), "{v}");
}

#[test]
fn a_clean_subtree_touch_is_allowed() {
    let (code, v) = claim(&world(Some(("packages/a/lib.rs", false))));
    assert_eq!(code, 0, "{v}");
}

#[test]
fn an_unresolvable_subtree_touch_is_refused() {
    for tree in [None, Some(("packages/a/lib.rs", true))] {
        let (code, v) = claim(&world(tree));
        assert_eq!(code, 3, "{v}");
        assert_eq!(v["error"]["fix"], "narrow --touch to explicit paths", "{v}");
    }
}

#[test]
fn claim_judges_gated_as_wave_does_not_by_the_stored_flag() {
    // Imported before packages/a/hoist.toml existed: stored gated is false, but the default
    // branch's tree makes the task gated now. Wave runs it alone, so claim must not refuse it
    // as bundled into the active #12. (#11 of `world` is a plain bundled task.)
    let fake = world(Some(("packages/a/hoist.toml", false)));
    let body = r#"<!-- bosun:task {"item":"M1","task":"T1","epic":10,"touch":["packages/**"]} -->"#;
    fake.issue(13, "Task 13", &["bosun:state/ready"], body);
    let lead = r#"<!-- bosun:task {"item":"M1","task":"T2","epic":10,"touch":["src/b.rs"]} -->"#;
    fake.issue(12, "Task 12", &["bosun:state/claimed"], lead);
    let (code, v) = run(&fake, "session_b", NOW, &["claim", "13"]);
    assert_eq!(code, 0, "{v}");
    let (code, v) = run(&fake, "session_c", NOW, &["claim", "11"]);
    assert_eq!(code, 3, "the plain task is still bundled: {v}");
}
