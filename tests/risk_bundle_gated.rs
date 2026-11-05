//! R-bundle: risk-check re-judges a PR that closes several tasks and changes a gated path (D-016).

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
fn two_closes_and_a_gated_file_is_r_bundle_owner() {
    let (code, v) = check("Closes #7\nCloses #8", "hoist.toml");
    assert_eq!(code, 5, "{v}");
    assert!(rules(&v).contains(&"R-bundle".to_string()), "{v}");
    assert_eq!(v["data"]["requirement"], "owner", "{v}");
}

#[test]
fn this_repo_qualified_and_url_forms_count() {
    let body = "Closes #7\nCloses acme/widget#8\nFixes https://github.com/acme/widget/issues/9";
    let (code, v) = check(body, "hoist.toml");
    assert_eq!(code, 5, "{v}");
    assert!(rules(&v).contains(&"R-bundle".to_string()), "{v}");
}

#[test]
fn the_same_issue_in_two_forms_counts_once() {
    let (_, v) = check("Closes #7\nCloses acme/widget#7", "hoist.toml");
    assert!(!rules(&v).contains(&"R-bundle".to_string()), "{v}");
}

#[test]
fn another_repos_issue_does_not_count() {
    let (_, v) = check("Closes #7\nFixes keelstackhq/splice#40", "hoist.toml");
    assert!(!rules(&v).contains(&"R-bundle".to_string()), "{v}");
}

#[test]
fn one_close_is_not_a_bundle() {
    let (_, v) = check("Closes #7", "hoist.toml");
    assert!(!rules(&v).contains(&"R-bundle".to_string()), "{v}");
}

#[test]
fn two_closes_on_ordinary_code_is_not_r_bundle() {
    let (code, v) = check("Closes #7\nCloses #8", "src/a.rs");
    assert_eq!(code, 0, "{v}");
    assert!(!rules(&v).contains(&"R-bundle".to_string()), "{v}");
}
