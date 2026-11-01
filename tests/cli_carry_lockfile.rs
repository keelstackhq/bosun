//! risk-check carries a verdict across a merge of main whose only conflict was `Cargo.lock`, when
//! every entry of the regenerated lockfile is an entry of the approved head's or main's (rule 6).
#![cfg_attr(rustfmt, rustfmt::skip)]

mod common;

use bosun::core::records::Record;
use common::fake::FakeGithub;
use common::run;
use serde_json::{Value, json};

const B: &str = "b000000000000000000000000000000000000000"; const H: &str = "a000000000000000000000000000000000000000"; const M: &str = "c000000000000000000000000000000000000000"; const H2: &str = "d000000000000000000000000000000000000000";
const D: &str = "e000000000000000000000000000000000000000";
const TB: &str = "f0b0000000000000000000000000000000000000"; const TM: &str = "f0c0000000000000000000000000000000000000"; const TH: &str = "f0a0000000000000000000000000000000000000"; const TH2: &str = "f0d0000000000000000000000000000000000000";
const LEASE: &str = r#"{"bosun":"lease","session":"session_a","origin":"spawned","expires":"2026-10-22T10:30:00Z","touch":["src/pr.rs"]}"#;

type Ent = (&'static str, &'static str, &'static str, &'static str);

fn tree(main: &'static str, pr: &'static str, lock: &'static str, dir: &'static str) -> Vec<Ent> {
    vec![("Cargo.lock", "100644", "blob", lock), ("src", "040000", "tree", dir),
        ("src/main.rs", "100644", "blob", main), ("src/pr.rs", "100644", "blob", pr)]
}

fn cmp(fake: &FakeGithub, a: &str, b: &str, status: &str, base: Option<&str>, files: &[&str]) {
    let key = format!("{a}...{b}");
    let f: Vec<Value> = files.iter().map(|f| json!({"filename": f, "status": "modified", "additions": 1, "deletions": 0})).collect();
    let mut s = fake.state.lock().unwrap(); s.compares.insert(key.clone(), f);
    s.compare_statuses.insert(key.clone(), status.into()); if let Some(x) = base { s.compare_bases.insert(key, x.into()); }
}

fn pkg(name: &str, sum: &str) -> String {
    format!("[[package]]\nname = \"{name}\"\nversion = \"1.0.0\"\nsource = \"registry+https://github.com/rust-lang/crates.io-index\"\nchecksum = \"{sum}\"\n")
}

fn lock(pkgs: &[String]) -> String { format!("# generated\nversion = 4\n\n{}", pkgs.join("\n")) }

/// PR 1 (task #7) whose head H2 merges main's M into H; both changed Cargo.lock; a verdict on H.
fn world(merged_lock: &str) -> FakeGithub {
    let fake = FakeGithub::start();
    fake.pull(1, "change", "feat", "Closes #7"); fake.pull_as_issue(1, "change");
    fake.pull_set(1, "head", json!({"sha": H2, "ref": "feat"})); fake.pull_set(1, "base", json!({"sha": D, "ref": "main"}));
    fake.issue(7, "the task", &[], "Do it.\n<!-- bosun:task {} -->"); fake.comment(7, LEASE);
    for (sha, t, parents) in [(H2, TH2, vec![H, M]), (H, TH, vec![B]), (M, TM, vec![B]), (B, TB, vec![])] { fake.git_commit(sha, t, &parents); }
    for (t, e) in [(TB, tree("m1", "p0", "l0", "d0")), (TM, tree("m2", "p0", "lm", "d1")), (TH, tree("m1", "p1", "lh", "d2")), (TH2, tree("m2", "p1", "l2", "d3"))] { fake.git_tree(t, &e, false); }
    for (a, b, st, base, files) in [(D, "refs/heads/main", "identical", None, vec![]), (M, D, "ahead", None, vec![]), (D, H, "diverged", Some(B), vec![]),
        (B, M, "ahead", None, vec!["src/main.rs", "Cargo.lock"]), (B, H, "ahead", None, vec!["src/pr.rs", "Cargo.lock"])] { cmp(&fake, a, b, st, base, &files); }
    fake.content("Cargo.lock", H, &lock(&[pkg("anyhow", "bb")]));
    fake.content("Cargo.lock", M, &lock(&[pkg("tokio", "cc")]));
    fake.content("Cargo.lock", H2, merged_lock);
    let r = Record::Verdict { session: "session_r".into(), head_sha: H.into(), approved: true, confidence: Some(0.95), major: Some(0) };
    fake.comment_by(1, "github-actions[bot]", &format!("Review verdict: x.\n\n{}", r.to_line()));
    fake
}

fn carried_from(fake: &FakeGithub) -> Value {
    let (code, v) = run(fake, "session_test", "2026-10-22T10:00:00Z", &["risk-check", "1"]);
    assert_eq!(code, 0, "{v}");
    v["data"]["verdict_carried_from"].clone()
}

#[test]
fn a_regenerated_lockfile_of_both_sides_carries_the_verdict() {
    let fake = world(&lock(&[pkg("anyhow", "bb"), pkg("tokio", "cc")]));
    assert_eq!(carried_from(&fake), H);
}

#[test]
fn a_stitched_or_new_lockfile_entry_means_no_carry() {
    // anyhow's name with tokio's checksum: each line is on one side, the entry is on neither.
    let fake = world(&lock(&[pkg("anyhow", "cc"), pkg("tokio", "cc")]));
    assert_eq!(carried_from(&fake), Value::Null);
    let fake = world(&lock(&[pkg("anyhow", "bb"), pkg("tokio", "cc"), pkg("evil", "dd")]));
    assert_eq!(carried_from(&fake), Value::Null);
}

#[test]
fn an_unreadable_lockfile_means_no_carry() {
    let fake = world(&lock(&[pkg("anyhow", "bb"), pkg("tokio", "cc")]));
    fake.fail_path_message("/contents/Cargo.lock", 500, "SECRET-GITHUB-TEXT");
    let (code, v) = run(&fake, "session_test", "2026-10-22T10:00:00Z", &["risk-check", "1"]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["verdict_carried_from"], Value::Null);
    assert!(!v.to_string().contains("SECRET"));
}
