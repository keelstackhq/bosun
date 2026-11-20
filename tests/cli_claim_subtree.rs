//! D-021: a wildcard `--touch` inside a bundle is not checked for gated paths: gated work
//! bundles like any task, and risk-check asks the owner on the PR.

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
fn a_subtree_touch_is_allowed_whatever_the_tree_holds() {
    for tree in [
        Some(("packages/a/hoist.toml", false)),
        Some(("packages/a/lib.rs", false)),
        Some(("packages/a/lib.rs", true)),
        None,
    ] {
        let fake = world(tree);
        let (code, v) = claim(&fake);
        assert_eq!(code, 0, "{tree:?}: {v}");
        let trees = (fake.requests().iter())
            .filter(|r| r.path.contains("/git/trees/"))
            .count();
        assert_eq!(trees, 0, "{tree:?}: the tree is not read for gating");
    }
}
