//! End to end: a PR that adds a line to `.bosun/template-tests` widens what test-integrity waives,
//! so `bosun risk-check` rates it R-weaken (spec §8).
mod common;

use common::fake::FakeGithub;
use common::run;

const NOW: &str = "2026-10-22T10:00:00Z";
const BASE: &str = "base0000";

fn pr() -> FakeGithub {
    let fake = FakeGithub::start();
    fake.pull(1, "change", "feature", "");
    fake.content(
        "governance/owners.toml",
        BASE,
        "[[owner]]\ngithub = \"ada\"\n",
    );
    fake.content(
        ".bosun/risk.toml",
        BASE,
        "[[rule]]\nid = \"R-weaken\"\nwhy = \"w\"\ncapability = \"weaken\"\n",
    );
    fake
}

#[test]
fn a_line_added_to_template_tests_is_a_weakening() {
    let fake = pr();
    let patch = "@@ -1,1 +1,2 @@\n tests/a/**\n+tests/b/**";
    fake.pull_file(1, ".bosun/template-tests", "modified", 1, 0, Some(patch));
    fake.content(".bosun/template-tests", BASE, "tests/a/**\n");
    fake.content(".bosun/template-tests", "sha1", "tests/a/**\ntests/b/**\n");
    let (code, v) = run(&fake, "session_test", NOW, &["risk-check", "1"]);
    assert_eq!(code, 5, "{v}");
    assert_eq!(v["data"]["level"], "high", "{v}");
    let m = &v["data"]["matches"][0];
    assert_eq!(m["rule"], "R-weaken", "{v}");
    assert!(m["evidence"].to_string().contains("template-tests"), "{v}");
}
