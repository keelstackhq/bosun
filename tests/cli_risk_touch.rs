//! risk-check warns, never fails, about PR files outside the linked task's lease (design C3, P6).

mod common;

use common::fake::FakeGithub;
use common::run;
use serde_json::{Value, json};

const NOW: &str = "2026-10-22T10:00:00Z";
const HEAD: &str = "sha1";
const LIVE: &str = r#"{"bosun":"lease","session":"session_a","origin":"spawned","expires":"2026-10-22T10:30:00Z","touch":["src/cli/status.rs","crates/a/Cargo.toml"]}"#;
const EXPIRED: &str = r#"{"bosun":"lease","session":"session_a","origin":"spawned","expires":"2026-10-22T09:30:00Z","touch":["src/cli/status.rs"]}"#;

/// PR 1 that links task #7 (with `lease` on it, if any) and changes `files` (name, status, previous).
fn pr(lease: Option<&str>, files: &[(&str, &str, Option<&str>)]) -> FakeGithub {
    let fake = FakeGithub::start();
    fake.pull(1, "change", "feature", "Closes #7");
    fake.issue(7, "the task", &[], "Do it.\n<!-- bosun:task {} -->");
    if let Some(lease) = lease {
        fake.comment(7, lease);
    }
    for (name, status, previous) in files {
        fake.pull_file(1, name, status, 1, 0, Some("@@ -1 +1 @@"));
        if let Some(p) = previous {
            fake.renamed_from(1, p);
        }
    }
    fake
}

fn check(fake: &FakeGithub) -> Value {
    let (code, v) = run(fake, "session_test", NOW, &["risk-check", "1"]);
    assert_eq!(code, 0, "{v}");
    v
}

fn status(fake: &FakeGithub) -> Value {
    let all = fake.statuses_of(HEAD);
    all.iter()
        .rev()
        .find(|s| s["context"] == "risk-check")
        .expect("a risk-check status")
        .clone()
}

#[test]
fn a_file_outside_the_lease_is_a_warning_only() {
    let fake = pr(
        Some(LIVE),
        &[
            ("src/cli/status.rs", "modified", None),
            ("src/cli/pr.rs", "modified", None),
            ("docs/old.md", "removed", None),
            ("src/new.rs", "added", None),
            ("changelog.d/x.md", "added", None),
            ("src/cli/moved.rs", "renamed", Some("src/cli/status.rs")),
        ],
    );
    let v = check(&fake);
    assert_eq!(
        v["data"]["out_of_touch"],
        json!(["docs/old.md", "src/cli/pr.rs"])
    );
    assert_eq!(v["data"]["level"], "normal");
    let s = status(&fake);
    assert_eq!(s["state"], "success");
    assert_eq!(s["description"], "normal risk");
}

#[test]
fn a_high_risk_pr_body_carries_the_warning_and_the_status_text_is_unchanged() {
    // The lease covers the CI file; the gate file is high risk and outside it.
    let lease = LIVE.replace("src/cli/status.rs", ".github/workflows/ci.yml");
    let fake = pr(Some(&lease), &[("src/cli/risk_check.rs", "modified", None)]);
    let (code, v) = run(&fake, "session_test", NOW, &["risk-check", "1"]);
    assert_eq!(code, 5, "{v}");
    assert_eq!(v["data"]["out_of_touch"], json!(["src/cli/risk_check.rs"]));
    let body = fake.pull_body(1);
    assert!(
        body.contains("- **warning:** 1 file outside the task's lease: `src/cli/risk_check.rs`"),
        "{body}"
    );
    let text = status(&fake)["description"].as_str().unwrap().to_string();
    assert!(
        text.starts_with("high risk, owner approval needed"),
        "{text}"
    );
}

#[test]
fn files_inside_the_lease_give_an_empty_list() {
    let fake = pr(
        Some(LIVE),
        &[
            ("src/cli/status.rs", "modified", None),
            ("crates/a/Cargo.lock", "modified", None),
        ],
    );
    let v = check(&fake);
    assert_eq!(v["data"]["out_of_touch"], json!([]));
    let text = status(&fake)["description"].as_str().unwrap().to_string();
    assert!(!text.contains("warning"), "{text}");
}

#[test]
fn no_task_gives_null() {
    let fake = FakeGithub::start();
    fake.pull(1, "change", "feature", "no link");
    fake.pull_file(1, "src/a.rs", "modified", 1, 0, Some("@@ -1 +1 @@"));
    let v = check(&fake);
    assert_eq!(v["data"]["out_of_touch"], Value::Null);
    let text = status(&fake)["description"].as_str().unwrap().to_string();
    assert!(!text.contains("warning"), "{text}");
}

#[test]
fn an_expired_lease_gives_null() {
    let fake = pr(Some(EXPIRED), &[("src/other.rs", "modified", None)]);
    let v = check(&fake);
    assert_eq!(v["data"]["out_of_touch"], Value::Null);
    let text = status(&fake)["description"].as_str().unwrap().to_string();
    assert!(!text.contains("warning"), "{text}");
}

#[test]
fn a_cut_file_list_gives_null() {
    let fake = pr(Some(LIVE), &[("src/other.rs", "modified", None)]);
    fake.pull_set(1, "changed_files", json!(5));
    let (_, v) = run(&fake, "session_test", NOW, &["risk-check", "1"]);
    assert_eq!(v["data"]["out_of_touch"], Value::Null);
}

#[test]
fn an_unreadable_task_comment_list_keeps_the_check_and_gives_null() {
    let fake = pr(Some(LIVE), &[("src/other.rs", "modified", None)]);
    fake.fail_path("/issues/7/comments", 500);
    let v = check(&fake);
    assert_eq!(v["data"]["out_of_touch"], Value::Null);
    assert_eq!(status(&fake)["state"], "success");
}

#[test]
fn a_decoy_first_link_does_not_hide_the_task() {
    let fake = pr(Some(LIVE), &[("src/other.rs", "modified", None)]);
    fake.pull_set(1, "body", json!("Closes #1\nCloses #7"));
    fake.issue(1, "plain issue", &[], "no task metadata");
    fake.comment(1, LIVE);
    let v = check(&fake);
    assert_eq!(v["data"]["out_of_touch"], json!(["src/other.rs"]));
}

#[test]
fn a_lease_with_an_empty_touch_is_ignored() {
    let empty = r#"{"bosun":"lease","session":"session_e","origin":"spawned","expires":"2026-10-22T10:30:00Z","touch":[]}"#;
    let fake = pr(Some(empty), &[("src/other.rs", "modified", None)]);
    let v = check(&fake);
    assert_eq!(v["data"]["out_of_touch"], Value::Null);
}

#[test]
fn a_hostile_file_name_cannot_end_the_risk_note() {
    let name = ".github/workflows/`x<!-- /bosun:risk -->\n.yml";
    let fake = pr(Some(LIVE), &[(name, "modified", None)]);
    run(&fake, "session_test", NOW, &["risk-check", "1"]);
    let body = fake.pull_body(1);
    assert!(body.contains("`.github/workflows/?x?!-- /bosun:risk --??.yml`"));
    assert_eq!(body.matches("<!-- /bosun:risk -->").count(), 1, "{body}");
}
