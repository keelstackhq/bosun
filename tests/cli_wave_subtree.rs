//! D-021: `wave plan` bundles a subtree touch whatever the default branch's tree holds: gated
//! work bundles like any task, so the tree is not read for gating.

mod common;

use common::fake::FakeGithub;
use common::run;
use serde_json::Value;

const NOW: &str = "2026-10-22T10:00:00Z";

type Entries<'a> = &'a [(&'a str, &'a str, &'a str, &'a str)];

fn world(tree: Option<(Entries, bool)>) -> FakeGithub {
    let fake = FakeGithub::start();
    fake.issue(
        10,
        "Epic",
        &["bosun:state/claimed"],
        "Epic\n<!-- bosun:epic {\"item\":\"M1\",\"shape\":\"single\"} -->",
    );
    for (n, id, touch) in [(11, "T1", "packages/**"), (12, "T2", "docs/x.md")] {
        let body = format!(
            r#"<!-- bosun:task {{"item":"M1","task":"{id}","epic":10,"depends":[],"touch":["{touch}"]}} -->"#
        );
        fake.issue(n, id, &["bosun:state/ready"], &body);
    }
    match tree {
        Some((e, cut)) => fake.git_tree("main", e, cut),
        None => fake.no_main_tree(),
    }
    fake
}

fn plan(fake: &FakeGithub) -> Value {
    let (code, v) = run(fake, "session_test", NOW, &["wave", "plan", "10"]);
    assert_eq!(code, 0, "{v}");
    v
}

fn lead_closes(v: &Value) -> (u64, Vec<u64>) {
    let first = &v["data"]["run"][0];
    let closes = first["closes"].as_array().cloned().unwrap_or_default();
    let closes = closes.iter().map(|n| n.as_u64().unwrap()).collect();
    (first["issue"].as_u64().unwrap(), closes)
}

#[test]
fn a_subtree_holding_a_gated_file_joins_the_bundle() {
    let fake = world(Some((
        &[("packages/a/hoist.toml", "100644", "blob", "s")],
        false,
    )));
    let v = plan(&fake);
    assert_eq!(lead_closes(&v), (11, vec![11, 12]), "{v}");
    assert_eq!(v["data"]["run"].as_array().unwrap().len(), 1, "{v}");
}

#[test]
fn an_unreadable_tree_changes_nothing() {
    for tree in [None, Some((&[][..], true))] {
        let fake = world(tree);
        let v = plan(&fake);
        assert_eq!(lead_closes(&v), (11, vec![11, 12]), "{v}");
        let w = v["data"]["warnings"].as_array().unwrap();
        assert!(
            !w.iter()
                .any(|w| w.as_str().unwrap().contains("cannot be checked")),
            "{v}"
        );
    }
}
