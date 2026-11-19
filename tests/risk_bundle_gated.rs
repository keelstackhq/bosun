//! D-021: risk-check has no R-bundle; a bundle with a gated path is high by that path's rule.

mod common;

use common::fake::FakeGithub;
use common::run;
use serde_json::Value;

const NOW: &str = "2026-10-22T10:00:00Z";
const BASE: &str = "base0000";

fn check(body: &str, path: &str) -> (i32, Value) {
    let fake = FakeGithub::start();
    fake.pull(1, "change", "feature", body);
    fake.issue(7, "task", &[], "");
    fake.content(
        "governance/owners.toml",
        BASE,
        "[[owner]]\ngithub = \"ada\"\n",
    );
    fake.pull_file(1, path, "modified", 1, 0, Some("@@ -0,0 +1,1 @@\n+x"));
    run(&fake, "session_test", NOW, &["risk-check", "1"])
}

fn rules(v: &Value) -> Vec<String> {
    v["data"]["matches"]
        .as_array()
        .map(|a| {
            a.iter()
                .map(|m| m["rule"].as_str().unwrap().to_string())
                .collect()
        })
        .unwrap_or_default()
}

#[test]
fn a_bundle_changing_a_gated_file_is_judged_by_the_path_rule_alone() {
    // D-021: gated work may bundle; the gated path still rates high, no R-bundle on top.
    let (code, v) = check("Closes #7\nCloses #8", "hoist.toml");
    assert_eq!(code, 5, "{v}");
    let r = rules(&v);
    assert!(!r.is_empty() && !r.contains(&"R-bundle".to_string()), "{v}");
    assert_eq!(v["data"]["level"], "high", "{v}");
}

#[test]
fn two_closes_on_ordinary_code_is_normal() {
    let (code, v) = check("Closes #7\nCloses #8", "src/a.rs");
    assert_eq!(code, 0, "{v}");
    assert!(rules(&v).is_empty(), "{v}");
}
