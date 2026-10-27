mod common;

use common::fake::FakeGithub;
use common::{lease_note, run};
use serde_json::json;

const T0: &str = "2026-10-22T10:00:00Z";
const T5: &str = "2026-10-22T10:05:00Z";

fn ready(fake: &FakeGithub, n: u64, touch: &str) {
    fake.issue(
        n,
        "task",
        &["bosun:state/ready"],
        &format!(
            "Do it.\n\n<!-- bosun:task {{\"item\":\"M1\",\"task\":\"T1\",\"touch\":[\"{touch}\"],\"kind\":\"feature\"}} -->"
        ),
    );
}

fn leases(fake: &FakeGithub, n: u64) -> usize {
    fake.comments_of(n)
        .iter()
        .filter(|c| c.body.contains(r#""bosun":"lease","#))
        .count()
}

/// A rival lease on `src/b/x.rs` (issue 11), created earlier than the grow comment, that shows up
/// after the grow comment is posted (GET, GET, POST on issue 10), so only the race check sees it.
fn rival_after_the_grow_post(fake: &FakeGithub) {
    fake.after_requests("/issues/10/comments", 3, |s| {
        s.comments.push(common::fake::FakeComment {
            id: 50,
            issue: 11,
            body: lease_note("session_w", "2026-10-22T10:30:00Z", "src/b/x.rs"),
            user: "octocat".into(),
            created_at: "2026-10-22T09:59:59Z".into(),
            updated_at: "2026-10-22T09:59:59Z".into(),
            via_app: None,
            no_app_field: false,
            association: "MEMBER".into(),
        });
    });
}

#[test]
fn reclaim_with_new_touch_grows_the_lease() {
    let fake = FakeGithub::start();
    ready(&fake, 10, "src/a/**");
    let (code, _) = run(&fake, "session_a", T0, &["claim", "10"]);
    assert_eq!(code, 0);
    let (code, v) = run(
        &fake,
        "session_a",
        T5,
        &[
            "claim",
            "10",
            "--touch",
            "src/b/x.rs",
            "--touch",
            "src/a/**",
        ],
    );
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["added"], json!(["src/b/x.rs"]));
    assert_eq!(
        v["data"]["lease"]["touch"],
        json!(["src/a/**", "src/b/x.rs"])
    );
    assert_eq!(v["data"]["lease"]["expires"], "2026-10-22T10:35:00Z");
    assert_eq!(leases(&fake, 10), 2);
    assert!(fake.labels_of(10).contains(&"bosun:state/claimed".into()));
    // The grown lease is the live one.
    let (_, st) = run(&fake, "session_a", T5, &["status"]);
    assert!(st.to_string().contains("src/b/x.rs"), "{st}");
    // Another session is now refused on the added path.
    ready(&fake, 11, "src/b/**");
    let (code, v) = run(&fake, "session_b", T5, &["claim", "11"]);
    assert_eq!(code, 3, "{v}");
    assert_eq!(v["data"]["conflicts"][0]["issue"], 10);
}

#[test]
fn growing_into_a_path_another_session_holds_is_exit_3() {
    let fake = FakeGithub::start();
    ready(&fake, 10, "src/a/**");
    run(&fake, "session_a", T0, &["claim", "10"]);
    fake.issue(11, "other", &["bosun:state/building"], "");
    fake.comment(
        11,
        &lease_note("session_z", "2026-10-22T10:20:00Z", "src/b/*.rs"),
    );
    let (code, v) = run(
        &fake,
        "session_a",
        T5,
        &["claim", "10", "--touch", "src/b/x.rs"],
    );
    assert_eq!(code, 3, "{v}");
    assert_eq!(v["error"]["kind"], "conflict");
    assert!(!v["error"]["fix"].as_str().unwrap().is_empty());
    assert_eq!(v["data"]["conflicts"].as_array().unwrap().len(), 1);
    assert_eq!(leases(&fake, 10), 1);
}

#[test]
fn reclaim_with_present_globs_is_idempotent() {
    let fake = FakeGithub::start();
    ready(&fake, 10, "src/a/**");
    let (_, first) = run(&fake, "session_a", T0, &["claim", "10"]);
    let (code, v) = run(
        &fake,
        "session_a",
        T5,
        &["claim", "10", "--touch", "src/a/**"],
    );
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["added"], json!([]));
    assert_eq!(v["data"]["comment_id"], first["data"]["comment_id"]);
    assert_eq!(leases(&fake, 10), 1);
}

#[test]
fn plain_reclaim_is_unchanged() {
    let fake = FakeGithub::start();
    ready(&fake, 10, "src/a/**");
    let (_, first) = run(&fake, "session_a", T0, &["claim", "10"]);
    let (code, v) = run(&fake, "session_a", T5, &["claim", "10"]);
    assert_eq!(code, 0, "{v}");
    assert!(v["data"].get("added").is_none());
    assert_eq!(v["data"]["lease"], first["data"]["lease"]);
    assert_eq!(leases(&fake, 10), 1);
}

#[test]
fn losing_a_grow_race_keeps_the_previous_lease() {
    let fake = FakeGithub::start();
    fake.issue(10, "task", &["bosun:state/claimed"], "");
    let lease = r#"{"bosun":"lease","session":"session_a","origin":"local","expires":"2026-10-22T10:30:00Z","touch":["src/a/**"],"thread_ts":"1700.1"}"#;
    fake.comment(10, &format!("Claimed.\n\n{lease}"));
    fake.issue(11, "other", &["bosun:state/claimed"], "");
    // A rival lease on the added path, created earlier than the grow comment, shows up after the
    // grow comment is posted (GET, GET, POST on issue 10), so only the race check can see it.
    rival_after_the_grow_post(&fake);
    let (code, v) = run(
        &fake,
        "session_a",
        T5,
        &["claim", "10", "--touch", "src/b/x.rs"],
    );
    assert_eq!(code, 3, "{v}");
    let msg = v["error"]["message"].as_str().unwrap();
    assert!(msg.contains("lost the race for src/b/x.rs") && msg.contains("session_w"));
    assert!(!v["error"]["fix"].as_str().unwrap().is_empty());
    assert_eq!(v["data"]["restored"], true);
    assert!(fake.labels_of(10).contains(&"bosun:state/claimed".into()));
    // Nothing was ended; the grow comment carries the old lease again, thread included.
    let comments = fake.comments_of(10);
    assert_eq!(comments.len(), 2);
    assert!(comments.iter().all(|c| !c.body.contains("lease-end")));
    let last = &comments[1].body;
    assert!(
        last.contains(r#""expires":"2026-10-22T10:30:00Z""#),
        "{last}"
    );
    assert!(last.contains(r#""touch":["src/a/**"]"#) && last.contains("1700.1"));
    let (_, again) = run(&fake, "session_a", T5, &["claim", "10"]);
    assert_eq!(
        again["data"]["lease"]["touch"],
        json!(["src/a/**"]),
        "{again}"
    );
}

#[test]
fn a_failed_restore_after_a_lost_grow_race_is_exit_1() {
    let fake = FakeGithub::start();
    fake.issue(10, "task", &["bosun:state/claimed"], "");
    let first = fake.comment(
        10,
        &lease_note("session_a", "2026-10-22T10:30:00Z", "src/a/**"),
    );
    fake.issue(11, "other", &["bosun:state/claimed"], "");
    fake.fail_path(&format!("/issues/comments/{}", first + 1), 500);
    rival_after_the_grow_post(&fake);
    let (code, v) = run(
        &fake,
        "session_a",
        T5,
        &["claim", "10", "--touch", "src/b/x.rs"],
    );
    assert_eq!(code, 1, "{v}");
    let msg = v["error"]["message"].as_str().unwrap();
    assert!(msg.contains("restoring the old lease failed") && msg.contains("src/b/x.rs"));
    assert!(
        v["error"]["fix"]
            .as_str()
            .unwrap()
            .contains("bosun release 10")
    );
    assert_eq!(v["data"]["restored"], false);
}

#[test]
fn a_bad_glob_on_grow_is_a_usage_error() {
    let fake = FakeGithub::start();
    ready(&fake, 10, "src/a/**");
    run(&fake, "session_a", T0, &["claim", "10"]);
    let (code, v) = run(&fake, "session_a", T5, &["claim", "10", "--touch", "../x"]);
    assert_eq!(code, 2, "{v}");
    assert_eq!(leases(&fake, 10), 1);
}
