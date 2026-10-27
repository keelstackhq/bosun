//! risk-check carries an approval over a clean merge of main (design 2026-10-27 §3 C7); any doubt is no carry, exit 5.
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
const PUB: &str = "11qYAYKxCrfVS/7TyWQHOg7hcvPapiMlrwIaaPcHURo=";
/// The key `k1` over `bosun-approve v1 acme/widget#1 <H>`, and over the same for another head.
const SIG_H: &str = "6RhPzvTreegRnggi6cn//Uq6kvHNt3KlkZgBBGo5aognp21gZ3esNw7TgJRavnqckrH0tGZ9g7hgO+rIHm9eDw==";
const SIG_OTHER: &str = "dTnLJ8NM8jp0umTiTZRuGW0lwGdxPoLlu069r9NXkSyrLmKj3SBW1ULZrmXNsE/qzfn+5gY0INdSAqttzPxmBg==";
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
/// High-risk PR 1 (task #7, leased by session_a): H2 merges main's M into H; owner ada, key k1; nothing approved.
fn world() -> FakeGithub { world_of(".github/workflows/ci.yml") }
fn world_of(file: &str) -> FakeGithub {
    let fake = FakeGithub::start();
    fake.pull(1, "change", "feat", "Closes #7"); fake.pull_as_issue(1, "change");
    fake.pull_set(1, "head", json!({"sha": H2, "ref": "feat"})); fake.pull_set(1, "base", json!({"sha": D, "ref": "main"}));
    fake.issue(7, "the task", &[], "Do it.\n<!-- bosun:task {} -->"); fake.comment(7, LEASE);
    for (sha, t, parents) in [(H2, TH2, vec![H, M]), (H, TH, vec![B]), (M, TM, vec![B]), (B, TB, vec![])] { fake.git_commit(sha, t, &parents); }
    for (t, e) in [(TB, tree("m1", "p0", "d0")), (TM, tree("m2", "p0", "d1")), (TH, tree("m1", "p1", "d2")), (TH2, tree("m2", "p1", "d3"))] { fake.git_tree(t, &e, false); }
    for (a, b, st, base, file) in [(D, "refs/heads/main", "identical", None, ""), (M, D, "ahead", None, ""), (D, H, "diverged", Some(B), ""),
        (B, M, "ahead", None, "src/main.rs"), (B, H, "ahead", None, "src/pr.rs")] { cmp(&fake, a, b, st, base, file); }
    fake.content("governance/owners.toml", D, "[[owner]]\ngithub = \"ada\"\n");
    fake.content(".bosun/approvers.toml", "refs/heads/main", &format!("[[key]]\nid = \"k1\"\ned25519 = \"{PUB}\"\n"));
    // The PR's one changed file decides the class: ci.yml wants the owner, channel/** two verdicts.
    fake.pull_file(1, file, "modified", 1, 0, Some("@@ -0,0 +1,1 @@\n+x"));
    fake
}
fn two_hops() -> FakeGithub { hops(world()) }
fn hops(fake: FakeGithub) -> FakeGithub {
    for (sha, t, parents) in [(H2, TH2, vec![X1, M]), (X1, TH2, vec![H, M1]), (M1, TB, vec![B]), (M, TM, vec![M1])] { fake.git_commit(sha, t, &parents); }
    for to in [D, M] { cmp(&fake, M1, to, "ahead", None, ""); }
    fake
}
fn verdict(fake: &FakeGithub, sha: &str, session: &str, approved: bool) {
    let r = Record::Verdict { session: session.into(), head_sha: sha.into(), approved, confidence: Some(0.95), major: Some(0) };
    fake.comment_by(1, "github-actions[bot]", &format!("Review verdict: x.\n\n{}", r.to_line()));
}
fn two(fake: &FakeGithub, sha: &str) { verdict(fake, sha, "session_r", true); verdict(fake, sha, "session_s", true); }
fn signed(fake: &FakeGithub, sha: &str, sig: &str) {
    fake.comment_at(1, "claude", &format!("/bosun approve {sha}\nbosun-sig ed25519 k1 {sig}"), "2026-10-22T09:00:00Z");
    fake.last_comment(|c| c.via_app = Some("claude".into()));
}
fn go(fake: &FakeGithub) -> (i32, Value, String) {
    let (code, v) = run(fake, "session_test", "2026-10-22T10:00:00Z", &["risk-check", "1"]);
    let s = fake.statuses_of(H2).into_iter().rev().find(|s| s["context"] == "risk-check").unwrap();
    (code, v, format!("{}: {}", s["state"].as_str().unwrap(), s["description"].as_str().unwrap()))
}
fn carries(fake: &FakeGithub) -> bool {
    let (code, v, st) = go(fake);
    let from = &v["data"]["approval_carried_from"];
    if *from == Value::Null { assert!(code == 5 && st.starts_with("failure: high risk"), "{code} {st} {v}"); return false; }
    assert!(code == 0 && st.starts_with("success: high risk") && st.contains("(carried from a000000)") && *from == H, "{code} {st} {v}");
    true
}
#[test] fn an_owner_review_a_comment_or_a_signature_on_h_carries() {
    let fake = world(); fake.review(1, "ada", "APPROVED", H);
    let (code, v, st) = go(&fake);
    assert_eq!((code, st.as_str()), (0, "success: high risk (R-ci), approved by ada (carried from a000000)"), "{v}");
    assert_eq!((&v["data"]["approval_carried_from"], &v["data"]["approved_by"]), (&json!(H), &json!("ada")));
    let fake = world(); fake.comment_at(1, "ada", &format!("/bosun approve {H}"), "2026-10-22T09:00:00Z"); assert!(carries(&fake));
    let fake = two_hops(); signed(&fake, H, SIG_H); // two hops carry from the older head
    let (_, v, st) = go(&fake);
    assert!(st.contains("approved by key k1 (carried from a000000)") && v["data"]["approval_carried_from"] == H, "{st}");
}
#[test] fn two_verdicts_on_h_carry_in_the_two_verdicts_class() {
    let fake = verdicts_world(); two(&fake, H);
    let (code, v, st) = go(&fake);
    assert_eq!((code, st.as_str()), (0, "success: high risk (R-ci-review), approved by two verdicts: session_r, session_s (carried from a000000)"), "{v}");
    let fake = verdicts_world(); verdict(&fake, H, "session_r", true); assert!(!carries(&fake)); // one is not enough
}
/// How many compares and tree reads `carry_over::check` made.
fn checks(fake: &FakeGithub) -> usize { fake.requests().iter().filter(|r| r.path.contains("/compare/") || r.path.contains("/trees/")).count() }
#[test] fn an_approval_on_the_head_itself_needs_no_carry_and_no_approval_needs_no_check() {
    let fake = world(); fake.review(1, "ada", "APPROVED", H2);
    let (code, v, st) = go(&fake);
    assert_eq!((code, st.as_str(), &v["data"]["approval_carried_from"], checks(&fake)), (0, "success: high risk (R-ci), approved by ada", &Value::Null, 0));
    let fake = world(); fake.review(1, "bob", "APPROVED", H); // not an owner
    assert!(!carries(&fake) && checks(&fake) == 0);
}
#[test] fn a_changed_merge_or_a_foreign_base_means_no_carry() {
    let fake = world(); fake.review(1, "ada", "APPROVED", H); fake.git_tree(TH2, &tree("m1", "p1", "d3"), false); // take ours
    assert!(!carries(&fake));
    let fake = world(); fake.review(1, "ada", "APPROVED", H); fake.pull_set(1, "base", json!({"sha": D, "ref": "release"}));
    assert!(!carries(&fake));
}
#[test] fn an_owner_who_withdrew_anywhere_on_the_path_blocks_the_carry() {
    for (state, sha, blocks) in [("CHANGES_REQUESTED", H2, true), ("DISMISSED", H2, true), ("DISMISSED", H, true), ("CHANGES_REQUESTED", H, true),
        ("CHANGES_REQUESTED", M, false) /* off the first-parent path */, ("COMMENTED", H2, false)] {
        let fake = world(); fake.review(1, "ada", "APPROVED", H); fake.review(1, "ada", state, sha);
        assert_eq!(!carries(&fake), blocks, "{state} on {sha}");
    }
    let fake = two_hops(); fake.review(1, "ada", "APPROVED", H); fake.review(1, "ada", "CHANGES_REQUESTED", X1); // on an intermediate hop
    assert!(!carries(&fake));
    let fake = world(); fake.content("governance/owners.toml", D, "[[owner]]\ngithub = \"ada\"\n[[owner]]\ngithub = \"bob\"\n");
    fake.review(1, "ada", "APPROVED", H); fake.review(1, "bob", "CHANGES_REQUESTED", H);
    assert!(!carries(&fake));
}
#[test] fn a_signature_counts_only_for_exactly_h_and_never_with_a_trust_file_or_an_unseen_diff() {
    let fake = world(); signed(&fake, H, SIG_OTHER); assert!(!carries(&fake)); // signs another head
    let fake = world(); signed(&fake, H, SIG_H); fake.pull_file(1, ".bosun/approvers.toml", "modified", 1, 0, Some("@@ -1 +1 @@\n+x"));
    assert!(!carries(&fake)); // H2 changes a trust file: signed approvals do not count
    let fake = world(); signed(&fake, H, SIG_H); fake.pull_set(1, "changed_files", json!(5)); assert!(!carries(&fake)); // R-unseen
    let fake = world(); signed(&fake, H, SIG_H); fake.review(1, "ada", "CHANGES_REQUESTED", H2); assert!(!carries(&fake));
    let fake = world(); signed(&fake, H, SIG_H); assert!(carries(&fake));
}
fn verdicts_world() -> FakeGithub { world_of("channel/x.toml") }
#[test] fn a_later_verdict_record_supersedes_a_carried_pair_of_verdicts() {
    let fake = verdicts_world(); two(&fake, H); verdict(&fake, H2, "session_t", true); assert!(!carries(&fake)); // on H2 itself
    let fake = hops(verdicts_world()); two(&fake, H); verdict(&fake, X1, "session_t", false); assert!(!carries(&fake)); // a disapproval on a hop
    let fake = hops(verdicts_world()); verdict(&fake, X1, "session_t", true); two(&fake, H); assert!(!carries(&fake)); // a record on a hop, even an older one
    let fake = verdicts_world(); two(&fake, H); verdict(&fake, OFF, "session_t", false); assert!(!carries(&fake)); // later, off the path
    let fake = verdicts_world(); verdict(&fake, H, "session_r", true); verdict(&fake, OFF, "session_t", false); verdict(&fake, H, "session_s", true);
    assert!(!carries(&fake)); // after the older of the two counted records
    let fake = verdicts_world(); verdict(&fake, OFF, "session_t", false); two(&fake, H); assert!(carries(&fake)); // judged before: no objection
}
#[test] fn a_leased_session_or_a_tightened_rule_stops_the_carry() {
    let fake = verdicts_world(); two(&fake, H); fake.comment(7, &LEASE.replace("session_a", "session_s")); assert!(!carries(&fake));
    let fake = world(); two(&fake, H); assert!(!carries(&fake)); // the owner class wants the owner, two verdicts do not satisfy it
    let fake = verdicts_world(); fake.review(1, "ada", "APPROVED", H); assert!(carries(&fake)); // the owner satisfies any class
}
#[test] fn a_read_failure_inside_the_carry_means_no_carry_and_exit_5_without_leaks() {
    for r in [format!("/git/trees/{TH2}"), format!("/compare/{B}...{M}"), format!("/git/commits/{H}")] {
        let fake = world(); fake.review(1, "ada", "APPROVED", H); fake.fail_path_message(&r, 500, "SECRET-GITHUB-TEXT");
        let (code, v, st) = go(&fake);
        assert!(code == 5 && st.starts_with("failure") && v["data"]["approval_carried_from"] == Value::Null, "{r}: {code} {v}");
        assert!(!v.to_string().contains("SECRET") && !st.contains("SECRET"), "{r}");
    }
}
const DSIG_H: &str = "7eBGpBbT6Rte+wYI0WlDix0/r6VfQwXQ37+ml3p2mNKDrvnb3k6mjEaZOJHKM6bQ+Ex7yLNBc3UOUw8d3RG0CA=="; const DSIG_H2: &str = "v8nTMrF8x55j1N9vefhXen1154RY//XXdjrCIT1ynjYGSOYDTWaXCNeynmEwSC5GgcHkb8nswFRbYPbVi6TzDA==";
fn delegate_world(sha: &str, sig: &str) -> FakeGithub { // R-d is a delegate rule; key k1 is a delegate key; one clean verdict and its signature on `sha`
    let fake = world_of("deny.toml"); fake.content(".bosun/risk.toml", D, "[[rule]]\nid = \"R-d\"\nwhy = \"d\"\napproval = \"delegate\"\npaths = [\"deny.toml\"]\n");
    fake.content(".bosun/approvers.toml", "refs/heads/main", "[[key]]\nid = \"k1\"\ned25519 = \"PUAXw+hDiVqStwqnTRt+vJyYLM8uxJaMwM1V8Sr0Zgw=\"\nrole = \"delegate\"\n");
    verdict(&fake, sha, "session_r", true); signed(&fake, sha, sig); fake
}
#[test] fn the_delegate_class_does_not_carry_yet() {
    let (code, v, st) = go(&delegate_world(H2, DSIG_H2)); // the same approval on the head itself passes
    assert_eq!((code, st.as_str(), &v["data"]["approval_carried_from"]), (0, "success: high risk (R-d), approved by delegate k1", &Value::Null), "{v}");
    assert!(!carries(&delegate_world(H, DSIG_H)));
}
#[test] fn an_edited_counted_verdict_record_does_not_carry() {
    let fake = verdicts_world(); two(&fake, H); fake.last_comment(|c| c.updated_at = "2026-10-23T00:00:00Z".into()); assert!(!carries(&fake));
}
