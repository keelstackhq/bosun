//! Approval carry-over across a clean merge of main (design 2026-10-27 §3 C7, rules 1-4, and 6:
//! a lockfile both sides changed may differ if its entries all come from `H` or `M`). `H2` must be
//! provably the plain join of the default branch into the approved head `H`; the
//! expected tree is recomputed from `M` and `H`, never trusted from `H2`. Every doubt is a `Refusal`.
//! Rule 5 (withdrawn review, superseded verdict) is not decided here: P9b checks it after `Ok`.

use std::collections::{BTreeMap, BTreeSet};

use crate::core::lockmerge::{lockfile_kind, merged_lock_ok};

pub const MAX_HOPS: usize = 5;

/// A first-parent commit. `hops[0]` is `H2`; the last hop is the one whose first parent is `H`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hop {
    pub sha: String,
    pub parents: Vec<String>,
    pub second_parent_on_default: bool,
    /// Its second parent descends from the next-older hop's second parent. Ignored for the oldest
    /// hop (true by definition).
    pub descends_from_previous_second_parent: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileChange {
    pub filename: String,
    pub previous_filename: Option<String>,
    pub status: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Blob,
    Tree,
    Commit,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub mode: String,
    pub kind: Kind,
    pub sha: String,
}

pub type RawTree = Vec<(String, Entry)>;
type Tree = BTreeMap<String, Entry>;

#[derive(Debug, Clone)]
pub struct CarryFacts<'a> {
    pub base_is_default: bool,
    /// The approved head, the current head, `M` (second parent of `H2`) and `B` (merge-base of `H`
    /// and the default branch).
    pub h: String,
    pub h2: String,
    pub m: String,
    pub b: String,
    pub hops: Vec<Hop>,
    pub b_ancestor_of_m: bool,
    pub pr_files: Vec<FileChange>,
    pub main_files: Vec<FileChange>,
    pub tree_b: RawTree,
    pub tree_m: RawTree,
    pub tree_h: RawTree,
    pub tree_h2: RawTree,
    pub truncated: bool,
    pub trust_files: &'a [&'a str],
    /// The texts of each lockfile both the PR and main changed, at `H`, `M` and `H2`.
    pub lock_texts: BTreeMap<String, LockTexts>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Refusal {
    NotDefaultBase,
    TooManyHops(usize),
    NotMerge,
    OctopusOrNonMerge,
    BrokenChain,
    SecondParentOffDefault,
    MainWentBack,
    PathsOverlapMain { path: String },
    BaseMismatch { path: String },
    TreeMismatch { path: String },
    DirFileClash { path: String },
    DuplicatePath { path: String },
    Truncated,
    TrustFile { path: String },
    LockNotMerged { path: String },
}

fn plain_ascii(c: char) -> bool {
    c != '`' && (' '..='~').contains(&c)
}
/// A path safe to show: at most 80 printable ASCII characters, no backtick, no bidi trick.
fn esc(p: &str) -> String {
    p.chars()
        .take(80)
        .map(|c| if plain_ascii(c) { c } else { '?' })
        .collect()
}

impl Refusal {
    pub fn message(&self) -> String {
        let (text, path) = match self {
            Self::NotDefaultBase => ("the PR's base is not the default branch", None),
            Self::TooManyHops(_) => ("too many commits since the approved head", None),
            Self::NotMerge => ("no merge commit after the approved head", None),
            Self::OctopusOrNonMerge => ("a later commit is not a two-parent merge", None),
            Self::BrokenChain => ("the commits are not one chain from the approved head", None),
            Self::SecondParentOffDefault => ("a merge parent is not on the default branch", None),
            Self::MainWentBack => ("a merge brings in an older default-branch commit", None),
            Self::Truncated => ("a compare or tree read was truncated", None),
            Self::PathsOverlapMain { path } => ("main also changed", Some(path)),
            Self::BaseMismatch { path } => ("differs between base and main:", Some(path)),
            Self::TreeMismatch { path } => ("differs from the expected merge:", Some(path)),
            Self::DirFileClash { path } => ("file on one side, dir on the other:", Some(path)),
            Self::DuplicatePath { path } => ("appears twice in a tree:", Some(path)),
            Self::TrustFile { path } => ("the PR changes the trust file", Some(path)),
            Self::LockNotMerged { path } => ("has an entry on neither side:", Some(path)),
        };
        path.map_or(text.to_string(), |p| format!("{text} `{}`", esc(p)))
    }
}

fn paths(files: &[FileChange]) -> BTreeSet<&str> {
    fn names(f: &FileChange) -> [Option<&str>; 2] {
        [Some(f.filename.as_str()), f.previous_filename.as_deref()]
    }
    files.iter().flat_map(names).flatten().collect()
}

/// One lockfile's text at the approved head, at `M` and at `H2`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LockTexts {
    pub h: String,
    pub m: String,
    pub h2: String,
}

/// Whether both sides modified `path`, a lockfile, in place: no add, delete or rename on
/// either side (each must list it once, as `modified`, with no old name).
pub fn lock_in_place(f: &CarryFacts, path: &str) -> bool {
    let in_place = |files: &[FileChange]| {
        let mut hits = files
            .iter()
            .filter(|c| c.filename == path || c.previous_filename.as_deref() == Some(path));
        matches!(
            (hits.next(), hits.next()),
            (Some(c), None) if c.status == "modified" && c.previous_filename.is_none()
        )
    };
    lockfile_kind(path).is_some() && in_place(&f.pr_files) && in_place(&f.main_files)
}

/// Files and gitlinks only: directory shas follow from them and are ignored. Refuses duplicates.
fn files_of(raw: &RawTree) -> Result<Tree, Refusal> {
    let mut out = Tree::new();
    let mut seen = BTreeSet::new();
    for (path, e) in raw {
        if !seen.insert(path) {
            return Err(Refusal::DuplicatePath { path: path.clone() });
        }
        if e.kind != Kind::Tree {
            out.insert(path.clone(), e.clone());
        }
    }
    Ok(out)
}

/// `p` and each of its parent directories, shortest first.
fn with_parents(p: &str) -> Vec<&str> {
    let mut v: Vec<&str> = p.match_indices('/').map(|(i, _)| &p[..i]).collect();
    v.push(p);
    v
}

/// Decide whether an approval for `H` carries over to `H2`. Fails closed.
pub fn check(f: &CarryFacts) -> Result<(), Refusal> {
    let s = |x: &str| x.to_string();
    let n = f.hops.len();
    let refuse = |bad: bool, r: Refusal| if bad { Err(r) } else { Ok(()) };
    refuse(!f.base_is_default, Refusal::NotDefaultBase)?;
    refuse(f.truncated, Refusal::Truncated)?;
    // Rule 1.
    refuse(n == 0, Refusal::NotMerge)?;
    refuse(n > MAX_HOPS, Refusal::TooManyHops(n))?;
    let hs = &f.hops;
    refuse(
        hs.iter().any(|h| h.parents.len() != 2),
        Refusal::OctopusOrNonMerge,
    )?;
    let linked = hs.windows(2).all(|w| w[0].parents[0] == w[1].sha);
    let pinned = hs[0].sha == f.h2 && hs[0].parents[1] == f.m && hs[n - 1].parents[0] == f.h;
    refuse(!linked || !pinned, Refusal::BrokenChain)?;
    refuse(
        hs.iter().any(|h| !h.second_parent_on_default),
        Refusal::SecondParentOffDefault,
    )?;
    let forward = hs[..n - 1]
        .iter()
        .all(|h| h.descends_from_previous_second_parent);
    refuse(!f.b_ancestor_of_m || !forward, Refusal::MainWentBack)?;
    // Rule 2. A lockfile both sides modified in place is the one overlap allowed; rule 6 checks
    // its merged text entry by entry.
    let p = paths(&f.pr_files);
    let mut locks = BTreeSet::new();
    for path in p.intersection(&paths(&f.main_files)) {
        if !lock_in_place(f, path) {
            return Err(Refusal::PathsOverlapMain { path: s(path) });
        }
        locks.insert(*path);
    }
    // Rule 4.
    if let Some(path) = p.iter().find(|x| f.trust_files.contains(x)) {
        return Err(Refusal::TrustFile { path: s(path) });
    }
    // Rule 3.
    let (tb, tm, th, th2) = (
        files_of(&f.tree_b)?,
        files_of(&f.tree_m)?,
        files_of(&f.tree_h)?,
        files_of(&f.tree_h2)?,
    );
    let mut expected = tm.clone();
    for path in &p {
        let chain = with_parents(path);
        // Main changed a merged lockfile itself; its directories must still match.
        let own = if locks.contains(path) {
            chain.len() - 1
        } else {
            chain.len()
        };
        if let Some(q) = chain[..own].iter().find(|q| tm.get(**q) != tb.get(**q)) {
            return Err(Refusal::BaseMismatch { path: s(q) });
        }
        let below = format!("{path}/");
        let file_above = chain[..chain.len() - 1]
            .iter()
            .find(|q| tm.contains_key(**q));
        let dir_below = tm.keys().any(|k| k.starts_with(&below)).then_some(path);
        if let Some(q) = file_above.or(dir_below) {
            return Err(Refusal::DirFileClash { path: s(q) });
        }
        match th.get(*path) {
            Some(e) => expected.insert(s(path), e.clone()),
            None => expected.remove(*path),
        };
    }
    // Rule 6: a merged lockfile holds only whole entries of H's or M's copy.
    for path in locks {
        let refused = || Refusal::LockNotMerged { path: s(path) };
        let e = th2.get(path).ok_or_else(refused)?;
        let texts = f.lock_texts.get(path).ok_or_else(refused)?;
        let kind = lockfile_kind(path).ok_or_else(refused)?;
        let plain = e.kind == Kind::Blob && e.mode == "100644";
        if !plain || !merged_lock_ok(kind, &texts.h, &texts.m, &texts.h2) {
            return Err(refused());
        }
        expected.insert(s(path), e.clone());
    }
    let wrong = expected
        .iter()
        .find(|(k, v)| th2.get(*k) != Some(*v))
        .map(|(k, _)| k);
    let extra = th2.keys().find(|k| !expected.contains_key(*k));
    if let Some(path) = wrong.or(extra) {
        return Err(Refusal::TreeMismatch { path: path.clone() });
    }
    Ok(())
}

#[cfg(test)]
#[rustfmt::skip]
mod tests {
    use super::*;
    fn e(mode: &str, kind: Kind, sha: &str) -> Entry { Entry { mode: mode.into(), kind, sha: sha.into() } }
    fn blob(sha: &str) -> Entry { e("100644", Kind::Blob, sha) }
    fn tr(items: &[(&str, Entry)]) -> RawTree { items.iter().map(|(k, v)| (k.to_string(), v.clone())).collect() }
    fn del(t: &mut RawTree, path: &str) { t.retain(|(k, _)| k != path); }
    fn set(t: &mut RawTree, path: &str, ent: Entry) { del(t, path); t.push((path.into(), ent)); }
    fn fc(name: &str, prev: Option<&str>) -> FileChange {
        FileChange { filename: name.into(), previous_filename: prev.map(String::from), status: "x".into() }
    }
    fn chain(n: usize) -> Vec<Hop> {
        let sha = |i: usize| if i == 0 { "h2".to_string() } else { format!("c{i}") };
        (0..n).map(|i| Hop {
            sha: sha(i),
            parents: vec![if i + 1 == n { "h".into() } else { sha(i + 1) }, format!("m{i}")],
            second_parent_on_default: true,
            descends_from_previous_second_parent: true,
        }).collect()
    }
    fn err(f: &CarryFacts) -> Refusal { check(f).unwrap_err() }
    fn mismatch(p: &str) -> Refusal { Refusal::TreeMismatch { path: p.into() } }
    fn overlap(p: &str) -> Refusal { Refusal::PathsOverlapMain { path: p.into() } }
    fn clash(p: &str) -> Refusal { Refusal::DirFileClash { path: p.into() } }

    /// The PR changed `src/pr.rs`, main `src/main.rs`; directory entries differ, as on GitHub.
    fn honest() -> CarryFacts<'static> {
        let t = |d, main, pr| tr(&[
            ("src", e("040000", Kind::Tree, d)), ("a", blob("a1")),
            ("src/main.rs", blob(main)), ("src/pr.rs", blob(pr)),
        ]);
        let mut hops = chain(1);
        hops[0].parents[1] = "m".into();
        CarryFacts {
            base_is_default: true, h: "h".into(), h2: "h2".into(), m: "m".into(), b: "b".into(),
            hops, b_ancestor_of_m: true,
            pr_files: vec![fc("src/pr.rs", None)], main_files: vec![fc("src/main.rs", None)],
            tree_b: t("d0", "m1", "p0"), tree_m: t("d1", "m2", "p0"),
            tree_h: t("d2", "m1", "p1"), tree_h2: t("d3", "m2", "p1"),
            truncated: false, trust_files: &["CODEOWNERS", ".github/CODEOWNERS"], lock_texts: BTreeMap::new(),
        }
    }
    /// `honest()` where the PR and main both changed `Cargo.lock` and H2 holds a regenerated one.
    fn lock_merge(merged: &str) -> CarryFacts<'static> {
        let pkg = |n: &str, sum: &str| format!("[[package]]\nname = \"{n}\"\nversion = \"1.0.0\"\nchecksum = \"{sum}\"\n");
        let mut f = honest();
        let m = |name: &str| FileChange { filename: name.into(), previous_filename: None, status: "modified".into() };
        f.pr_files.push(m("Cargo.lock"));
        f.main_files.push(m("Cargo.lock"));
        for (t, sha) in [(&mut f.tree_b, "l0"), (&mut f.tree_m, "lm"), (&mut f.tree_h, "lh"), (&mut f.tree_h2, "l2")] {
            set(t, "Cargo.lock", blob(sha));
        }
        let texts = LockTexts {
            h: format!("version = 4\n\n{}", pkg("anyhow", "bb")),
            m: format!("version = 4\n\n{}", pkg("tokio", "cc")),
            h2: merged.replace("ANYHOW", &pkg("anyhow", "bb")).replace("TOKIO", &pkg("tokio", "cc"))
                .replace("STITCHED", &pkg("anyhow", "cc")),
        };
        f.lock_texts.insert("Cargo.lock".into(), texts);
        f
    }
    fn lock_refused() -> Refusal { Refusal::LockNotMerged { path: "Cargo.lock".into() } }
    #[test] fn a_regenerated_lockfile_of_both_sides_carries() {
        assert_eq!(check(&lock_merge("version = 4\n\nANYHOW\nTOKIO")), Ok(()));
    }
    #[test] fn a_lockfile_with_a_new_or_stitched_entry_is_refused() {
        assert_eq!(err(&lock_merge("version = 4\n\nSTITCHED\nTOKIO")), lock_refused());
        let evil = "version = 4\n\nANYHOW\nTOKIO\n[[package]]\nname = \"evil\"\nversion = \"0.1.0\"\n";
        assert_eq!(err(&lock_merge(evil)), lock_refused());
        let mut f = lock_merge("version = 4\n\nANYHOW\nTOKIO");
        f.lock_texts.clear(); // texts not read: fail closed
        assert_eq!(err(&f), lock_refused());
        let mut g = lock_merge("version = 4\n\nANYHOW\nTOKIO");
        set(&mut g.tree_h2, "Cargo.lock", e("100755", Kind::Blob, "l2"));
        assert_eq!(err(&g), lock_refused());
    }
    #[test] fn only_lockfiles_changed_in_place_may_overlap() {
        let mut f = lock_merge("version = 4\n\nANYHOW\nTOKIO");
        f.main_files[1].previous_filename = Some("old.lock".into()); // main renamed it in
        assert_eq!(err(&f), overlap("Cargo.lock"));
        let mut g = lock_merge("version = 4\n\nANYHOW\nTOKIO");
        g.pr_files[1].status = "added".into();
        assert_eq!(err(&g), overlap("Cargo.lock"));
        let mut h = lock_merge("version = 4\n\nANYHOW\nTOKIO");
        h.pr_files.push(fc("src/main.rs", None)); // any other overlap still refuses
        assert_eq!(err(&h), overlap("src/main.rs"));
        let mut k = lock_merge("version = 4\n\nANYHOW\nTOKIO");
        set(&mut k.tree_h2, "src/pr.rs", blob("evil")); // the rest of the tree is still exact
        assert_eq!(err(&k), mismatch("src/pr.rs"));
    }
    #[test] fn honest_nested_merge_carries() { assert_eq!(check(&honest()), Ok(())); }
    #[test] fn take_ours_and_main_moves_are_refused() {
        let mut f = honest(); // H2 kept H's blob for a file main also changed
        f.pr_files.push(fc("src/main.rs", None));
        set(&mut f.tree_h2, "src/main.rs", blob("m1"));
        assert_eq!(err(&f), overlap("src/main.rs"));
        for (pr, main, at) in [
            (fc("x", None), fc("z", Some("x")), "x"), // main renamed a path of P
            (fc("x", None), fc("x", None), "x"),      // main deleted or changed it
            (fc("new", Some("old")), fc("old", None), "old"), // the PR's own rename
        ] {
            let mut g = honest();
            (g.pr_files, g.main_files) = (vec![pr], vec![main]);
            assert_eq!(err(&g), overlap(at));
        }
    }
    #[test] fn going_back_or_off_main_is_refused() {
        let mut f = honest();
        f.b_ancestor_of_m = false;
        assert_eq!(err(&f), Refusal::MainWentBack);
        let mut g = honest();
        (g.hops, g.m) = (chain(2), "m0".into());
        assert_eq!(check(&g), Ok(()));
        g.hops[0].descends_from_previous_second_parent = false;
        assert_eq!(err(&g), Refusal::MainWentBack);
        g.hops[0].descends_from_previous_second_parent = true;
        g.hops[1].descends_from_previous_second_parent = false; // oldest hop: ignored
        assert_eq!(check(&g), Ok(()));
        g.hops[1].second_parent_on_default = false; // an intermediate hop
        assert_eq!(err(&g), Refusal::SecondParentOffDefault);
    }
    #[test] fn broken_chains_are_refused() {
        let edits: [fn(&mut CarryFacts); 5] = [
            |f| f.m = "other".into(),
            |f| f.h2 = "other".into(),
            |f| f.h = "other".into(),
            |f| f.hops[0].parents[0] = "other".into(),
            |f| f.hops.reverse(),
        ];
        for edit in edits {
            let mut f = honest();
            (f.hops, f.m) = (chain(2), "m0".into());
            edit(&mut f);
            assert_eq!(err(&f), Refusal::BrokenChain);
        }
    }
    #[test] fn hop_shapes_are_refused() {
        let mut f = honest();
        f.hops[0].parents.push("third".into());
        assert_eq!(err(&f), Refusal::OctopusOrNonMerge);
        f.hops[0].parents.truncate(1);
        assert_eq!(err(&f), Refusal::OctopusOrNonMerge);
        f.hops.clear();
        assert_eq!(err(&f), Refusal::NotMerge);
        (f.hops, f.m) = (chain(6), "m0".into());
        assert_eq!(err(&f), Refusal::TooManyHops(6));
        f.hops = chain(5);
        assert_eq!(check(&f), Ok(()));
    }
    #[test] fn changed_entries_are_refused() {
        for (path, ent) in [
            ("src/pr.rs", e("100755", Kind::Blob, "p1")), // mode only
            ("src/pr.rs", e("120000", Kind::Blob, "p1")), // symlink
            ("src/pr.rs", e("160000", Kind::Commit, "p1")),
            ("a", e("160000", Kind::Commit, "a1")), // gitlink on an untouched path
            ("a", e("100755", Kind::Blob, "a1")),
            ("evil.sh", blob("x")), // extra file
        ] {
            let mut f = honest();
            set(&mut f.tree_h2, path, ent);
            assert_eq!(err(&f), mismatch(path), "{path}");
        }
        let mut f = honest();
        del(&mut f.tree_h2, "a");
        assert_eq!(err(&f), mismatch("a"));
        let mut g = honest(); // H deleted pr.rs: H2 must not keep it
        del(&mut g.tree_h, "src/pr.rs");
        assert_eq!(err(&g), mismatch("src/pr.rs"));
        del(&mut g.tree_h2, "src/pr.rs");
        assert_eq!(check(&g), Ok(()));
    }
    #[test] fn dir_file_clashes_are_refused() {
        for (pr, main, at) in [
            ("d/x", "d", "d"), ("q/d/x", "q/d", "q/d"), // main has a file where the PR needs a dir
            ("d", "d/y", "d"), ("q/d", "q/d/x", "q/d"), // main has entries below the PR's file
        ] {
            let mut f = honest();
            f.pr_files = vec![fc(pr, None)];
            set(&mut f.tree_m, main, blob("m"));
            set(&mut f.tree_b, main, blob("m"));
            set(&mut f.tree_h, pr, blob("x"));
            assert_eq!(err(&f), clash(at), "{pr} vs {main}");
        }
    }
    #[test] fn base_and_main_must_agree_on_p() {
        let mut f = honest(); // main changed a P path without the compare showing it
        set(&mut f.tree_m, "src/pr.rs", blob("sneaky"));
        assert_eq!(err(&f), Refusal::BaseMismatch { path: "src/pr.rs".into() });
        let mut g = honest(); // ...or one of its parents became a file
        g.pr_files = vec![fc("q/x", None)];
        set(&mut g.tree_m, "q", blob("f"));
        assert_eq!(err(&g), Refusal::BaseMismatch { path: "q".into() });
    }
    #[test] fn duplicates_are_refused() {
        for which in 0..4 {
            let mut f = honest();
            let t = vec![&mut f.tree_b, &mut f.tree_m, &mut f.tree_h, &mut f.tree_h2].swap_remove(which);
            t.push(("a".into(), blob("again")));
            assert_eq!(err(&f), Refusal::DuplicatePath { path: "a".into() });
        }
    }
    #[test] fn truncation_trust_and_base_are_refused() {
        let mut f = honest();
        f.truncated = true;
        assert_eq!(err(&f), Refusal::Truncated);
        let mut g = honest();
        g.base_is_default = false;
        assert_eq!(err(&g), Refusal::NotDefaultBase);
        for pr in [fc("CODEOWNERS", None), fc(".github/CODEOWNERS", None), fc("x", Some(".github/CODEOWNERS"))] {
            let mut t = honest();
            t.pr_files.push(pr);
            assert!(matches!(err(&t), Refusal::TrustFile { .. }));
        }
    }
    #[test] fn empty_p_still_requires_the_tree_of_m() {
        let mut f = honest();
        f.pr_files.clear();
        f.tree_h2 = f.tree_m.clone();
        assert_eq!(check(&f), Ok(()));
        set(&mut f.tree_h2, "src/pr.rs", blob("p1"));
        assert_eq!(err(&f), mismatch("src/pr.rs"));
    }
    #[test] fn messages_escape_paths() {
        let m = Refusal::TrustFile { path: format!("a`b\n\u{202e}{}", "x".repeat(200)) }.message();
        assert!(m.is_ascii() && !m.contains('\n') && m.matches('`').count() == 2 && m.len() < 140);
    }
}
