//! Carry-over fact gathering (design 2026-10-27 §3 C7) against the stateful fake: strictly by sha.
#![cfg_attr(rustfmt, rustfmt::skip)]

mod common;

use bosun::cli::carry_over::check;
use bosun::github::{LiveGithub, Pull};
use common::TOKEN_CANARY;
use common::fake::FakeGithub;
use serde_json::{Value, json};

const B: &str = "b000000000000000000000000000000000000000"; const H: &str = "a000000000000000000000000000000000000000"; const M: &str = "c000000000000000000000000000000000000000"; const H2: &str = "d000000000000000000000000000000000000000";
const D: &str = "e000000000000000000000000000000000000000"; const M1: &str = "c100000000000000000000000000000000000000"; const X1: &str = "d100000000000000000000000000000000000000";
const TB: &str = "f0b0000000000000000000000000000000000000"; const TM: &str = "f0c0000000000000000000000000000000000000"; const TH: &str = "f0a0000000000000000000000000000000000000";
const TH2: &str = "f0d0000000000000000000000000000000000000"; const TOTHER: &str = "f0e0000000000000000000000000000000000000";

type Ent = (&'static str, &'static str, &'static str, &'static str);

fn tree(main: &'static str, pr: &'static str, dir: &'static str) -> Vec<Ent> {
    vec![("src", "040000", "tree", dir), ("a", "100644", "blob", "a1"),
        ("src/main.rs", "100644", "blob", main), ("src/pr.rs", "100644", "blob", pr)]
}
fn file(name: &str, prev: Option<&str>) -> Value {
    json!({"filename": name, "status": "modified", "additions": 1, "deletions": 0, "previous_filename": prev})
}

struct W { fake: FakeGithub, gh: LiveGithub, pull: Pull }

impl W {
    fn cmp(&self, a: &str, b: &str, status: &str, base: Option<&str>, files: &[Value]) {
        let k = format!("{a}...{}", if b == "main" { "refs/heads/main" } else { b }); // the default branch by full ref
        let mut s = self.fake.state.lock().unwrap();
        s.compares.insert(k.clone(), files.to_vec());
        s.compare_statuses.insert(k.clone(), status.into());
        s.compare_bases.remove(&k);
        if let Some(x) = base { s.compare_bases.insert(k, x.into()); }
    }
    fn tree(&self, sha: &str, ents: &[Ent]) { self.fake.git_tree(sha, ents, false); }
    fn commit(&self, sha: &str, t: &str, parents: &[&str]) { self.fake.git_commit(sha, t, parents); }
    fn check(&self) -> Result<Vec<String>, String> { check(&self.gh, &self.pull, "main", H) }
    fn refused(&self) -> String { self.check().expect_err("must refuse") }
}

/// B -> H (changes src/pr.rs); B -> M (changes src/main.rs); H2 = merge(H, M); D = main's tip.
fn honest() -> W {
    let fake = FakeGithub::start();
    let gh = LiveGithub::with(&fake.url(), TOKEN_CANARY, "acme", "widget", 0);
    let pull = json!({"number": 1, "state": "open",
        "head": {"sha": H2, "ref": "feat"}, "base": {"sha": D, "ref": "main"}});
    let w = W { fake, gh, pull: serde_json::from_value(pull).unwrap() };
    for (c, t, p) in [(H2, TH2, &[H, M][..]), (H, TH, &[B]), (M, TM, &[B]), (B, TB, &[])] { w.commit(c, t, p); }
    for (t, m, p, d) in [(TB, "m1", "p0", "d0"), (TM, "m2", "p0", "d1"), (TH, "m1", "p1", "d2"), (TH2, "m2", "p1", "d3")] {
        w.tree(t, &tree(m, p, d));
    }
    w.cmp(D, "main", "identical", None, &[]);
    w.cmp(M, D, "ahead", None, &[]);
    w.cmp(D, H, "diverged", Some(B), &[]);
    w.cmp(B, M, "ahead", None, &[file("src/main.rs", None)]);
    w.cmp(B, H, "ahead", None, &[file("src/pr.rs", None)]);
    w
}

/// H -> X1 = merge(H, M1) -> H2 = merge(X1, M); M1 is an older main commit, M its child.
fn two_hops() -> W {
    let w = honest();
    w.commit(H2, TH2, &[X1, M]); w.commit(X1, TH2, &[H, M1]); w.commit(M1, TB, &[B]); w.commit(M, TM, &[M1]);
    w.cmp(M1, D, "ahead", None, &[]); w.cmp(M1, M, "ahead", None, &[]);
    w
}

#[test]
fn one_or_two_honest_hops_carry_with_their_path() {
    assert_eq!(honest().check().unwrap(), [H2]);
    let w = two_hops();
    assert_eq!(w.check().unwrap(), [H2, X1]);
    w.cmp(M1, M, "behind", None, &[]); // the newer merge brings an older main commit
    assert!(w.refused().contains("older default-branch"));
    let w = two_hops();
    w.cmp(M1, D, "diverged", None, &[]); // the older hop's main commit is off main
    assert!(w.refused().contains("not on the default branch"));
}

#[test]
fn take_ours_and_main_moving_a_p_path_are_refused() {
    let w = honest();
    w.cmp(B, H, "ahead", None, &[file("src/pr.rs", None), file("src/main.rs", None)]);
    w.tree(TH2, &tree("m1", "p1", "d3")); // H2 kept H's blob for a file main changed
    assert!(w.refused().contains("main also changed `src/main.rs`"));
    for main in [vec![file("src/z.rs", Some("src/pr.rs"))] /* renamed */, vec![file("src/pr.rs", None)] /* deleted */] {
        let w = honest();
        w.cmp(B, M, "ahead", None, &main);
        assert!(w.refused().contains("main also changed `src/pr.rs`"), "{main:?}");
    }
}

#[test]
fn an_old_main_commit_is_refused() {
    let w = honest();
    for status in ["behind", "diverged", ""] {
        w.cmp(B, M, status, None, &[file("src/main.rs", None)]);
        assert!(w.refused().contains("older default-branch"), "{status:?}");
    }
    w.cmp(B, M, "ahead", None, &[file("src/main.rs", None)]); w.cmp(M, D, "diverged", None, &[]); // M is off main
    assert!(w.refused().contains("not on the default branch"));
}

#[test]
fn changed_entries_in_h2_are_refused() {
    let edits = [("src/pr.rs", "100755", "blob") /* mode only */, ("src/pr.rs", "160000", "commit") /* gitlink */,
        ("a", "160000", "commit"), ("a", "120000", "blob"), ("evil.sh", "100644", "blob") /* extra file */];
    for (path, mode, kind) in edits {
        let w = honest();
        let mut t = tree("m2", "p1", "d3");
        t.retain(|e| e.0 != path);
        t.push((path, mode, kind, "x9"));
        w.tree(TH2, &t);
        assert!(w.refused().contains("differs from the expected merge"), "{path}");
    }
    let w = honest(); // a dir/file clash
    w.cmp(B, H, "ahead", None, &[file("d/x", None)]);
    let mut t = tree("m2", "p1", "d3"); t.push(("d", "100644", "blob", "f1"));
    w.tree(TB, &t); w.tree(TM, &t);
    assert!(w.refused().contains("file on one side, dir on the other"));
}

#[test]
fn non_merge_and_octopus_commits_are_refused() {
    let w = honest();
    w.commit(H2, TH2, &[X1]); // a plain commit on top of a merge
    w.commit(X1, TH2, &[H, M]);
    assert!(w.refused().contains("not a two-parent merge"));
    w.commit(H2, TH2, &[H, M, B]); // an octopus
    assert!(w.refused().contains("not a two-parent merge"));
    let w = honest(); assert!(check(&w.gh, &w.pull, "main", M).is_err()); // h not in the chain
}

#[test]
fn truncation_and_trust_files_are_refused() {
    let w = honest();
    let many: Vec<Value> = (0..300).map(|i| file(&format!("f{i}"), None)).collect();
    w.cmp(B, H, "ahead", None, &many);
    assert!(w.refused().contains("truncated"));
    let w = honest(); w.fake.git_tree(TM, &tree("m2", "p0", "d1"), true);
    assert!(w.refused().contains("truncated"));
    for name in ["CODEOWNERS", "governance/owners.toml"] {
        let w = honest();
        w.cmp(B, H, "ahead", None, &[file("src/pr.rs", None), file(name, None)]);
        assert!(w.refused().contains("trust file"), "{name}");
    }
}

#[test]
fn a_base_off_the_default_branch_or_an_approved_head_is_refused() {
    let mut w = honest();
    assert!(check(&w.gh, &w.pull, "main", H2).unwrap_err().contains("current head"));
    for status in ["diverged", "behind", ""] { // a stale or rewritten base sha
        w.cmp(D, "main", status, None, &[]);
        assert!(w.refused().contains("base commit is not on the default branch"), "{status:?}");
    }
    w.pull.base_ref = "release".into();
    assert!(w.refused().contains("not the default branch"));
}

#[test]
fn a_tree_that_is_not_the_commits_tree_is_refused() {
    let w = honest();
    w.fake.git_tree_raw(TH2, json!({"sha": TOTHER, "truncated": false, "tree": []}));
    assert!(w.refused().contains("cannot read tree"));
    w.tree(TH2, &tree("m2", "p1", "d3")); assert!(w.check().is_ok());
    let mut t = tree("m2", "p1", "d3"); t.push(("odd", "100644", "tag", "x1")); // an unknown entry kind
    w.tree(TH2, &t);
    assert!(w.refused().contains("cannot read tree"));
}

#[test]
fn a_commit_that_is_not_the_one_asked_for_is_refused() {
    for lie in [H, M, B, H2] {
        let w = honest();
        let mut s = w.fake.state.lock().unwrap();
        let c = s.git_commits[if lie == H { M } else { H }].clone(); s.git_commits.insert(lie.into(), c); drop(s);
        assert!(w.refused().contains("cannot read commit"), "{lie}");
    }
}

#[test]
fn github_errors_and_missing_objects_refuse_without_leaking() {
    let mut reads: Vec<String> = [H2, H, M, B].map(|s| format!("/git/commits/{s}")).to_vec();
    reads.extend([TB, TM, TH, TH2].map(|s| format!("/git/trees/{s}")));
    reads.extend([(D, "refs%2Fheads%2Fmain"), (M, D), (D, H), (B, M), (B, H)].map(|(a, b)| format!("/compare/{a}...{b}")));
    for r in &reads {
        let w = honest();
        w.fake.fail_path_message(r, 500, "SECRET-GITHUB-TEXT");
        let why = w.refused();
        assert!(!why.contains("SECRET") && why.len() < 100, "{r}: {why}");
    }
    let w = honest(); w.fake.state.lock().unwrap().git_commits.remove(H2);
    assert!(w.refused().contains("cannot read commit d000000"));
    let w = honest(); w.cmp(D, H, "diverged", None, &[]); // GitHub names no merge base
    assert!(w.refused().contains("merge base"));
}

#[test]
fn every_read_is_by_full_sha_and_a_short_sha_refuses() {
    let w = honest();
    assert!(w.check().is_ok());
    for r in w.fake.requests() {
        let p = r.path.split('?').next().unwrap();
        let ids: Vec<&str> = p.rsplit('/').next().unwrap().split("...").collect();
        let pinned = ids.iter().all(|i| i.len() == 40 && i.bytes().all(|b| b.is_ascii_hexdigit()));
        assert!(pinned || p.ends_with(&format!("/compare/{D}...refs%2Fheads%2Fmain")), "{p}");
    }
    let mut w = honest(); w.pull.head_sha = "d000000".into();
    assert!(w.refused().contains("not a full sha"));
    let w = honest(); w.commit(H2, TH2, &[H, "c000000"]); // GitHub names a parent by a short sha
    assert!(w.refused().contains("cannot read commit"));
}
