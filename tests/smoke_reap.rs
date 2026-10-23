//! UC-M1-3 smoke test: a lease that is not renewed for 30 min is reaped and the task is ready
//! again. The clock jumps through `BOSUN_NOW`, as a janitor run 15 min apart would see it.

mod common;

use common::fake::FakeGithub;
use common::run;
use serde_json::json;

const TASK: &str = r#"Do the thing.

<!-- bosun:task {"item":"M1","task":"T5","touch":["src/x/**"],"kind":"feature"} -->"#;

fn ends(fake: &FakeGithub, n: u64) -> Vec<String> {
    fake.comments_of(n)
        .into_iter()
        .filter(|c| c.body.contains(r#""bosun":"lease-end""#))
        .map(|c| c.body)
        .collect()
}

#[test]
fn uc_m1_3_an_unrenewed_lease_is_reaped_and_the_task_is_ready_again() {
    let fake = FakeGithub::start();
    fake.issue(5, "task", &["bosun:state/ready"], TASK);
    run(&fake, "session_a", "2026-10-22T10:00:00Z", &["join"]);
    let (code, v) = run(&fake, "session_a", "2026-10-22T10:00:00Z", &["claim", "5"]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["lease"]["expires"], "2026-10-22T10:30:00Z");

    // A heartbeat at +20 min renews the lease to +50 min.
    let (code, v) = run(&fake, "session_a", "2026-10-22T10:20:00Z", &["heartbeat"]);
    assert_eq!((code, &v["data"]["leases"]), (0, &json!([5])), "{v}");

    // Janitor runs while the lease is live: nothing is reaped, another session cannot claim.
    for now in ["2026-10-22T10:31:00Z", "2026-10-22T10:49:59Z"] {
        let (code, v) = run(&fake, "janitor", now, &["reap"]);
        assert_eq!((code, &v["data"]["reaped"]), (0, &json!([])), "{now}: {v}");
    }
    assert_eq!(fake.labels_of(5), vec!["bosun:state/claimed"]);
    let (code, _) = run(&fake, "session_b", "2026-10-22T10:45:00Z", &["claim", "5"]);
    assert_eq!(code, 3, "a live lease blocks the claim");

    // The session stops renewing. At the expiry the lease is no longer live.
    let (code, v) = run(&fake, "janitor", "2026-10-22T10:50:00Z", &["reap"]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(
        v["data"]["reaped"],
        json!([{"issue": 5, "session": "session_a"}])
    );
    assert_eq!(fake.labels_of(5), vec!["bosun:state/ready"]);
    let e = ends(&fake, 5);
    assert_eq!(e.len(), 1);
    assert!(e[0].contains(r#""reason":"reaped""#), "{}", e[0]);

    // A second janitor run reaps nothing more.
    let (_, v) = run(&fake, "janitor", "2026-10-22T11:05:00Z", &["reap"]);
    assert_eq!(v["data"]["reaped"], json!([]));

    // The task is ready again: another session claims it.
    let (code, v) = run(&fake, "session_b", "2026-10-22T11:05:00Z", &["claim", "5"]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(fake.labels_of(5), vec!["bosun:state/claimed"]);

    // The old holder's heartbeat no longer extends anything.
    let (_, v) = run(&fake, "session_a", "2026-10-22T11:06:00Z", &["heartbeat"]);
    assert_eq!(v["data"]["leases"], json!([]));
}
