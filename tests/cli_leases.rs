mod common;

use common::fake::FakeGithub;
use common::{lease_note, run};
use serde_json::json;

const T0: &str = "2026-10-22T10:00:00Z";
const T31: &str = "2026-10-22T10:31:00Z";

fn task(touch: &str, kind: &str) -> String {
    format!(
        r#"Do the thing.

<!-- bosun:task {{"item":"M1","task":"T1","touch":[{touch}],"kind":"{kind}"}} -->"#
    )
}

fn ready(fake: &FakeGithub, n: u64, touch: &str) {
    fake.issue(
        n,
        "task",
        &["bosun:state/ready"],
        &task(&format!("\"{touch}\""), "feature"),
    );
}

fn lease_ends(fake: &FakeGithub, n: u64) -> Vec<String> {
    fake.comments_of(n)
        .into_iter()
        .filter(|c| c.body.contains(r#""bosun":"lease-end""#))
        .map(|c| c.body)
        .collect()
}

#[test]
fn claim_writes_a_lease_and_moves_to_claimed() {
    let fake = FakeGithub::start();
    ready(&fake, 10, "src/core/**");
    let (code, v) = run(&fake, "session_a", T0, &["claim", "10"]);
    assert_eq!(code, 0, "{v}");
    let comments = fake.comments_of(10);
    assert_eq!(comments.len(), 1);
    assert!(
        comments[0]
            .body
            .contains(r#""bosun":"lease","session":"session_a""#)
    );
    assert_eq!(v["data"]["issue"], 10);
    assert_eq!(v["data"]["comment_id"], comments[0].id);
    assert_eq!(v["data"]["lease"]["session"], "session_a");
    assert_eq!(v["data"]["lease"]["expires"], "2026-10-22T10:30:00Z");
    assert_eq!(v["data"]["lease"]["touch"], json!(["src/core/**"]));
    let labels = fake.labels_of(10);
    assert!(labels.contains(&"bosun:state/claimed".into()));
    assert!(!labels.contains(&"bosun:state/ready".into()));
}

#[test]
fn overlapping_touch_with_a_live_lease_is_exit_3() {
    let fake = FakeGithub::start();
    ready(&fake, 10, "src/core/**");
    fake.issue(11, "other", &["bosun:state/building"], "");
    fake.comment(
        11,
        &lease_note("session_z", "2026-10-22T10:20:00Z", "src/core/*.rs"),
    );
    let (code, v) = run(
        &fake,
        "session_a",
        T0,
        &["claim", "10", "--touch", "src/core/lease.rs"],
    );
    assert_eq!(code, 3, "{v}");
    assert_eq!(v["error"]["kind"], "conflict");
    let msg = v["error"]["message"].as_str().unwrap();
    assert!(msg.contains("#11") && msg.contains("session_z") && msg.contains("src/core/*.rs"));
    assert_eq!(v["data"]["conflicts"].as_array().unwrap().len(), 1);
    assert!(fake.comments_of(10).is_empty());
    assert!(fake.labels_of(10).contains(&"bosun:state/ready".into()));
}

#[test]
fn disjoint_touch_and_expired_leases_do_not_conflict() {
    let fake = FakeGithub::start();
    ready(&fake, 10, "src/core/**");
    fake.issue(11, "other", &["bosun:state/claimed"], "");
    fake.comment(
        11,
        &lease_note("session_z", "2026-10-22T10:20:00Z", "docs/**"),
    );
    fake.issue(12, "stale", &["bosun:state/claimed"], "");
    fake.comment(
        12,
        &lease_note("session_y", "2026-10-22T09:59:00Z", "src/core/**"),
    );
    let (code, v) = run(&fake, "session_a", T0, &["claim", "10"]);
    assert_eq!(code, 0, "{v}");
}

#[test]
fn reclaim_by_the_holder_is_idempotent() {
    let fake = FakeGithub::start();
    ready(&fake, 10, "src/**");
    let (_, first) = run(&fake, "session_a", T0, &["claim", "10"]);
    let (code, second) = run(&fake, "session_a", "2026-10-22T10:05:00Z", &["claim", "10"]);
    assert_eq!(code, 0, "{second}");
    assert_eq!(fake.comments_of(10).len(), 1);
    assert_eq!(first["data"]["comment_id"], second["data"]["comment_id"]);
    assert_eq!(first["data"]["lease"], second["data"]["lease"]);
    let (code, v) = run(&fake, "session_b", "2026-10-22T10:05:00Z", &["claim", "10"]);
    assert_eq!(code, 3, "{v}");
    assert!(
        v["error"]["message"]
            .as_str()
            .unwrap()
            .contains("#10 is claimed")
    );
}

#[test]
fn area_owned_by_another_live_session_is_exit_3() {
    let fake = FakeGithub::start();
    fake.issue(
        10,
        "task",
        &["bosun:state/ready", "bosun:area/lifecycle"],
        &task("\"src/a.rs\"", "feature"),
    );
    run(&fake, "session_z", T0, &["join", "--area", "lifecycle"]);
    let (code, v) = run(&fake, "session_a", T0, &["claim", "10"]);
    assert_eq!(code, 3, "{v}");
    assert!(
        v["error"]["message"]
            .as_str()
            .unwrap()
            .contains("session_z")
    );
    let (code, v) = run(&fake, "session_z", T0, &["claim", "10"]);
    assert_eq!(code, 0, "the owner may claim: {v}");
}

#[test]
fn an_expired_area_owner_does_not_block() {
    let fake = FakeGithub::start();
    fake.issue(
        10,
        "task",
        &["bosun:state/ready", "bosun:area/lifecycle"],
        &task("\"src/a.rs\"", "feature"),
    );
    run(
        &fake,
        "session_z",
        "2026-10-22T08:00:00Z",
        &["join", "--area", "lifecycle"],
    );
    assert_eq!(run(&fake, "session_a", T0, &["claim", "10"]).0, 0);
}

#[test]
fn halt_and_main_red_stop_claims_except_mainfix() {
    let fake = FakeGithub::start();
    ready(&fake, 10, "src/a.rs");
    fake.variable("BOSUN_HALT", "1");
    assert_eq!(run(&fake, "session_a", T0, &["claim", "10"]).0, 4);

    let fake = FakeGithub::start();
    ready(&fake, 10, "src/a.rs");
    fake.issue(
        11,
        "fix main",
        &["bosun:state/ready"],
        &task("\"src/b.rs\"", "mainfix"),
    );
    fake.issue(99, "main is red", &["bosun:main-red"], "");
    let (code, v) = run(&fake, "session_a", T0, &["claim", "10"]);
    assert_eq!(code, 4, "{v}");
    assert!(v["error"]["message"].as_str().unwrap().contains("#99"));
    assert_eq!(run(&fake, "session_a", T0, &["claim", "11"]).0, 0);
}

#[test]
fn the_variable_halts_on_1_and_404_or_403_is_no_signal() {
    let fake = FakeGithub::start();
    ready(&fake, 10, "src/a.rs");
    // The variable is not set: the fake answers 404.
    assert_eq!(run(&fake, "session_a", T0, &["claim", "10"]).0, 0);

    let fake = FakeGithub::start();
    ready(&fake, 10, "src/a.rs");
    fake.variable("BOSUN_HALT", "1");
    fake.fail_path("/actions/variables/BOSUN_HALT", 403);
    assert_eq!(
        run(&fake, "session_a", T0, &["claim", "10"]).0,
        0,
        "403 is no signal"
    );

    let fake = FakeGithub::start();
    ready(&fake, 10, "src/a.rs");
    fake.fail_path("/actions/variables/BOSUN_HALT", 500);
    let (code, v) = run(&fake, "session_a", T0, &["claim", "10"]);
    assert_eq!(code, 4, "{v}");
    let msg = v["error"]["message"].as_str().unwrap();
    assert!(msg.contains("cannot read the BOSUN_HALT variable"), "{msg}");
}

#[test]
fn an_open_halt_issue_by_an_owner_halts() {
    let fake = FakeGithub::start();
    ready(&fake, 10, "src/a.rs");
    fake.content(
        "governance/owners.toml",
        "main",
        "[[owner]]\ngithub = \"ada\"\n",
    );
    fake.issue(50, "stop", &["bosun:halt"], "");
    fake.issue_by(50, "mallory");
    assert_eq!(
        run(&fake, "session_a", T0, &["claim", "10"]).0,
        0,
        "a non-owner cannot halt"
    );
    fake.issue(51, "stop", &["bosun:halt"], "");
    fake.issue_by(51, "Ada");
    let (code, v) = run(&fake, "session_a", T0, &["claim", "10"]);
    assert_eq!(code, 4, "{v}");
    let msg = v["error"]["message"].as_str().unwrap();
    assert!(msg.contains("#51"), "{msg}");
    assert!(v["error"]["fix"].as_str().unwrap().contains("close"));
}

#[test]
fn a_halt_issue_with_no_owners_listed_halts() {
    let fake = FakeGithub::start();
    ready(&fake, 10, "src/a.rs");
    fake.issue(50, "stop", &["bosun:halt"], "");
    let (code, v) = run(&fake, "session_a", T0, &["claim", "10"]);
    assert_eq!(code, 4, "{v}");
    assert!(v["error"]["fix"].as_str().unwrap().contains("owners.toml"));
}

#[test]
fn unreadable_halt_issues_fail_closed() {
    let fake = FakeGithub::start();
    ready(&fake, 10, "src/a.rs");
    fake.fail_next(403, 1);
    let (code, v) = run(&fake, "session_a", T0, &["claim", "10"]);
    assert_eq!(code, 4, "{v}");
    let msg = v["error"]["message"].as_str().unwrap();
    assert!(msg.contains("bosun:halt"), "{msg}");
}

#[test]
fn claim_refuses_bad_input() {
    let fake = FakeGithub::start();
    let (code, _) = run(&fake, "session_a", T0, &["claim", "404", "--touch", "a"]);
    assert_eq!(code, 1);
    fake.issue(10, "task", &["bosun:state/ready"], "no metadata");
    let (code, v) = run(&fake, "session_a", T0, &["claim", "10"]);
    assert_eq!(code, 2, "{v}");
    let (code, _) = run(&fake, "session_a", T0, &["claim", "10", "--touch", "../x"]);
    assert_eq!(code, 2);
    fake.issue(11, "busy", &["bosun:state/building"], "");
    let (code, v) = run(&fake, "session_a", T0, &["claim", "11", "--touch", "a"]);
    assert_eq!(code, 3);
    assert!(
        v["error"]["message"]
            .as_str()
            .unwrap()
            .contains("#11 is building")
    );
}

#[test]
fn release_ends_the_lease_and_moves_the_state() {
    let fake = FakeGithub::start();
    ready(&fake, 10, "src/a.rs");
    ready(&fake, 11, "src/b.rs");
    run(&fake, "session_a", T0, &["claim", "10"]);
    run(&fake, "session_a", T0, &["claim", "11"]);
    let (code, v) = run(&fake, "session_a", T0, &["release", "10"]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(lease_ends(&fake, 10).len(), 1);
    assert!(lease_ends(&fake, 10)[0].contains(r#""reason":"released""#));
    assert!(fake.labels_of(10).contains(&"bosun:state/ready".into()));
    assert_eq!(fake.labels_of(10).len(), 1);
    let (code, _) = run(
        &fake,
        "session_a",
        T0,
        &["release", "11", "--to", "blocked"],
    );
    assert_eq!(code, 0);
    assert_eq!(fake.labels_of(11), vec!["bosun:state/blocked"]);
    // the lease is gone, so another session can take the paths and a second release fails
    let (code, v) = run(&fake, "session_a", T0, &["release", "10"]);
    assert_eq!(code, 3);
    assert!(
        v["error"]["message"]
            .as_str()
            .unwrap()
            .contains("no lease of session_a on #10")
    );
    assert_eq!(run(&fake, "session_b", T0, &["claim", "10"]).0, 0);
}

#[test]
fn release_is_not_halted() {
    let fake = FakeGithub::start();
    ready(&fake, 10, "src/a.rs");
    run(&fake, "session_a", T0, &["claim", "10"]);
    fake.variable("BOSUN_HALT", "1");
    assert_eq!(
        run(&fake, "session_a", T0, &["release", "10", "--to", "review"]).0,
        0
    );
    assert_eq!(run(&fake, "session_a", T0, &["reap"]).0, 0);
}

#[test]
fn reap_ends_expired_leases_and_members() {
    let fake = FakeGithub::start();
    ready(&fake, 10, "src/a.rs");
    fake.issue(12, "live", &["bosun:state/building"], "");
    fake.comment(
        12,
        &lease_note("session_c", "2026-10-22T11:00:00Z", "docs/**"),
    );
    fake.issue(13, "late", &["bosun:state/review"], "");
    fake.comment(
        13,
        &lease_note("session_d", "2026-10-22T10:10:00Z", "docs/x"),
    );
    run(&fake, "session_a", T0, &["join"]);
    run(&fake, "session_a", T0, &["claim", "10"]);
    run(&fake, "session_c", T31, &["join"]);

    let (code, v) = run(&fake, "session_b", T31, &["reap"]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(
        v["data"]["reaped"],
        json!([{"issue": 10, "session": "session_a"}, {"issue": 13, "session": "session_d"}])
    );
    assert_eq!(v["data"]["left"], json!(["session_a"]));
    assert!(lease_ends(&fake, 10)[0].contains(r#""reason":"reaped""#));
    assert_eq!(fake.labels_of(10), vec!["bosun:state/ready"]);
    assert_eq!(
        fake.labels_of(13),
        vec!["bosun:state/review"],
        "review stays"
    );
    assert!(lease_ends(&fake, 12).is_empty());
    let reg = fake
        .state
        .lock()
        .unwrap()
        .issues
        .iter()
        .find(|i| i.title == "bosun registry")
        .unwrap()
        .number;
    let leaves: Vec<_> = fake
        .comments_of(reg)
        .into_iter()
        .filter(|c| c.body.contains(r#""bosun":"leave""#) && c.body.contains("session_a"))
        .collect();
    assert_eq!(leaves.len(), 1);
    assert!(leaves[0].body.contains(r#""reason":"reaped""#));
    // nothing left to reap
    let (_, v) = run(&fake, "session_b", T31, &["reap"]);
    assert_eq!(v["data"]["reaped"], json!([]));
    assert_eq!(v["data"]["left"], json!([]));
}

/// A comment that GitHub created before the claimer's own, inserted by the fake mid-claim.
fn rival_lease(s: &mut common::fake::State, issue: u64, session: &str, touch: &str) {
    s.comments.push(common::fake::FakeComment {
        id: 50,
        issue,
        body: lease_note(session, "2026-10-22T10:30:00Z", touch),
        user: "octocat".into(),
        created_at: "2026-10-22T09:59:59Z".into(),
        updated_at: "2026-10-22T09:59:59Z".into(),
    });
}

#[test]
fn the_same_session_conflicts_across_issues() {
    let fake = FakeGithub::start();
    ready(&fake, 10, "src/core/**");
    fake.issue(11, "in flight", &["bosun:state/building"], "");
    fake.comment(
        11,
        &lease_note("session_a", "2026-10-22T10:20:00Z", "src/core/x.rs"),
    );
    let (code, v) = run(&fake, "session_a", T0, &["claim", "10"]);
    assert_eq!(code, 3, "{v}");
    assert!(v["error"]["message"].as_str().unwrap().contains("#11"));
    // A per-task session id of the same orchestrator conflicts too, and re-claims its own.
    let (code, _) = run(&fake, "session_a/y", T0, &["claim", "10"]);
    assert_eq!(code, 3);
    let (code, v) = run(
        &fake,
        "session_a/y",
        T0,
        &["claim", "10", "--touch", "docs/**"],
    );
    assert_eq!(code, 0, "{v}");
    let (code, v) = run(&fake, "session_a/y", T0, &["claim", "10"]);
    assert_eq!(code, 0, "idempotent re-claim: {v}");
}

#[test]
fn losing_a_claim_race_on_the_same_issue_ends_the_lease_and_exits_3() {
    let fake = FakeGithub::start();
    ready(&fake, 10, "src/core/**");
    // The winner's lease and state land right after the loser's POST.
    fake.after_requests("/issues/10/comments", 1, |s| {
        rival_lease(s, 10, "session_w", "src/core/**");
        s.issues[0].labels = vec!["bosun:state/claimed".into()];
    });
    let (code, v) = run(&fake, "session_a", T0, &["claim", "10"]);
    assert_eq!(code, 3, "{v}");
    assert!(
        v["error"]["message"]
            .as_str()
            .unwrap()
            .contains("session_w")
    );
    let ends: Vec<String> = lease_ends(&fake, 10);
    assert_eq!(ends.len(), 1);
    assert!(ends[0].contains(r#""session":"session_a""#) && ends[0].contains("lost-race"));
    assert_eq!(fake.labels_of(10), vec!["bosun:state/claimed".to_string()]);
}

#[test]
fn losing_a_claim_race_against_an_overlapping_task_exits_3() {
    let fake = FakeGithub::start();
    ready(&fake, 10, "src/core/**");
    fake.issue(11, "other", &["bosun:state/claimed"], "");
    fake.after_requests("/issues/11/comments", 1, |s| {
        rival_lease(s, 11, "session_w", "src/core/lease.rs");
    });
    let (code, v) = run(&fake, "session_a", T0, &["claim", "10"]);
    assert_eq!(code, 3, "{v}");
    assert!(v["error"]["message"].as_str().unwrap().contains("#11"));
    assert_eq!(lease_ends(&fake, 10).len(), 1);
    assert!(fake.labels_of(10).contains(&"bosun:state/ready".into()));
}
