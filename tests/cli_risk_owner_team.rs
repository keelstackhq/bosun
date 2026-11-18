//! A team entry in governance/owners.toml makes no approver until team expansion is built
//! (spec §16, fail closed); a team of another org is a parse error.
mod common;

use common::fake::FakeGithub;
use common::run;
use serde_json::{Value, json};

const HEAD: &str = "0123456789abcdef0123456789abcdef01234567";

fn high_pr(owners: &str) -> FakeGithub {
    let fake = FakeGithub::start();
    fake.pull(1, "change", "feature", "");
    fake.pull_as_issue(1, "change");
    fake.pull_set(1, "head", json!({"sha": HEAD, "ref": "feature"}));
    fake.content("governance/owners.toml", "base0000", owners);
    let diff = Some("@@ -0,0 +1,1 @@\n+x");
    fake.pull_file(1, ".github/workflows/ci.yml", "modified", 1, 0, diff);
    fake
}

fn check(fake: &FakeGithub) -> (i32, Value) {
    run(
        fake,
        "session_test",
        "2026-10-22T12:00:00Z",
        &["risk-check", "1"],
    )
}

const OWNERS: &str = "[[owner]]\ngithub = \"ada\"\n[[owner]]\nteam = \"acme/core\"\n";

#[test]
fn a_team_member_review_approves_nothing() {
    for who in ["core-member", "acme/core", "core"] {
        let fake = high_pr(OWNERS);
        fake.review(1, who, "APPROVED", HEAD);
        let (code, v) = check(&fake);
        assert_ne!(code, 0, "{who}: {v}");
    }
    let fake = high_pr(OWNERS);
    fake.review(1, "ada", "APPROVED", HEAD);
    let (code, v) = check(&fake);
    assert_eq!(code, 0, "{v}");
}

#[test]
fn a_team_of_another_org_is_a_parse_error() {
    let fake = high_pr(&OWNERS.replace("acme/core", "other/core"));
    fake.review(1, "ada", "APPROVED", HEAD);
    let (code, v) = check(&fake);
    assert_ne!(code, 0, "{v}");
    assert!(v.to_string().contains("other/core"), "{v}");
}
