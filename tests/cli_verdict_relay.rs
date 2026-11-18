//! `bosun verdict comment --from <file>` with a verdict comment body relayed by another session
//! keeps the original reviewer and adds `relayed_by` (spec §7).

mod common;

use bosun::core::verdict::{Verdict, comment_body};
use common::fake::FakeGithub;
use common::run;
use serde_json::Value;

const T0: &str = "2026-11-18T10:00:00Z";
const JSON: &str = r#"{"pr":42,"head_sha":"sha42","verdict":"approve","confidence":0.95}"#;

fn setup() -> FakeGithub {
    let fake = FakeGithub::start();
    fake.issue(7, "the task", &["bosun:state/review"], "Do it.");
    fake.pull_as_issue(42, "the pr");
    fake.pull(42, "the pr", "claude/x", "Closes #7");
    fake.pull_file(42, "src/a.rs", "added", 1, 0, Some("@@ -0,0 +1 @@\n+x"));
    fake
}

fn comment(fake: &FakeGithub, session: &str, input: &str) -> (i32, Value) {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("v.txt");
    std::fs::write(&file, input).unwrap();
    run(
        fake,
        session,
        T0,
        &["verdict", "comment", "--from", file.to_str().unwrap()],
    )
}

fn reviewed_body() -> String {
    comment_body(&Verdict::parse(JSON).unwrap(), "session_rev", Some("m-rev"))
}

#[test]
fn a_relayed_body_keeps_the_reviewer_and_adds_relayed_by() {
    let fake = setup();
    let (code, v) = comment(&fake, "session_orch", &reviewed_body());
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["reviewer"], "session_rev");
    assert_eq!(v["data"]["relayed_by"], "session_orch");
    let body = fake.comments_of(42)[0].body.clone();
    assert!(
        body.contains("<!-- bosun:reviewer session_rev -->"),
        "{body}"
    );
    assert!(
        body.contains("<!-- bosun:relayed_by session_orch -->"),
        "{body}"
    );
    assert!(body.contains("<!-- bosun:model m-rev -->"), "{body}");
    assert!(
        !body.contains("session_orch -->\n<!-- bosun:reviewer"),
        "{body}"
    );

    // apply records the original reviewer and reports the relay.
    let id = fake.comments_of(42)[0].id;
    fake.last_comment(|c| c.via_app = Some("claude".into()));
    let (code, v) = run(
        &fake,
        "session_ci",
        T0,
        &["verdict", "apply", &id.to_string()],
    );
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["relayed_by"], "session_orch");
    let records: Vec<String> = fake
        .comments_of(42)
        .iter()
        .map(|c| c.body.clone())
        .collect();
    assert!(
        records
            .iter()
            .any(|b| b.contains(r#""session":"session_rev""#)),
        "{records:?}"
    );
}

#[test]
fn a_body_from_the_same_session_is_not_relayed() {
    let fake = setup();
    let (code, v) = comment(&fake, "session_rev", &reviewed_body());
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["reviewer"], "session_rev");
    assert!(v["data"].get("relayed_by").is_none(), "{v}");
    let body = fake.comments_of(42)[0].body.clone();
    assert!(!body.contains("relayed_by"), "{body}");

    let id = fake.comments_of(42)[0].id;
    fake.last_comment(|c| c.via_app = Some("claude".into()));
    let (code, v) = run(
        &fake,
        "session_ci",
        T0,
        &["verdict", "apply", &id.to_string()],
    );
    assert_eq!(code, 0, "{v}");
    assert!(v["data"].get("relayed_by").is_none(), "{v}");
}

#[test]
fn plain_json_is_posted_as_the_session_s_own_verdict() {
    let fake = setup();
    let (code, v) = comment(&fake, "session_orch", JSON);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["reviewer"], "session_orch");
    assert!(v["data"].get("relayed_by").is_none(), "{v}");
    let body = fake.comments_of(42)[0].body.clone();
    assert!(
        body.contains("<!-- bosun:reviewer session_orch -->"),
        "{body}"
    );
    assert!(!body.contains("relayed_by"), "{body}");
}
