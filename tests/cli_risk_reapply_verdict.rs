//! risk-check re-applies the newest valid verdict comment for exactly the head when it turns
//! green and the head has no `review-verdict` status (spec §8): never an error, fail closed.

mod common;

use bosun::core::verdict::{Verdict, comment_body};
use common::fake::FakeGithub;
use common::run;
use serde_json::{Value, json};

const NOW: &str = "2026-10-22T10:00:00Z";
const HEAD: &str = "sha1";

fn pr() -> FakeGithub {
    let fake = FakeGithub::start();
    fake.pull_as_issue(1, "change");
    fake.pull(1, "change", "feature", "Closes #7");
    fake.issue(7, "task", &[], "");
    fake.content(
        "governance/owners.toml",
        "base0000",
        "[[owner]]\ngithub = \"ada\"\n",
    );
    fake.pull_file(1, "src/a.rs", "modified", 1, 0, Some("@@ -0,0 +1,1 @@\n+x"));
    fake
}

fn verdict_comment(fake: &FakeGithub, head: &str, user: &str, session: &str) -> u64 {
    let text = format!(r#"{{"pr":1,"head_sha":"{head}","verdict":"approve","confidence":0.95}}"#);
    let v = Verdict::parse(&text).unwrap();
    fake.comment_by(1, user, &comment_body(&v, session, None));
    fake.comments_of(1).last().unwrap().id
}

fn verdicts(fake: &FakeGithub) -> Vec<Value> {
    let all = fake.statuses_of(HEAD).into_iter();
    all.filter(|s| s["context"] == "review-verdict").collect()
}

fn check(fake: &FakeGithub) -> Value {
    let (code, v) = run(fake, "session_test", NOW, &["risk-check", "1"]);
    assert_eq!(code, 0, "{v}");
    v
}

#[test]
fn a_green_head_gets_its_verdict_comment_applied() {
    let fake = pr();
    verdict_comment(&fake, HEAD, "rev", "session_q");
    let id = verdict_comment(&fake, HEAD, "rev", "session_r");
    let v = check(&fake);
    assert_eq!(v["data"]["verdict_reapplied"], json!(id), "{v}");
    let st = verdicts(&fake);
    assert_eq!(
        (st.len(), &st[0]["state"]),
        (1, &json!("success")),
        "{st:?}"
    );
}

#[test]
fn a_verdict_for_an_older_head_is_not_applied() {
    let fake = pr();
    verdict_comment(&fake, "sha0", "rev", "session_r");
    let v = check(&fake);
    assert_eq!(v["data"]["verdict_reapplied"], Value::Null, "{v}");
    assert!(verdicts(&fake).is_empty());
}

#[test]
fn an_existing_verdict_status_is_left_alone() {
    let fake = pr();
    verdict_comment(&fake, HEAD, "rev", "session_r");
    fake.status(HEAD, "review-verdict", "failure", "real");
    let v = check(&fake);
    assert_eq!(v["data"]["verdict_reapplied"], Value::Null, "{v}");
    let st = verdicts(&fake);
    assert_eq!((st.len(), &st[0]["description"]), (1, &json!("real")));
}

#[test]
fn a_refused_comment_is_skipped_and_never_an_error() {
    let fake = pr();
    verdict_comment(&fake, HEAD, "mallory", "session_m");
    fake.permission("mallory", "read");
    let v = check(&fake);
    assert_eq!(v["data"]["verdict_reapplied"], Value::Null, "{v}");
    assert!(verdicts(&fake).is_empty());
    // An older valid one still applies under a newer refused one.
    let fake = pr();
    let id = verdict_comment(&fake, HEAD, "rev", "session_r");
    verdict_comment(&fake, HEAD, "mallory", "session_m");
    fake.permission("mallory", "read");
    assert_eq!(check(&fake)["data"]["verdict_reapplied"], json!(id));
}

#[test]
fn a_dry_run_applies_nothing() {
    let fake = pr();
    verdict_comment(&fake, HEAD, "rev", "session_r");
    let (code, v) = run(
        &fake,
        "session_test",
        NOW,
        &["risk-check", "1", "--dry-run"],
    );
    assert_eq!(code, 0, "{v}");
    assert!(verdicts(&fake).is_empty());
}
