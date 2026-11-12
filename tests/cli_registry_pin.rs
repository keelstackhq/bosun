//! The registry is pinned: `.bosun/registry` on the default branch, else the lowest-numbered
//! labelled issue in any state. A newer labelled issue never replaces it.
mod common;

use common::fake::FakeGithub;
use common::run;

const T0: &str = "2026-10-22T10:00:00Z";
const LABEL: &str = "bosun:registry";

fn join_note(session: &str) -> String {
    format!(
        "{{\"bosun\":\"join\",\"session\":\"{session}\",\"origin\":\"spawned\",\"area\":\"gates\",\"depth\":1,\"expires\":\"2026-10-22T10:20:00Z\"}}"
    )
}

fn labelled(fake: &FakeGithub) -> usize {
    let s = fake.state.lock().unwrap();
    s.issues
        .iter()
        .filter(|i| i.labels.iter().any(|l| l == LABEL))
        .count()
}

#[test]
fn a_newer_open_labelled_issue_does_not_replace_the_registry() {
    let fake = FakeGithub::start();
    fake.issue(2, "rival", &[LABEL], "");
    fake.issue(1, "bosun registry", &[LABEL], "");
    fake.comment(1, &join_note("session_b"));
    let (code, v) = run(&fake, "session_a", T0, &["join"]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["registry_issue"], 1);
    assert_eq!(fake.comments_of(1).len(), 2, "joined on the old registry");
    assert_eq!(fake.comments_of(2).len(), 0);
}

#[test]
fn a_closed_lowest_registry_still_counts() {
    let fake = FakeGithub::start();
    fake.issue(1, "bosun registry", &[LABEL], "");
    fake.close_issue(1, "2026-10-21T10:00:00Z");
    let (code, v) = run(&fake, "session_a", T0, &["join"]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["registry_issue"], 1);
    assert_eq!(labelled(&fake), 1, "no new registry created");
}

#[test]
fn the_file_on_the_default_branch_wins_over_a_lower_issue() {
    let fake = FakeGithub::start();
    fake.issue(5, "pinned", &[LABEL], "");
    fake.issue(1, "old", &[LABEL], "");
    fake.content(".bosun/registry", "refs/heads/main", "5\n");
    let (code, v) = run(&fake, "session_a", T0, &["join"]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["registry_issue"], 5);
}

#[test]
fn the_file_naming_a_missing_issue_or_a_non_number_is_an_error() {
    let fake = FakeGithub::start();
    fake.issue(1, "bosun registry", &[LABEL], "");
    for n in ["9", "abc"] {
        fake.content(".bosun/registry", "refs/heads/main", n);
        let (code, v) = run(&fake, "session_a", T0, &["join"]);
        assert_eq!(code, 1, "{v}");
        let msg = v["error"]["message"].as_str().unwrap();
        assert!(msg.contains(".bosun/registry"), "{v}");
        assert!(!v["error"]["fix"].as_str().unwrap().is_empty());
    }
}

#[test]
fn no_issue_and_no_file_creates_one() {
    let fake = FakeGithub::start();
    let (code, v) = run(&fake, "session_a", T0, &["join"]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(labelled(&fake), 1);
}

#[test]
fn the_pin_is_the_trust_root_when_the_label_is_removed_from_it() {
    let fake = FakeGithub::start();
    fake.issue(2, "rival", &[LABEL], "");
    fake.issue(1, "bosun registry", &[], "");
    fake.content(".bosun/registry", "refs/heads/main", "1");
    let (code, v) = run(&fake, "session_a", T0, &["join"]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["registry_issue"], 1);
}

#[test]
fn doctor_warns_about_a_missing_pin() {
    let fake = FakeGithub::start();
    fake.issue(1, "bosun registry", &[LABEL], "");
    let (_, v) = run(&fake, "session_a", T0, &["doctor"]);
    let item = v["data"]["items"]
        .as_array()
        .or_else(|| v["data"]["steps"].as_array())
        .unwrap()
        .iter()
        .find(|i| i["name"] == "registry issue")
        .unwrap();
    assert_eq!(item["status"], "warn", "{v}");
    assert!(item["detail"].as_str().unwrap().contains(".bosun/registry"));
}

#[test]
fn unpinned_the_lowest_open_wins_over_a_lower_closed_one() {
    let fake = FakeGithub::start();
    fake.issue(1, "rotated", &[LABEL], "");
    fake.close_issue(1, "2026-10-21T10:00:00Z");
    fake.issue(2, "live", &[LABEL], "");
    fake.issue(3, "rival", &[LABEL], "");
    let (code, v) = run(&fake, "session_a", T0, &["join"]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["registry_issue"], 2);
}
