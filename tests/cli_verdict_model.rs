//! A verdict record carries the reviewer model; `[approvals] review_models` at base limits which
//! models count for the verdict approval classes (operating model #8).

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

fn pr(models: Option<&str>) -> FakeGithub {
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
    if let Some(m) = models {
        fake.content(
            ".bosun/bosun.toml",
            BASE,
            &format!("[approvals]\nreview_models = {m}\n"),
        );
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

fn verdict(fake: &FakeGithub, session: &str, model: Option<&str>) {
    let m = model.map_or(String::new(), |m| format!(",\"model\":\"{m}\""));
    fake.comment_by(
        1,
        BOT,
        &format!(
            "Review verdict: x.\n\n{{\"bosun\":\"verdict\",\"session\":\"{session}\",\"head_sha\":\"{HEAD}\",\"approved\":true,\"confidence\":0.95{m}}}"
        ),
    );
}

fn check(fake: &FakeGithub) -> (i32, Value) {
    run(fake, "session_test", NOW, &["risk-check", "1"])
}

#[test]
fn unset_review_models_counts_any_model_or_none() {
    let fake = pr(None);
    verdict(&fake, "s1", Some("m-new"));
    verdict(&fake, "s2", None);
    assert_eq!(check(&fake).0, 0);
}

#[test]
fn only_enrolled_models_count_and_the_fix_says_why() {
    let fake = pr(Some(r#"["m-old"]"#));
    verdict(&fake, "s1", Some("m-old"));
    verdict(&fake, "s2", Some("m-new"));
    verdict(&fake, "s3", None);
    let (code, v) = check(&fake);
    assert_eq!(code, 5, "{v}");
    assert_eq!(v["data"]["verdicts"], serde_json::json!(["s1"]));
    let fix = v["error"]["fix"].as_str().unwrap();
    assert!(
        fix.contains("review_models") && fix.contains("m-old"),
        "{fix}"
    );
    verdict(&fake, "s4", Some("m-old"));
    assert_eq!(check(&fake).0, 0);
}

#[test]
fn verdict_comment_carries_the_model_tag_and_post_records_it() {
    let fake = pr(None);
    fake.pull_head_sha(1, HEAD);
    let json = format!(r#"{{"pr":1,"head_sha":"{HEAD}","verdict":"approve","confidence":0.95}}"#);
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("v.json");
    std::fs::write(&file, json).unwrap();
    let args = ["verdict", "comment", "--from", file.to_str().unwrap()];
    let out = bosun()
        .env("BOSUN_GITHUB_API", fake.url())
        .env("BOSUN_MODEL", "m-rev")
        .args(args)
        .output()
        .unwrap();
    assert_eq!(
        out.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&out.stdout)
    );
    let body = fake.comments_of(1).last().unwrap().body.clone();
    assert!(body.contains("<!-- bosun:model m-rev -->"), "{body}");
    let out = bosun()
        .env("BOSUN_GITHUB_API", fake.url())
        .env("BOSUN_MODEL", "m-rev")
        .args(["verdict", "post", "--from", file.to_str().unwrap()])
        .output()
        .unwrap();
    assert_eq!(
        out.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&out.stdout)
    );
    let body = fake.comments_of(1).last().unwrap().body.clone();
    assert!(body.contains(r#""model":"m-rev""#), "{body}");
}
