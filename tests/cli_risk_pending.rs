//! risk-check posts `pending` on the head before it judges, so auto-merge cannot fire while a
//! run is in flight, and never leaves that `pending` behind: a run that cannot finish posts
//! `error`, and the next run overwrites either.

mod common;

use common::fake::FakeGithub;
use common::run;
use serde_json::Value;

const NOW: &str = "2026-10-22T10:00:00Z";
const HEAD: &str = "sha1";

fn pr() -> FakeGithub {
    let fake = FakeGithub::start();
    fake.pull(1, "change", "feature", "Closes #7");
    fake.pull_file(
        1,
        "src/a.rs",
        "modified",
        1,
        0,
        Some("@@ -0,0 +1,1 @@\n+let x = 1;"),
    );
    fake
}

/// `(state, description)` of every risk-check status on the head, oldest first.
fn risk_statuses(fake: &FakeGithub) -> Vec<(String, String)> {
    let text = |s: &Value, k: &str| s[k].as_str().unwrap_or_default().to_string();
    fake.statuses_of(HEAD)
        .iter()
        .filter(|s| s["context"] == "risk-check")
        .map(|s| (text(s, "state"), text(s, "description")))
        .collect()
}

#[test]
fn a_run_posts_pending_first_and_its_verdict_last() {
    let fake = pr();
    let (code, v) = run(&fake, "session_test", NOW, &["risk-check", "1"]);
    assert_eq!(code, 0, "{v}");
    let st = risk_statuses(&fake);
    assert_eq!(st.len(), 2, "{st:?}");
    assert_eq!(st[0].0, "pending");
    assert_eq!(st[1], ("success".into(), "normal risk".into()));
}

#[test]
fn a_run_that_cannot_finish_leaves_error_not_pending() {
    let fake = pr();
    fake.fail_path_message("/pulls/1/files", 500, "SECRET-GITHUB-TEXT");
    let (code, v) = run(&fake, "session_test", NOW, &["risk-check", "1"]);
    assert_eq!(code, 1, "{v}");
    let st = risk_statuses(&fake);
    let last = st.last().expect("a status");
    assert_eq!(last.0, "error", "{st:?}");
    assert!(
        !last.1.contains("SECRET") && last.1.contains("re-run"),
        "{}",
        last.1
    );
}

#[test]
fn the_next_run_overwrites_a_stale_pending() {
    let fake = pr();
    fake.status(
        HEAD,
        "risk-check",
        "pending",
        "risk-check is judging this head",
    );
    let (code, _) = run(&fake, "session_test", NOW, &["risk-check", "1"]);
    assert_eq!(code, 0);
    assert_eq!(risk_statuses(&fake).last().unwrap().0, "success");
}

#[test]
fn a_failed_pending_post_does_not_stop_the_run() {
    let fake = pr();
    // After the PR read, the next request (the pending post) fails once.
    fake.after_requests("/pulls/1", 0, |s| s.fail = Some((500, 1)));
    let (code, v) = run(&fake, "session_test", NOW, &["risk-check", "1"]);
    assert_eq!(code, 0, "{v}");
    let st = risk_statuses(&fake);
    assert_eq!((st.len(), st[0].0.as_str()), (1, "success"), "{st:?}");
}

#[test]
fn a_high_risk_run_waiting_for_the_owner_keeps_failure_not_error() {
    let fake = FakeGithub::start();
    fake.pull(1, "change", "feature", "Closes #7");
    fake.pull_file(
        1,
        "src/cli/risk_check.rs",
        "modified",
        1,
        0,
        Some("@@ -0,0 +1,1 @@\n+// x"),
    );
    let (code, v) = run(&fake, "session_test", NOW, &["risk-check", "1"]);
    assert_eq!(code, 5, "{v}");
    let st = risk_statuses(&fake);
    assert_eq!(st[0].0, "pending");
    assert_eq!(st.last().unwrap().0, "failure", "{st:?}");
}
