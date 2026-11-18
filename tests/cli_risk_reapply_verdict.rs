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
fn a_refused_comment_is_ignored_and_never_an_error() {
    let fake = pr();
    verdict_comment(&fake, HEAD, "mallory", "session_m");
    fake.permission("mallory", "read");
    let v = check(&fake);
    assert_eq!(v["data"]["verdict_reapplied"], Value::Null, "{v}");
    assert!(verdicts(&fake).is_empty());
    // An outsider's newer comment is ignored, as verdict.yml ignores it: the older one applies.
    let fake = pr();
    let id = verdict_comment(&fake, HEAD, "rev", "session_r");
    verdict_comment(&fake, HEAD, "mallory", "session_m");
    fake.permission("mallory", "read");
    assert_eq!(check(&fake)["data"]["verdict_reapplied"], json!(id));
    assert_eq!(verdicts(&fake).last().unwrap()["state"], "success");
}

fn body(verdict: &str, session: &str) -> String {
    let text = format!(r#"{{"pr":1,"head_sha":"{HEAD}","verdict":"{verdict}","confidence":0.9}}"#);
    comment_body(&Verdict::parse(&text).unwrap(), session, None)
}

fn withdrawn(st: &[Value]) -> bool {
    let last = st.last().unwrap();
    last["state"] == "failure"
        && last["description"]
            .as_str()
            .unwrap()
            .starts_with("re-apply withdrawn: ")
}

#[test]
fn an_erroring_newest_comment_withdraws_and_a_rerun_recovers() {
    let fake = pr();
    verdict_comment(&fake, HEAD, "rev", "session_q");
    fake.comment_by(1, "rob", &body("request-changes", "session_r"));
    // The newest apply errors before it posts any status.
    fake.fail_path("/collaborators/rob/permission", 500);
    let v = check(&fake);
    assert!(v["data"]["verdict_reapplied"].is_null(), "{v}");
    let st = verdicts(&fake);
    assert!(st.iter().all(|s| s["state"] != "success"), "{st:?}");
    assert!(withdrawn(&st), "{st:?}");
    // The error passes: a later run re-applies the newest.
    fake.state.lock().unwrap().fail_paths.clear();
    let rob = fake
        .comments_of(1)
        .iter()
        .find(|c| c.user == "rob")
        .unwrap()
        .id;
    assert_eq!(check(&fake)["data"]["verdict_reapplied"], json!(rob));
    let st = verdicts(&fake);
    let last = st.last().unwrap();
    assert_eq!(last["state"], "failure", "{st:?}");
    assert!(!withdrawn(&st), "{st:?}");
}

/// A comment by `user` lands right after `n` statuses were posted on the head.
fn push(fake: &FakeGithub, n: usize, user: &'static str, text: String) {
    fake.after_requests(&format!("/statuses/{HEAD}"), n, move |s| {
        let id = common::fake::add_comment(s, 1, &text);
        s.comments.iter_mut().find(|c| c.id == id).unwrap().user = user.into();
    });
}

fn id_of(fake: &FakeGithub, session: &str) -> u64 {
    let all = fake.comments_of(1);
    let c = all
        .iter()
        .find(|c| c.body.contains(session) && c.body.contains("bosun:verdict"));
    c.unwrap().id
}

#[test]
fn newer_verdicts_that_raced_the_applies_stand_instead() {
    let fake = pr();
    verdict_comment(&fake, HEAD, "rev", "session_q");
    // risk-check posts 1, q's apply 2, s's apply 3, t's apply 4.
    push(&fake, 2, "rev", body("request-changes", "session_s"));
    push(&fake, 3, "rev", body("approve", "session_t"));
    let v = check(&fake);
    assert_eq!(
        v["data"]["verdict_reapplied"],
        json!(id_of(&fake, "session_t")),
        "{v}"
    );
    let st = verdicts(&fake);
    assert_eq!(st.last().unwrap()["state"], "success", "{st:?}");
    assert_eq!(st.len(), 3, "{st:?}");
}

#[test]
fn an_outsiders_racing_comment_is_ignored() {
    let fake = pr();
    let q = verdict_comment(&fake, HEAD, "rev", "session_q");
    push(&fake, 2, "mallory", body("request-changes", "session_s"));
    fake.permission("mallory", "read");
    let v = check(&fake);
    assert_eq!(v["data"]["verdict_reapplied"], json!(q), "{v}");
    let st = verdicts(&fake);
    assert_eq!(st.last().unwrap()["state"], "success", "{st:?}");
}

#[test]
fn verdicts_racing_every_round_withdraw() {
    let fake = pr();
    verdict_comment(&fake, HEAD, "rev", "session_q");
    for (n, s) in [(2, "session_s"), (3, "session_t"), (4, "session_u")] {
        push(&fake, n, "rev", body("approve", s));
    }
    let v = check(&fake);
    assert!(v["data"]["verdict_reapplied"].is_null(), "{v}");
    assert!(withdrawn(&verdicts(&fake)));
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

#[test]
fn a_failed_write_after_the_status_never_lets_an_older_comment_replace_it() {
    let fake = pr();
    verdict_comment(&fake, HEAD, "rev", "session_q");
    let text =
        format!(r#"{{"pr":1,"head_sha":"{HEAD}","verdict":"request-changes","confidence":0.9}}"#);
    let v = Verdict::parse(&text).unwrap();
    fake.comment_by(1, "rev", &comment_body(&v, "session_r", None));
    // risk-check posts its status, the newest verdict posts its failure, then its record
    // comment fails, so its apply errors after the status stands.
    fake.after_requests(&format!("/statuses/{HEAD}"), 2, |s| {
        s.fail = Some((500, 1));
    });
    let v = check(&fake);
    assert!(v["data"]["verdict_reapplied"].is_null(), "{v}");
    let st = verdicts(&fake);
    assert!(st.iter().all(|s| s["state"] == "failure"), "{st:?}");
}
