//! Reads the facts for approval carry-over (design 2026-10-27 §3 C7) from GitHub, strictly by sha,
//! and maps them to `core::carry`, which decides. Every error, `None`, mismatch or unknown value
//! is a refusal with a short reason; no GitHub message reaches it, paths only escaped by core.

use std::collections::HashMap;

use super::risk_check::{decisive_review, delegation_blocked, verdict_inputs, withdrawn};
use super::verdict::CONTEXT;
use crate::core::approve::TRUST_FILES;
use crate::core::carry::{self, CarryFacts, Entry, FileChange, Hop, Kind, RawTree, Refusal};
use crate::core::records::{Record, parse_record};
use crate::core::risk::{VERDICT_AUTHOR, apply_record};
use crate::github::{Comment, Compare, GitCommit, Github, PrFile, Pull, Review, Status};

/// Seven hex characters of a sha taken from GitHub data, else `?`.
fn short(sha: &str) -> String {
    let s: String = sha.chars().take(7).collect();
    let hex = s.len() == 7 && s.chars().all(|c| c.is_ascii_hexdigit());
    if hex { s } else { "?".into() }
}

/// A full lowercase sha: everything is read and compared by these, never by a name.
fn sha40(s: &str) -> bool {
    s.len() == 40 && s.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

fn on_default(status: &str) -> bool {
    status == "ahead" || status == "identical"
}

struct Reads<'a> {
    gh: &'a dyn Github,
    commits: HashMap<String, GitCommit>,
}

impl Reads<'_> {
    /// The commit `sha`, which must come back under that very sha. Read once per `check`.
    fn commit(&mut self, sha: &str) -> Result<GitCommit, String> {
        if let Some(c) = self.commits.get(sha) {
            return Ok(c.clone());
        }
        let full = |c: &GitCommit| c.parents.iter().chain([&c.sha, &c.tree]).all(|s| sha40(s));
        let c = self.gh.commit(sha).ok().flatten();
        let c = c.filter(|c| c.sha == sha && full(c));
        let c = c.ok_or_else(|| format!("cannot read commit {}", short(sha)))?;
        self.commits.insert(sha.into(), c.clone());
        Ok(c)
    }

    fn compare(&self, base: &str, head: &str) -> Result<Compare, String> {
        let why = |_| format!("cannot compare {} with {}", short(base), short(head));
        self.gh.compare(base, head).map_err(why)
    }

    fn ancestor(&self, a: &str, b: &str) -> bool {
        self.compare(a, b).is_ok_and(|c| on_default(&c.status))
    }

    /// The tree of commit `x`, bound to the commit: the tree read must be the tree it names.
    /// Returns the entries and whether GitHub cut the listing short.
    fn tree(&mut self, x: &str) -> Result<(RawTree, bool), String> {
        let c = self.commit(x)?;
        let bad = || format!("cannot read tree of commit {}", short(x));
        let t = self.gh.tree(&c.tree).ok().flatten();
        let t = t.filter(|t| t.sha == c.tree).ok_or_else(bad)?;
        let mut out = RawTree::new();
        for e in t.entries {
            let kind = match e.kind.as_str() {
                "blob" => Kind::Blob,
                "tree" => Kind::Tree,
                "commit" => Kind::Commit,
                _ => return Err(bad()),
            };
            let (mode, sha) = (e.mode, e.sha);
            out.push((e.path, Entry { mode, kind, sha }));
        }
        Ok((out, t.truncated))
    }
}

fn changes(c: &Compare) -> Vec<FileChange> {
    let f = |f: &PrFile| FileChange {
        filename: f.filename.clone(),
        previous_filename: f.previous_filename.clone(),
        status: f.status.clone(),
    };
    c.files.iter().map(f).collect()
}

/// Whether an approval of `h` carries to `pull.head_sha` (rules 1-4). `Ok` is the path from
/// `H2` down to the oldest hop (`h` excluded); `Err` is a safe, short reason.
pub fn check(
    gh: &dyn Github,
    pull: &Pull,
    default_branch: &str,
    h: &str,
) -> Result<Vec<String>, String> {
    if pull.base_ref != default_branch {
        return Err(Refusal::NotDefaultBase.message());
    }
    let (d, h2) = (pull.base_sha.as_str(), pull.head_sha.as_str());
    let mut r = Reads {
        gh,
        commits: HashMap::new(),
    };
    if ![h, h2, d].into_iter().all(sha40) {
        return Err("a head or base is not a full sha".into());
    }
    if h == h2 {
        return Err("the approved head is the current head".into());
    }
    // D vouches for every merge parent, so it must still be on the default branch tip.
    if !r.ancestor(d, &format!("refs/heads/{default_branch}")) {
        return Err("the PR's base commit is not on the default branch now".into());
    }
    // Rule 1: the first-parent chain from H2 down to the commit whose first parent is `h`.
    let mut chain = Vec::new();
    let mut sha = h2.to_string();
    loop {
        if chain.len() == carry::MAX_HOPS {
            return Err(Refusal::TooManyHops(chain.len() + 1).message());
        }
        let c = r.commit(&sha)?;
        if c.parents.len() != 2 {
            return Err(Refusal::OctopusOrNonMerge.message());
        }
        let done = c.parents[0] == h;
        sha = c.parents[0].clone();
        chain.push(c);
        if done {
            break;
        }
    }
    let hops: Vec<Hop> = chain
        .iter()
        .enumerate()
        .map(|(i, c)| Hop {
            sha: c.sha.clone(),
            parents: c.parents.clone(),
            second_parent_on_default: r.ancestor(&c.parents[1], d),
            descends_from_previous_second_parent: chain
                .get(i + 1)
                .is_none_or(|older| r.ancestor(&older.parents[1], &c.parents[1])),
        })
        .collect();
    // Rule 3's inputs: M, B and the two file lists.
    let m = chain[0].parents[1].clone();
    let b = r
        .compare(d, h)?
        .merge_base
        .filter(|b| sha40(b))
        .ok_or("GitHub names no merge base")?;
    let main = r.compare(&b, &m)?;
    let pr = r.compare(&b, h)?;
    let mut truncated = main.truncated || pr.truncated;
    let mut tr = |x: &str| {
        let (t, cut) = r.tree(x)?;
        truncated |= cut;
        Ok::<_, String>(t)
    };
    let (tree_b, tree_m, tree_h, tree_h2) = (tr(&b)?, tr(&m)?, tr(h)?, tr(h2)?);
    let facts = CarryFacts {
        base_is_default: true,
        h: h.into(),
        h2: h2.into(),
        m,
        b,
        hops,
        truncated,
        b_ancestor_of_m: on_default(&main.status),
        pr_files: changes(&pr),
        main_files: changes(&main),
        tree_b,
        tree_m,
        tree_h,
        tree_h2,
        trust_files: &TRUST_FILES,
    };
    carry::check(&facts).map_err(|e| e.message())?;
    Ok(chain.into_iter().map(|c| c.sha).collect())
}

pub fn is_verdict(s: &Status) -> bool {
    s.context.eq_ignore_ascii_case(CONTEXT)
}

/// Whether `sha` has a `review-verdict`; an error, no statuses (risk-check posted one) or 100+ count as yes.
pub fn has_verdict(gh: &dyn Github, sha: &str) -> bool {
    !gh.statuses(sha)
        .is_ok_and(|l| (1..100).contains(&l.len()) && !l.iter().any(is_verdict))
}

/// The heads an approval could carry from, nearest first: first parents of the merges on top of
/// the PR (at most `MAX_HOPS`). Empty off the default branch; the walk ends at any doubt.
pub fn candidates(gh: &dyn Github, pull: &Pull, default_branch: &str) -> Vec<String> {
    let mut out = Vec::new();
    if pull.base_ref != default_branch {
        return out;
    }
    let mut sha = pull.head_sha.clone();
    while out.len() < carry::MAX_HOPS {
        let c = gh.commit(&sha).ok().flatten();
        let Some(c) = c.filter(|c| {
            c.sha == sha && c.parents.len() == 2 && sha40(&sha) && sha40(&c.parents[0])
        }) else {
            break;
        };
        sha = c.parents[0].clone();
        out.push(sha.clone());
    }
    out
}

/// The id of the newest verdict record on `h` (workflow, unedited) if it approves independently.
fn approved_by_independent(comments: &[Comment], excluded: &[String], h: &str) -> Option<u64> {
    let bot = |c: &&Comment| c.user.eq_ignore_ascii_case(VERDICT_AUTHOR);
    let on_h = |c: &Comment| match apply_record(&c.body) {
        Some(Record::Verdict {
            session,
            head_sha,
            approved,
            confidence,
            ..
        }) if head_sha == h => Some((session, approved && confidence.is_some())),
        _ => None,
    };
    let newest = comments
        .iter()
        .filter(bot)
        .filter_map(|c| Some((c, on_h(c)?)))
        .next_back();
    let (c, (session, approved)) = newest?;
    let known = !session.is_empty() && session != "unknown";
    (approved && known && c.unedited() && !excluded.contains(&session)).then_some(c.id)
}

/// Rule 5: no record on the path, none newer than comment `approval` for a head off it, and no
/// owner withdrawn (per commit) on the path or `h`.
fn standing(
    comments: &[Comment],
    reviews: &[Review],
    owners: &[String],
    path: &[String],
    (h, approval): (&str, u64),
) -> bool {
    let superseded = comments.iter().any(|c| match parse_record(&c.body) {
        Some(Record::Verdict { head_sha, .. }) => {
            path.contains(&head_sha) || (c.id > approval && head_sha != h)
        }
        _ => false,
    });
    let withdrawn_by = |o: &String| {
        let at = |sha: &str| withdrawn(decisive_review(reviews, o, sha).as_deref());
        at(h) || path.iter().any(|sha| at(sha))
    };
    !superseded && !owners.iter().any(withdrawn_by)
}

/// The head `h` whose approval carries to `pull.head_sha` if rules 1-5 hold; any doubt is `None`.
pub fn carry_verdict(
    gh: &dyn Github,
    pull: &Pull,
    default_branch: &str,
    linked: &[u64],
    owners: &[String],
) -> Option<String> {
    let cands = candidates(gh, pull, default_branch);
    let open = || matches!(delegation_blocked(gh, &pull.base_ref, linked), Ok(None));
    if cands.is_empty() || linked.is_empty() || has_verdict(gh, &pull.head_sha) || !open() {
        return None;
    }
    let (comments, excluded) = verdict_inputs(gh, pull.number, linked).ok()?;
    let mut reviews = gh.list_reviews(pull.number).ok()?;
    reviews.sort_by_key(|r| r.id);
    cands.into_iter().find(|h| {
        let Some(id) = approved_by_independent(&comments, &excluded, h) else {
            return false;
        };
        (check(gh, pull, default_branch, h))
            .map_err(|why| eprintln!("bosun: risk-check: no carry from {}: {why}", short(h)))
            .is_ok_and(|path| standing(&comments, &reviews, owners, &path, (h, id)))
    })
}
