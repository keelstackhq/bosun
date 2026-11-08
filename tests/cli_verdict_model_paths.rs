//! The reviewer model on the remaining paths: `verdict apply` copies the comment's model tag,
//! `review_models` is read at the PR base only, and an unparsable base config fails closed.

mod common;

use common::fake::FakeGithub;
use common::{bosun, run};
use serde_json::Value;

const NOW: &str = "2026-10-22T10:00:00Z";
const BASE: &str = "base0000";
const HEAD: &str = "sha1";
const BOT: &str = "github-actions[bot]";
const RULES: &str = r#"
[[rule]]
id = "R-ci-review"
why = "r"
approval = "two-verdicts"
paths = [".github/workflows/**"]
"#;

fn pr(base_config: Option<&str>) -> FakeGithub {
    let fake = FakeGithub::start();
    fake.pull_as_issue(1, "change");
    fake.pull(1, "change", "feature", "Closes #7");
    fake.issue(7, "task", &[], "");
    fake.content(
        "governance/owners.toml",
        BASE,
        "[[owner]]\ngithub = \"ada\"\n",
    );
    fake.content(".bosun/risk.toml", BASE, RULES);
    if let Some(c) = base_config {
        fake.content(".bosun/bosun.toml", BASE, c);
    }
    fake.pull_file(
        1,
        ".github/workflows/other.yml",
        "modified",
        1,
        0,
        Some("@@ -0,0 +1,1 @@\n+x"),
    );
    fake
}

fn verdict(fake: &FakeGithub, session: &str, model: &str) {
    fake.comment_by(
        1,
        BOT,
        &format!(
            "Review verdict: x.\n\n{{\"bosun\":\"verdict\",\"session\":\"{session}\",\"head_sha\":\"{HEAD}\",\"approved\":true,\"confidence\":0.95,\"model\":\"{model}\"}}"
        ),
    );
}

fn check(fake: &FakeGithub) -> (i32, Value) {
    run(fake, "session_test", NOW, &["risk-check", "1"])
}

#[test]
fn verdict_apply_copies_the_comment_model_tag_into_the_record() {
    let fake = pr(None);
    fake.pull_head_sha(1, HEAD);
    let json = format!(r#"{{"pr":1,"head_sha":"{HEAD}","verdict":"approve","confidence":0.95}}"#);
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("v.json");
    std::fs::write(&file, json).unwrap();
    let out = bosun()
        .env("BOSUN_GITHUB_API", fake.url())
        .env("BOSUN_MODEL", "m-rev")
        .args(["verdict", "comment", "--from", file.to_str().unwrap()])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(0));
    let id = fake.comments_of(1)[0].id;
    fake.last_comment(|c| c.via_app = Some("claude".into()));
    // The applying session declares another model (none here): the comment's tag wins.
    let (code, v) = run(
        &fake,
        "session_ci",
        NOW,
        &["verdict", "apply", &id.to_string()],
    );
    assert_eq!(code, 0, "{v}");
    let rec = fake.comments_of(1).last().unwrap().body.clone();
    assert!(rec.contains(r#""model":"m-rev""#), "{rec}");
}

#[test]
fn review_models_set_only_at_the_head_is_ignored() {
    let fake = pr(None);
    let cfg = "[approvals]\nreview_models = [\"m-old\"]\n";
    fake.content(".bosun/bosun.toml", HEAD, cfg);
    fake.content(".bosun/bosun.toml", "feature", cfg);
    verdict(&fake, "s1", "m-new");
    verdict(&fake, "s2", "m-new");
    let (code, v) = check(&fake);
    assert_eq!(code, 0, "a PR cannot limit (or enroll) models itself: {v}");
}

#[test]
fn an_unparsable_base_config_counts_no_verdict() {
    let fake = pr(Some("[approvals\nreview_models = "));
    verdict(&fake, "s1", "m-old");
    verdict(&fake, "s2", "m-old");
    let (code, v) = check(&fake);
    assert_eq!(code, 5, "{v}");
    assert_eq!(v["data"]["verdicts"], serde_json::json!([]));
}
