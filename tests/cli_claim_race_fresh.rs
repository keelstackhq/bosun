//! Two ready tasks claimed at the same time with overlapping touch: the race re-check lists the
//! active issues after our own state change, so whichever claim lists later sees the other.

mod common;

use common::fake::{FakeComment, FakeGithub};
use common::{lease_note, run};

const T0: &str = "2026-10-22T10:00:00Z";

fn ready(fake: &FakeGithub, n: u64) {
    fake.issue(
        n,
        "task",
        &["bosun:state/ready"],
        &format!(
            "Do it.\n\n<!-- bosun:task {{\"item\":\"M1\",\"task\":\"T{n}\",\"touch\":[\"src/a/**\"],\"kind\":\"feature\"}} -->"
        ),
    );
}

/// After our lease comment on #10, before our label request, the rival claims #11:
/// its lease comment (created at `created`) and its `claimed` label appear.
fn rival_claims_11(fake: &FakeGithub, created: &'static str) {
    fake.after_requests("/issues/10/labels", 0, move |s| {
        s.comments.push(FakeComment {
            id: 50,
            issue: 11,
            body: lease_note("session_w", "2026-10-22T10:30:00Z", "src/a/**"),
            user: "octocat".into(),
            created_at: created.into(),
            updated_at: created.into(),
            via_app: None,
            no_app_field: false,
            association: "MEMBER".into(),
        });
        let i = s.issues.iter_mut().find(|i| i.number == 11).unwrap();
        i.labels = vec!["bosun:state/claimed".into()];
    });
}

/// B's whole claim of #11 (claimed, lease, list, settle) runs against the fake right after A's
/// lease comment on #10, before A's next request: whatever A has done to #10 by then is all B sees.
fn rival_claims_11_in_full(fake: &FakeGithub, at: &str, created: &'static str) {
    fake.after_requests(at, 0, move |s| {
        s.hooks.push(common::fake::Hook::at_next_request(move |s| {
            let a_visible = s
                .issues
                .iter()
                .any(|i| i.number == 10 && i.labels.contains(&"bosun:state/claimed".to_string()))
                && s.comments
                    .iter()
                    .any(|c| c.issue == 10 && c.body.contains(r#""bosun":"lease""#));
            let mk = |id: u64, body: String| FakeComment {
                id,
                issue: 11,
                body,
                user: "octocat".into(),
                created_at: created.into(),
                updated_at: created.into(),
                via_app: None,
                no_app_field: false,
                association: "MEMBER".into(),
            };
            s.comments.push(mk(
                60,
                lease_note("session_w", "2026-10-22T10:30:00Z", "src/a/**"),
            ));
            let i = s.issues.iter_mut().find(|i| i.number == 11).unwrap();
            i.labels = vec!["bosun:state/claimed".into()];
            if a_visible {
                let end = r#"{"bosun":"lease-end","session":"session_w","reason":"lost-race"}"#;
                s.comments.push(mk(61, end.into()));
                let i = s.issues.iter_mut().find(|i| i.number == 11).unwrap();
                i.labels = vec!["bosun:state/ready".into()];
            }
        }));
    });
}

#[test]
fn of_two_claims_at_the_same_time_exactly_one_wins() {
    let fake = FakeGithub::start();
    ready(&fake, 10);
    ready(&fake, 11);
    rival_claims_11_in_full(&fake, "/issues/10/comments", "2030-01-01T00:00:00Z");
    let (code, v) = run(&fake, "session_a", T0, &["claim", "10"]);
    let b_won = fake.labels_of(11).contains(&"bosun:state/claimed".into());
    assert!((code == 0) != b_won, "a exit {code}, b won {b_won}: {v}");
}

#[test]
fn reap_resets_a_task_left_claimed_by_a_lost_race() {
    let fake = FakeGithub::start();
    fake.issue(10, "task", &["bosun:state/claimed"], "");
    fake.comment(
        10,
        &lease_note("session_a", "2026-10-22T10:30:00Z", "src/a/**"),
    );
    fake.comment(
        10,
        r#"{"bosun":"lease-end","session":"session_a","reason":"lost-race"}"#,
    );
    // Updated 30 s ago: a re-claim may be about to post its lease.
    fake.issue_updated(10, "2026-10-22T09:59:30Z");
    let (code, v) = run(&fake, "session_a", T0, &["reap"]);
    assert_eq!(code, 0, "{v}");
    assert!(fake.labels_of(10).contains(&"bosun:state/claimed".into()));
    fake.issue_updated(10, "2026-10-22T09:40:00Z");
    let (code, v) = run(&fake, "session_a", T0, &["reap"]);
    assert_eq!(code, 0, "{v}");
    assert!(fake.labels_of(10).contains(&"bosun:state/ready".into()));
}

#[test]
fn losing_to_an_earlier_claim_that_ran_between_our_state_change_and_post() {
    let fake = FakeGithub::start();
    ready(&fake, 10);
    ready(&fake, 11);
    rival_claims_11_in_full(&fake, "/issues/10/labels", "2000-01-01T00:00:00Z");
    let (code, v) = run(&fake, "session_a", T0, &["claim", "10"]);
    assert_eq!(code, 3, "{v}");
    assert!(fake.labels_of(11).contains(&"bosun:state/claimed".into()));
    assert!(fake.labels_of(10).contains(&"bosun:state/ready".into()));
}

#[test]
fn losing_to_a_task_claimed_at_the_same_time_reverts_to_ready() {
    let fake = FakeGithub::start();
    ready(&fake, 10);
    ready(&fake, 11);
    rival_claims_11(&fake, "2000-01-01T00:00:00Z");
    let (code, v) = run(&fake, "session_a", T0, &["claim", "10"]);
    assert_eq!(code, 3, "{v}");
    assert_eq!(v["data"]["winner"]["issue"], 11, "{v}");
    assert_eq!(v["data"]["winner"]["session"], "session_w");
    assert!(!v["error"]["fix"].as_str().unwrap().is_empty());
    let comments = fake.comments_of(10);
    assert!(
        comments
            .iter()
            .any(|c| c.body.contains("lease-end") && c.body.contains("lost-race")),
        "{comments:?}"
    );
    let labels = fake.labels_of(10);
    assert!(labels.contains(&"bosun:state/ready".into()), "{labels:?}");
    assert!(
        !labels.contains(&"bosun:state/claimed".into()),
        "{labels:?}"
    );
}

#[test]
fn a_third_claimer_that_won_in_the_meantime_gets_its_claimed_state_back() {
    let fake = FakeGithub::start();
    ready(&fake, 10);
    ready(&fake, 11);
    rival_claims_11(&fake, "2000-01-01T00:00:00Z");
    // Right after our `ready` label write, a third session leases #10 (it saw `claimed` earlier).
    fake.after_requests("/issues/10/labels", 1, |s| {
        s.hooks.push(common::fake::Hook::at_next_request(|s| {
            s.comments.push(FakeComment {
                id: 70,
                issue: 10,
                body: lease_note("session_x", "2026-10-22T10:30:00Z", "src/b/**"),
                user: "octocat".into(),
                created_at: "2030-01-01T00:00:00Z".into(),
                updated_at: "2030-01-01T00:00:00Z".into(),
                via_app: None,
                no_app_field: false,
                association: "MEMBER".into(),
            });
        }));
    });
    let (code, v) = run(&fake, "session_a", T0, &["claim", "10"]);
    assert_eq!(code, 3, "{v}");
    let labels = fake.labels_of(10);
    assert!(labels.contains(&"bosun:state/claimed".into()), "{labels:?}");
    assert!(!labels.contains(&"bosun:state/ready".into()), "{labels:?}");
}

#[test]
fn winning_against_a_later_claim_made_at_the_same_time() {
    let fake = FakeGithub::start();
    ready(&fake, 10);
    ready(&fake, 11);
    rival_claims_11(&fake, "2030-01-01T00:00:00Z");
    let (code, v) = run(&fake, "session_a", T0, &["claim", "10"]);
    assert_eq!(code, 0, "{v}");
    assert!(fake.labels_of(10).contains(&"bosun:state/claimed".into()));
    assert!(
        fake.comments_of(10)
            .iter()
            .all(|c| !c.body.contains("lease-end"))
    );
}
