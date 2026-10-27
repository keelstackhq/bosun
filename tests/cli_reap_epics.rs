//! `bosun reap` closes an epic whose tasks are all merged or closed (spec section 4).

mod common;

use common::fake::FakeGithub;
use common::run;
use serde_json::json;

const NOW: &str = "2026-10-27T10:00:00Z";

fn epic(fake: &FakeGithub, n: u64, state: &str) {
    let body = format!(r#"C{n}: epic <!-- bosun:epic {{"item":"C{n}"}} -->"#);
    let labels = ["bosun:source/plan", &format!("bosun:state/{state}")];
    fake.issue(n, &format!("C{n}"), &labels, &body);
}

/// A task of epic `of`; it stays open, so `merged` here is a label on an open issue.
fn task(fake: &FakeGithub, n: u64, of: u64, state: &str) {
    let body = format!(r#"<!-- bosun:task {{"item":"C","task":"T{n}","epic":{of}}} -->"#);
    let labels = ["bosun:source/plan", &format!("bosun:state/{state}")];
    fake.issue(n, "task", &labels, &body);
}

fn live_lease(fake: &FakeGithub, n: u64) {
    fake.comment(
        n,
        &common::lease_note("orch", "2026-10-27T11:00:00Z", "x/**"),
    );
}

fn reap(fake: &FakeGithub) -> serde_json::Value {
    let (code, v) = run(fake, "janitor", NOW, &["reap"]);
    assert_eq!(code, 0, "{v}");
    v
}

#[test]
fn reap_closes_only_epics_whose_tasks_are_all_done() {
    let fake = FakeGithub::start();
    epic(&fake, 1, "ready");
    task(&fake, 2, 1, "merged"); // open but labelled merged: done
    task(&fake, 3, 1, "merged");
    epic(&fake, 4, "ready");
    task(&fake, 5, 4, "merged");
    task(&fake, 6, 4, "ready");
    epic(&fake, 7, "ready"); // no tasks yet

    assert_eq!(reap(&fake)["data"]["epics_closed"], json!([1]));
    let l = fake.labels_of(1);
    assert!(l.contains(&"bosun:state/merged".into()) && !l.contains(&"bosun:state/ready".into()));
    let closed = fake.issue_state(1);
    assert_eq!(closed.state, "closed");
    assert_eq!(closed.state_reason.as_deref(), Some("completed"));
    assert_eq!(fake.issue_state(4).state, "open");
    assert_eq!(fake.issue_state(7).state, "open");
    assert_eq!(reap(&fake)["data"]["epics_closed"], json!([]));
}

#[test]
fn reap_closes_a_claimed_epic_without_a_lease_and_leaves_the_rest() {
    let fake = FakeGithub::start();
    epic(&fake, 1, "ready"); // a live lease of an orchestrator
    live_lease(&fake, 1);
    epic(&fake, 3, "claimed"); // an orchestrator epic whose lease ended: closed
    epic(&fake, 5, "building");
    epic(&fake, 7, "ready"); // every task dropped: nothing was completed
    for e in [1, 3, 5] {
        task(&fake, e + 1, e, "merged");
    }
    task(&fake, 8, 7, "planned");
    fake.close_issue_as(8, NOW, "not_planned");
    epic(&fake, 9, "ready");
    task(&fake, 10, 9, "merged");
    fake.issue_updated(10, "2026-10-27T09:55:00Z");
    epic(&fake, 11, "claimed");
    task(&fake, 12, 11, "merged");
    live_lease(&fake, 11);

    assert_eq!(reap(&fake)["data"]["epics_closed"], json!([3]));
    for e in [1, 5, 7, 9, 11] {
        assert_eq!(fake.issue_state(e).state, "open", "#{e}");
    }
    let (_, v) = run(&fake, "janitor", "2026-10-27T10:06:00Z", &["reap"]);
    assert_eq!(v["data"]["epics_closed"], json!([9]), "{v}");
}

#[test]
fn a_failure_closing_an_epic_keeps_the_rest_of_the_reap() {
    let fake = FakeGithub::start();
    epic(&fake, 1, "ready");
    task(&fake, 2, 1, "merged");
    let meta = r#"<!-- bosun:task {"item":"X","task":"T1"} -->"#;
    fake.issue(5, "stale", &["bosun:state/claimed"], meta);
    fake.comment(
        5,
        &common::lease_note("gone", "2026-10-27T09:00:00Z", "x/**"),
    );
    fake.fail_path("/issues/1", 500);

    let v = reap(&fake);
    assert_eq!(v["data"]["epics_closed"], json!([]));
    assert!(v["data"]["epics_error"].is_string(), "{v}");
    assert_eq!(v["data"]["reaped"][0]["issue"], 5, "{v}");
    assert!(fake.labels_of(5).contains(&"bosun:state/ready".to_string()));
}
