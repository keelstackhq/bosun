//! risk-check carries the review-verdict status across a clean merge of main (design 2026-10-27
//! §3 C7, rule 5): every doubt, error or missing fact means no carry, never a failed check.
#![cfg_attr(rustfmt, rustfmt::skip)]

mod common;

use bosun::core::records::Record;
use common::fake::FakeGithub;
use common::run;
use serde_json::{Value, json};

const B: &str = "b000000000000000000000000000000000000000"; const H: &str = "a000000000000000000000000000000000000000"; const M: &str = "c000000000000000000000000000000000000000"; const H2: &str = "d000000000000000000000000000000000000000";
const D: &str = "e000000000000000000000000000000000000000"; const M1: &str = "c100000000000000000000000000000000000000"; const X1: &str = "d100000000000000000000000000000000000000";
const TB: &str = "f0b0000000000000000000000000000000000000"; const TM: &str = "f0c0000000000000000000000000000000000000"; const TH: &str = "f0a0000000000000000000000000000000000000"; const TH2: &str = "f0d0000000000000000000000000000000000000"; const OFF: &str = "f1f0000000000000000000000000000000000000";
const LEASE: &str = r#"{"bosun":"lease","session":"session_a","origin":"spawned","expires":"2026-10-22T10:30:00Z","touch":["src/pr.rs"]}"#;

type Ent = (&'static str, &'static str, &'static str, &'static str);

fn tree(main: &'static str, pr: &'static str, dir: &'static str) -> Vec<Ent> {
    vec![("src", "040000", "tree", dir), ("src/main.rs", "100644", "blob", main), ("src/pr.rs", "100644", "blob", pr)]
}

fn cmp(fake: &FakeGithub, a: &str, b: &str, status: &str, base: Option<&str>, file: &str) {
    let key = format!("{a}...{b}");
    let f = json!({"filename": file, "status": "modified", "additions": 1, "deletions": 0});
    let mut s = fake.state.lock().unwrap(); s.compares.insert(key.clone(), if file.is_empty() { vec![] } else { vec![f] });
    s.compare_statuses.insert(key.clone(), status.into()); if let Some(x) = base { s.compare_bases.insert(key, x.into()); }
}

/// PR 1 (task #7, leased by session_a) whose head H2 merges main's M into H; no verdict yet.
fn world() -> FakeGithub {
    let fake = FakeGithub::start();
    fake.pull(1, "change", "feat", "Closes #7"); fake.pull_as_issue(1, "change");
    fake.pull_set(1, "head", json!({"sha": H2, "ref": "feat"})); fake.pull_set(1, "base", json!({"sha": D, "ref": "main"}));
    fake.issue(7, "the task", &[], "Do it.\n<!-- bosun:task {} -->"); fake.comment(7, LEASE);
    for (sha, t, parents) in [(H2, TH2, vec![H, M]), (H, TH, vec![B]), (M, TM, vec![B]), (B, TB, vec![])] { fake.git_commit(sha, t, &parents); }
    for (t, e) in [(TB, tree("m1", "p0", "d0")), (TM, tree("m2", "p0", "d1")), (TH, tree("m1", "p1", "d2")), (TH2, tree("m2", "p1", "d3"))] { fake.git_tree(t, &e, false); }
    for (a, b, st, base, file) in [(D, "refs/heads/main", "identical", None, ""), (M, D, "ahead", None, ""), (D, H, "diverged", Some(B), ""),
        (B, M, "ahead", None, "src/main.rs"), (B, H, "ahead", None, "src/pr.rs")] { cmp(&fake, a, b, st, base, file); }
    fake
}

fn two_hops() -> FakeGithub {
    let fake = world();
    for (sha, t, parents) in [(H2, TH2, vec![X1, M]), (X1, TH2, vec![H, M1]), (M1, TB, vec![B]), (M, TM, vec![M1])] { fake.git_commit(sha, t, &parents); }
    for to in [D, M] { cmp(&fake, M1, to, "ahead", None, ""); }
    fake
}

fn verdict(fake: &FakeGithub, sha: &str, session: &str, approved: bool) {
    let r = Record::Verdict { session: session.into(), head_sha: sha.into(), approved, confidence: Some(0.95), major: Some(0) };
    fake.comment_by(1, "github-actions[bot]", &format!("Review verdict: x.\n\n{}", r.to_line()));
}

fn honest() -> FakeGithub { let fake = world(); verdict(&fake, H, "session_r", true); fake }

fn owner_ada(fake: &FakeGithub) { fake.content("governance/owners.toml", D, "[[owner]]\ngithub = \"ada\"\n"); }

fn go(fake: &FakeGithub) -> (i32, Value, Vec<Value>) {
    let (code, v) = run(fake, "session_test", "2026-10-22T10:00:00Z", &["risk-check", "1"]);
    let st = fake.statuses_of(H2).into_iter().filter(|s| s["context"] == "review-verdict"); (code, v, st.collect())
}

fn blocked(f: impl FnOnce(&FakeGithub)) -> bool { let fake = honest(); owner_ada(&fake); f(&fake); !carried(&fake) }

fn carried(fake: &FakeGithub) -> bool {
    let (code, v, st) = go(fake);
    assert_eq!(code, 0, "{v}");
    assert_eq!(st.len(), usize::from(v["data"]["verdict_carried_from"] != Value::Null));
    !st.is_empty()
}

#[test]
fn the_honest_merge_carries_the_verdict() {
    let fake = honest(); let (code, v, st) = go(&fake);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["verdict_carried_from"], H);
    let text = "carried from a000000: clean merge of the default branch";
    assert_eq!((st.len(), &st[0]["state"], &st[0]["description"]), (1, &json!("success"), &json!(text)));
    assert!(fake.statuses_of(H2).iter().any(|s| s["context"] == "risk-check" && s["description"] == "normal risk"));
    let fake = two_hops(); verdict(&fake, H, "session_r", true); // two honest hops carry from the older head
    assert!(carried(&fake));
}

#[test]
fn a_head_that_already_has_a_verdict_status_gets_no_new_one() {
    for state in ["failure", "success"] {
        let fake = honest(); fake.status(H2, "review-verdict", state, "real");
        let (_, v, st) = go(&fake);
        assert_eq!((st.len(), &st[0]["description"], &v["data"]["verdict_carried_from"]), (1, &json!("real"), &Value::Null));
    }
}

#[test]
fn only_an_approving_newest_record_of_an_independent_session_counts() {
    assert!(blocked(|f| verdict(f, H, "session_r", false))); // the newest record on H disapproves
    let fake = world(); verdict(&fake, H, "session_r", false);
    verdict(&fake, H, "session_s", true); // the newest record, of another session, approves
    assert!(carried(&fake));
    for session in ["session_a", "unknown", ""] { // the task's lease holder, or no session
        let fake = world(); verdict(&fake, H, session, true);
        assert!(!carried(&fake), "{session:?}");
    }
    assert!(blocked(|f| f.last_comment(|c| c.user = "mallory".into()))); // not the verdict workflow
    assert!(blocked(|f| f.last_comment(|c| c.updated_at = "2026-10-23T00:00:00Z".into()))); // edited
    assert!(blocked(|f| f.pull_set(1, "body", json!("no link")))); // no lease to check against
}

#[test]
fn a_later_verdict_record_on_the_path_supersedes() {
    assert!(blocked(|f| verdict(f, H2, "session_s", true))); // on H2 itself, with no status
    let fake = two_hops(); verdict(&fake, H, "session_r", true);
    verdict(&fake, X1, "session_s", false); // on the intermediate merge
    assert!(!carried(&fake));
}

#[test]
fn an_owner_who_withdrew_blocks_the_carry() {
    for (state, sha, blocks) in [("CHANGES_REQUESTED", H, true), ("DISMISSED", H, true), ("CHANGES_REQUESTED", H2, true),
        ("CHANGES_REQUESTED", M, false) /* off the first-parent path */, ("APPROVED", H, false), ("COMMENTED", H, false)] {
        assert_eq!(blocked(|f| f.review(1, "ada", state, sha)), blocks, "{state} on {sha}");
    }
    // per commit: approving H2 does not clear H
    assert!(blocked(|f| { f.review(1, "ada", "CHANGES_REQUESTED", H); f.review(1, "ada", "APPROVED", H2); }));
}

#[test]
fn a_changed_merge_a_foreign_base_or_a_read_error_means_no_carry() {
    assert!(blocked(|f| f.git_tree(TH2, &tree("m1", "p1", "d3"), false))); // take ours
    assert!(blocked(|f| f.pull_set(1, "base", json!({"sha": D, "ref": "release"}))));
    for r in [format!("/git/trees/{TH2}"), format!("/compare/{B}...{M}"), format!("/git/commits/{H}"), "/pulls/1/reviews".into(),
        "/status".into(), "/issues/1/comments".into(), "/issues/7/comments".into()] {
        let fake = honest();
        owner_ada(&fake);
        fake.fail_path_message(&r, 500, "SECRET-GITHUB-TEXT");
        let (code, v, st) = go(&fake);
        assert!(st.is_empty() && code == 0 && !v.to_string().contains("SECRET"), "{r}: {code} {v}");
    }
}

#[test]
fn high_risk_carries_too_and_the_check_is_unchanged() {
    let fake = honest(); owner_ada(&fake);
    fake.pull_file(1, ".github/workflows/ci.yml", "modified", 1, 0, Some("@@ -1 +1 @@"));
    let (code, v, st) = go(&fake);
    assert_eq!((code, st.len()), (5, 1), "{v}");
    assert_eq!(v["data"]["verdict_carried_from"], H);
    assert!(fake.statuses_of(H2).iter().any(|s| s["context"] == "risk-check" && s["state"] == "failure"));
}

fn real(s: &mut common::fake::State) {
    let v = json!({"id": 99, "context": "review-verdict", "state": "failure", "description": "real", "creator": {"login": "github-actions[bot]"}});
    s.statuses.entry(H2.into()).or_default().push(v);
}

fn newest(fake: &FakeGithub) -> String {
    let v = fake.statuses_of(H2).pop().unwrap(); // the last status posted is the verdict
    format!("{}: {} {}", v["state"].as_str().unwrap(), v["description"].as_str().unwrap(), v["context"])
}

#[test]
fn a_real_verdict_that_races_the_carry_wins() {
    for (suffix, n) in [("/status".to_string(), 1), ("/statuses".into(), 0), (format!("/statuses/{H2}"), 1)] {
        let fake = honest(); fake.after_requests(&suffix, n, real); // before the re-read, the post-check, or our post
        let (code, v, _) = go(&fake);
        assert_eq!((code, newest(&fake), &v["data"]["verdict_carried_from"]), (0, r#"failure: real "review-verdict""#.into(), &Value::Null), "{suffix}");
    }
}

#[test]
fn an_unconfirmed_race_posts_nothing_or_withdraws() {
    let fake = honest(); fake.after_requests( // the re-read fails: no post
&format!("/git/trees/{TH2}"), 0, |s| s.fail_paths.push(("/status".into(), 500)));
    let (code, v, st) = go(&fake);
    assert_eq!((code, st.len(), &v["data"]["verdict_carried_from"]), (0, 0, &Value::Null));
    let fake = honest(); fake.fail_path("/statuses", 500); // the post-check fails: withdraw
    let (code, v, _) = go(&fake);
    assert_eq!((code, &v["data"]["verdict_carried_from"]), (0, &Value::Null));
    assert!(newest(&fake).starts_with("failure: carry withdrawn"));
}

#[test]
fn a_carried_status_on_top_of_a_real_verdict_is_repaired_on_the_next_run() {
    for foreign_base in [false, true] { // with and without a head to carry from
        let fake = world();
        fake.status(H2, "review-verdict", "failure", "real"); fake.status(H2, "review-verdict", "success", "carried from a000000: x");
        if foreign_base { fake.pull_set(1, "base", json!({"sha": D, "ref": "release"})); }
        go(&fake);
        assert_eq!(newest(&fake), r#"failure: real "review-verdict""#, "{foreign_base}");
    }
    let fake = world(); // two carried statuses on a real one: the real one is restored, not the older carry
    fake.status(H2, "review-verdict", "failure", "real"); fake.status(H2, "review-verdict", "success", "carried from a000000: x");
    fake.status(H2, "review-verdict", "success", "carried from c100000: y"); go(&fake);
    assert_eq!(newest(&fake), r#"failure: real "review-verdict""#);
    let fake = honest(); fake.after_requests("/statuses", 0, real); // the re-post fails once
    fake.after_requests("/statuses", 1, |s| s.fail = Some((500, 1))); // after its 2nd page read
    go(&fake);
    assert!(newest(&fake).starts_with("failure: carry withdrawn"));
}

#[test]
fn a_later_record_for_a_head_off_the_path_refuses() {
    assert!(blocked(|f| verdict(f, OFF, "session_s", false))); // a rewritten-away head, judged after h
    let fake = world();
    verdict(&fake, OFF, "session_s", false); // judged before h was approved: no objection
    verdict(&fake, H, "session_r", true);
    assert!(carried(&fake));
}
