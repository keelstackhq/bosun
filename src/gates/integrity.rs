//! test-integrity (spec §7): compares each test file at the base with the working tree and refuses
//! a deleted test, a new skip, fewer assertions, or a snapshot change without a reason.

use std::path::Path;
use std::process::Command;

use serde_json::{Value, json};

use std::collections::BTreeMap;

use super::adapters::{self as rs, Marker};
use super::git;
use super::ts;
use crate::output::{BosunError, CmdResult, Outcome};

pub struct Request<'a> {
    pub root: &'a Path,
    pub base: &'a str,
    /// The PR body, searched for `snapshot-change: <reason>`.
    pub body: &'a str,
    /// Finding kinds the task allows: `delete-test`, `skip`, `fewer-asserts`.
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

/// A path is read by the ts adapters when it is TypeScript, else by the rust ones.
fn is_ts(path: &str) -> bool {
    path.ends_with(".ts") || path.ends_with(".tsx")
}

fn is_test_file(path: &str, text: &str) -> bool {
    if is_ts(path) {
        ts::is_test_file(path)
    } else {
        rs::is_test_file(path, text)
    }
}

fn pending_markers(path: &str, text: &str) -> Vec<Marker> {
    if is_ts(path) {
        ts::pending_markers(text)
    } else {
        rs::pending_markers(text)
    }
}

fn test_count(path: &str, text: &str) -> usize {
    if is_ts(path) {
        ts::test_count(text)
    } else {
        rs::test_count(text)
    }
}

fn assertions(path: &str, text: &str) -> usize {
    if is_ts(path) {
        ts::assertions(text)
    } else {
        rs::assertions(text)
    }
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
        let base_asserts = old_text.as_deref().map_or(0, |t| assertions(file, t));

        let Some(head) = &new_text else {
            let allowed = allows("delete-test");
            findings.push(finding(
                "deleted-test",
                file,
                Some(base_asserts),
                None,
                allowed,
            ));
            continue;
        };
        // Plain skips are counted; a UC marker in head that base lacks (by id, with
        // multiplicity) is a skip too. Removing UC markers is fine.
        let plain = |t: &str| {
            pending_markers(file, t)
                .iter()
                .filter(|m| m.uc.is_none())
                .count()
        };
        let ucs = |t: &str| {
            let mut m: BTreeMap<String, usize> = BTreeMap::new();
            for id in pending_markers(file, t).into_iter().filter_map(|m| m.uc) {
                *m.entry(id).or_default() += 1;
            }
            m
        };
        let base_ucs = old_text.as_deref().map(ucs).unwrap_or_default();
        let new_ucs: usize = ucs(head)
            .into_iter()
            .map(|(id, n)| n.saturating_sub(base_ucs.get(&id).copied().unwrap_or(0)))
            .sum();
        let base_skips = old_text.as_deref().map_or(0, plain);
        let head_skips = plain(head);
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
        let base_tests = old_text.as_deref().map_or(0, |t| test_count(file, t));
        let head_tests = test_count(file, head);
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
        let head_asserts = assertions(file, head);
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
    let data = json!({"base": req.base, "files_checked": files_checked, "findings": findings});
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
