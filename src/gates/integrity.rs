//! test-integrity (spec §7): compares each test file at the base with the working tree and refuses
//! a deleted test, a new skip, fewer assertions, or a snapshot change without a reason.

use std::path::Path;
use std::process::Command;

use serde_json::{Value, json};

use std::collections::BTreeMap;

use super::adapters::{self as rs, Marker, RegexKit};
use super::profile::Profile;
use super::ts;
use super::{git, stack_rel};
use crate::config::Stack;
use crate::output::{BosunError, CmdResult, Outcome};

pub struct Request<'a> {
    pub root: &'a Path,
    pub stacks: &'a [Stack],
    pub base: &'a str,
    /// The PR body, searched for `snapshot-change: <reason>`.
    pub body: &'a str,
    /// Finding kinds the task allows: `delete-test`, `skip`, `only`, `fewer-asserts`.
    /// `delete-test` also allows `fewer-tests`.
    pub allow: &'a [String],
}

/// One changed path: `old` is its path at the base (none when added), `new` its path at head
/// (none when deleted).
#[derive(Debug, PartialEq, Eq)]
struct Change {
    old: Option<String>,
    new: Option<String>,
}

/// Parse `git diff --name-status -M` output.
fn parse_changes(text: &str) -> Vec<Change> {
    text.lines()
        .filter_map(|line| {
            let mut parts = line.split('\t');
            let status = parts.next()?;
            let first = parts.next()?.to_string();
            let second = parts.next().map(String::from);
            Some(match status.chars().next()? {
                'A' => Change {
                    old: None,
                    new: Some(first),
                },
                'D' => Change {
                    old: Some(first),
                    new: None,
                },
                'R' | 'C' => Change {
                    old: Some(first),
                    new: second,
                },
                _ => Change {
                    old: Some(first.clone()),
                    new: Some(first),
                },
            })
        })
        .collect()
}

/// The text of `path` at `base`, or none when it did not exist there.
fn base_text(root: &Path, base: &str, path: &str) -> Option<String> {
    let out = Command::new("git")
        .arg("-C")
        .arg(root)
        .arg("show")
        .arg(format!("{base}:{path}"))
        .output()
        .ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).into_owned())
}

/// The adapters of one stack: the built-in ones (chosen by file extension: rust for `.rs`, ts for
/// TypeScript and JavaScript, anything else is not a test) or the profile's generic regex ones.
#[derive(Debug)]
enum Kit {
    Builtin,
    Regex(RegexKit),
}

/// The built-in adapter family of a path.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Rust,
    Ts,
    Other,
}

fn kind(path: &str) -> Kind {
    if path.ends_with(".rs") {
        Kind::Rust
    } else if ts::handles(path) {
        Kind::Ts
    } else {
        Kind::Other
    }
}

impl Kit {
    fn is_test_file(&self, path: &str, text: &str) -> bool {
        match (self, kind(path)) {
            (Kit::Regex(k), _) => k.is_test_file(path),
            (_, Kind::Rust) => rs::is_test_file(path, text),
            (_, Kind::Ts) => ts::is_test_file(path),
            _ => false,
        }
    }

    fn pending_markers(&self, path: &str, text: &str) -> Vec<Marker> {
        match (self, kind(path)) {
            (Kit::Regex(k), _) => k.pending_markers(text),
            (_, Kind::Rust) => rs::pending_markers(text),
            (_, Kind::Ts) => ts::pending_markers(text),
            _ => Vec::new(),
        }
    }

    fn test_count(&self, path: &str, text: &str) -> usize {
        match (self, kind(path)) {
            (Kit::Regex(_), _) => 0,
            (_, Kind::Rust) => rs::test_count(text),
            (_, Kind::Ts) => ts::test_count(text),
            _ => 0,
        }
    }

    fn assertions(&self, path: &str, text: &str) -> usize {
        match (self, kind(path)) {
            (Kit::Regex(k), _) => k.assertions(text),
            (_, Kind::Rust) => rs::assertions(text),
            (_, Kind::Ts) => ts::assertions(text),
            _ => 0,
        }
    }

    /// Focused tests (`it.only`): only the ts adapters know them.
    fn only_count(&self, path: &str, text: &str) -> usize {
        match (self, kind(path)) {
            (Kit::Builtin, Kind::Ts) => ts::only_count(text),
            _ => 0,
        }
    }
}

/// One kit per stack, keyed by the stack's repo-relative prefix (`py/`, or empty for the root).
/// Every stack gets one, so the innermost stack wins: a regex stack at the root does not claim
/// the files of a rust or ts stack below it.
fn kits(root: &Path, stacks: &[Stack]) -> Result<Vec<(String, Kit)>, BosunError> {
    let mut out = Vec::new();
    for stack in stacks {
        let profile = Profile::load(root, &stack.profile)?;
        let rel = stack_rel(&stack.path);
        let prefix = if rel.is_empty() {
            String::new()
        } else {
            format!("{rel}/")
        };
        let kit = if profile.adapters.uses_regex() {
            Kit::Regex(
                RegexKit::new(&profile.adapters.regex, &stack.path).map_err(|e| {
                    BosunError::error(
                        format!("profile {}: {e}", stack.profile),
                        format!(
                            "fix [adapters.regex] in .bosun/profiles/{}.toml",
                            stack.profile
                        ),
                    )
                })?,
            )
        } else {
            Kit::Builtin
        };
        out.push((prefix, kit));
    }
    Ok(out)
}

/// The kit of the innermost stack that holds the path; files in no stack use the built-in ones.
fn kit_for<'a>(kits: &'a [(String, Kit)], path: &str) -> &'a Kit {
    static BUILTIN: Kit = Kit::Builtin;
    kits.iter()
        .filter(|(prefix, _)| path.starts_with(prefix.as_str()))
        .max_by_key(|(prefix, _)| prefix.len())
        .map_or(&BUILTIN, |(_, kit)| kit)
}

/// Why integrity is reduced, if it is. Each stack (by path, once) whose profile uses a generic
/// adapter, or whose profile cannot be loaded (a typo in an adapter name, say), is named, for
/// example "stack py/ uses regex adapters". Shown by `bosun status` and in the test-integrity data.
pub fn reduced_reason(root: &Path, stacks: &[Stack]) -> Option<String> {
    let mut reasons: Vec<String> = Vec::new();
    for s in stacks {
        let why = match Profile::load(root, &s.profile) {
            Ok(p) => match p.adapters.generic_kind() {
                Some(k) => format!("stack {} uses {k} adapters", s.path),
                None => continue,
            },
            Err(e) => format!("stack {} has an unusable profile: {}", s.path, e.message),
        };
        if !reasons.contains(&why) {
            reasons.push(why);
        }
    }
    (!reasons.is_empty()).then(|| reasons.join("; "))
}

fn has_reason(body: &str) -> bool {
    body.lines().any(|l| {
        l.trim()
            .strip_prefix("snapshot-change:")
            .is_some_and(|r| !r.trim().is_empty())
    })
}

fn finding(
    kind: &str,
    file: &str,
    base: Option<usize>,
    head: Option<usize>,
    allowed: bool,
) -> Value {
    json!({"kind": kind, "file": file, "base": base, "head": head, "allowed": allowed})
}

pub fn run(req: &Request) -> CmdResult {
    // Compare with the merge base: a branch behind its base must not look like it lost tests.
    let merge_base = git(req.root, &["merge-base", req.base, "HEAD"])?
        .trim()
        .to_string();
    let diff = git(
        req.root,
        &["diff", "--name-status", "-M", &merge_base, "HEAD"],
    )?;
    let kits = kits(req.root, req.stacks)?;
    // Each side is read with the kit of the innermost stack holding its own path.
    let is_test_file = |p: &str, t: &str| kit_for(&kits, p).is_test_file(p, t);
    let pending_markers = |p: &str, t: &str| kit_for(&kits, p).pending_markers(p, t);
    let test_count = |p: &str, t: &str| kit_for(&kits, p).test_count(p, t);
    let assertions = |p: &str, t: &str| kit_for(&kits, p).assertions(p, t);
    let only_count = |p: &str, t: &str| kit_for(&kits, p).only_count(p, t);
    let allows = |name: &str| req.allow.iter().any(|a| a == name);
    let mut findings: Vec<Value> = Vec::new();
    let mut files_checked = 0;

    for change in parse_changes(&diff) {
        let old_text = change
            .old
            .as_deref()
            .and_then(|p| base_text(req.root, &merge_base, p));
        let new_text = change
            .new
            .as_deref()
            .and_then(|p| std::fs::read_to_string(req.root.join(p)).ok());

        if let Some(new) = &change.new
            && new.ends_with(".snap")
            && !has_reason(req.body)
        {
            findings.push(finding("snapshot-change", new, None, None, false));
        }

        // Each side is read with the adapter for its own path: a rename out of the test file
        // pattern (`a.test.ts` -> `a.test.ts.disabled`) must not hide what the base had.
        let was_test = matches!((&change.old, &old_text), (Some(p), Some(t)) if is_test_file(p, t));
        let is_test = matches!((&change.new, &new_text), (Some(p), Some(t)) if is_test_file(p, t));
        if !was_test && !is_test {
            continue;
        }
        files_checked += 1;
        let file = change
            .new
            .as_deref()
            .or(change.old.as_deref())
            .unwrap_or("");
        let (old_path, new_path) = (
            change.old.as_deref().unwrap_or(file),
            change.new.as_deref().unwrap_or(file),
        );
        let base_text = old_text.as_deref().filter(|_| was_test);
        let base_asserts = base_text.map_or(0, |t| assertions(old_path, t));

        let head = match &new_text {
            Some(h) if is_test => h,
            _ => {
                // Deleted, or no longer a test file: the base tests are gone.
                let allowed = allows("delete-test");
                findings.push(finding(
                    "deleted-test",
                    file,
                    Some(base_asserts),
                    None,
                    allowed,
                ));
                continue;
            }
        };
        // Plain skips are counted; a UC marker in head that base lacks (by id, with
        // multiplicity) is a skip too. Removing UC markers is fine.
        let plain = |p: &str, t: &str| {
            pending_markers(p, t)
                .iter()
                .filter(|m| m.uc.is_none())
                .count()
        };
        let ucs = |p: &str, t: &str| {
            let mut m: BTreeMap<String, usize> = BTreeMap::new();
            for id in pending_markers(p, t).into_iter().filter_map(|m| m.uc) {
                *m.entry(id).or_default() += 1;
            }
            m
        };
        let base_ucs = base_text.map(|t| ucs(old_path, t)).unwrap_or_default();
        let new_ucs: usize = ucs(new_path, head)
            .into_iter()
            .map(|(id, n)| n.saturating_sub(base_ucs.get(&id).copied().unwrap_or(0)))
            .sum();
        let base_skips = base_text.map_or(0, |t| plain(old_path, t));
        let head_skips = plain(new_path, head);
        if head_skips > base_skips || new_ucs > 0 {
            let allowed = allows("skip");
            findings.push(finding(
                "new-skip",
                file,
                Some(base_skips),
                Some(head_skips + new_ucs),
                allowed,
            ));
        }
        let base_only = base_text.map_or(0, |t| only_count(old_path, t));
        let head_only = only_count(new_path, head);
        if head_only > base_only {
            let allowed = allows("only");
            findings.push(finding(
                "new-only",
                file,
                Some(base_only),
                Some(head_only),
                allowed,
            ));
        }
        let base_tests = base_text.map_or(0, |t| test_count(old_path, t));
        let head_tests = test_count(new_path, head);
        if head_tests < base_tests {
            let allowed = allows("delete-test");
            findings.push(finding(
                "fewer-tests",
                file,
                Some(base_tests),
                Some(head_tests),
                allowed,
            ));
        }
        let head_asserts = assertions(new_path, head);
        if head_asserts < base_asserts {
            let allowed = allows("fewer-asserts");
            findings.push(finding(
                "fewer-asserts",
                file,
                Some(base_asserts),
                Some(head_asserts),
                allowed,
            ));
        }
    }

    let refused = findings.iter().filter(|f| f["allowed"] == false).count();
    let mut data = json!({"base": req.base, "files_checked": files_checked, "findings": findings});
    match reduced_reason(req.root, req.stacks) {
        Some(why) => {
            data["integrity"] = json!("reduced");
            data["integrity_reason"] = json!(why);
        }
        None => data["integrity"] = json!("full"),
    }
    if refused > 0 {
        return Err(BosunError::error(
            format!("test-integrity: {refused} findings"),
            "restore the test or assertion, or have the plan task allow it (spec §7)",
        )
        .with_data(data));
    }
    Ok(Outcome::data(data))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn name_status_lines_parse() {
        let c = parse_changes("M\ta.rs\nA\tb.rs\nD\tc.rs\nR087\td.rs\te.rs\n");
        assert_eq!(c.len(), 4);
        assert_eq!(c[0].old.as_deref(), Some("a.rs"));
        assert_eq!(c[1].old, None);
        assert_eq!(c[2].new, None);
        assert_eq!(c[3].old.as_deref(), Some("d.rs"));
        assert_eq!(c[3].new.as_deref(), Some("e.rs"));
    }

    #[test]
    fn snapshot_reason_must_be_non_empty() {
        assert!(has_reason("x\nsnapshot-change: new field"));
        assert!(!has_reason("snapshot-change:  "));
        assert!(!has_reason("see snapshot-change: x"));
    }
}
