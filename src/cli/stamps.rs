//! Reading and writing local gate stamps: `<git common dir>/bosun/gates/<tree>/<gate>`, where
//! `<tree>` is the tree of the working state (HEAD plus `git add -A`, built in a temporary index). The pure naming is in `core::stamps`.

use std::path::{Path, PathBuf};

use crate::config::Config;
use crate::core::stamps::{rel_path, required_gates, safe_name};
use crate::gates::profile::Profile;
use crate::output::BosunError;

/// Stdout of `git <args>` in `dir`, trimmed; `None` when git fails or prints nothing.
fn git_out(dir: &Path, args: &[&str]) -> Option<String> {
    let out = crate::gates::cmd("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .output()
        .ok()?;
    let text = String::from_utf8_lossy(&out.stdout).trim().to_string();
    (out.status.success() && !text.is_empty()).then_some(text)
}

/// The tree of `HEAD`.
pub fn head_tree(dir: &Path) -> Option<String> {
    git_out(dir, &["rev-parse", "HEAD^{tree}"])
}

/// The tree of commit `sha`; `None` when the checkout does not have it.
pub fn tree_of(dir: &Path, sha: &str) -> Option<String> {
    let hex = !sha.is_empty() && sha.chars().all(|c| c.is_ascii_hexdigit());
    hex.then(|| {
        git_out(
            dir,
            &["rev-parse", "--verify", "-q", &format!("{sha}^{{tree}}")],
        )
    })?
}

/// The full sha of `HEAD`.
pub fn head_sha(dir: &Path) -> Option<String> {
    git_out(dir, &["rev-parse", "HEAD"])
}

/// The tree of what the gates just ran on: `HEAD` plus every change in the working tree,
/// untracked non-ignored files included (what `git add -A` would commit). It is built in a
/// temporary index, so the real index and tree stay untouched.
pub fn working_tree(dir: &Path) -> Option<String> {
    let git_dir = PathBuf::from(git_out(
        dir,
        &["rev-parse", "--path-format=absolute", "--git-dir"],
    )?);
    let index = git_dir.join(format!("bosun-stamp-index-{}", std::process::id()));
    let run = |args: &[&str]| {
        crate::gates::cmd("git")
            .arg("-C")
            .arg(dir)
            .env("GIT_INDEX_FILE", &index)
            .args(args)
            .output()
            .ok()
    };
    // Any failure yields None, never a fallback to HEAD: the tree may be dirty.
    let tree = (|| {
        if !run(&["read-tree", "HEAD"])?.status.success() {
            return None;
        }
        if !run(&["add", "-A"])?.status.success() {
            return None;
        }
        let out = run(&["write-tree"])?;
        let text = String::from_utf8_lossy(&out.stdout).trim().to_string();
        (out.status.success() && !text.is_empty()).then_some(text)
    })();
    let _ = std::fs::remove_file(&index);
    tree
}

/// The directory that holds `bosun/gates`: the git common dir, shared by all worktrees.
fn common_dir(dir: &Path) -> Option<PathBuf> {
    git_out(
        dir,
        &["rev-parse", "--path-format=absolute", "--git-common-dir"],
    )
    .map(PathBuf::from)
}

fn stamp_file(dir: &Path, tree: &str, gate: &str) -> Option<PathBuf> {
    safe_name(gate).then_some(())?;
    Some(common_dir(dir)?.join(rel_path(tree, gate)))
}

/// Write the stamp of a passing `gate` for the current working state; the path written. A gate
/// that skipped (nothing to check on this tree) is stamped `skipped`: it counts as done.
/// Errors are for stderr only: a gate never fails because it could not stamp.
pub fn write(dir: &Path, gate: &str, skipped: bool) -> Result<PathBuf, String> {
    let tree = working_tree(dir).ok_or("cannot read the tree of the working state")?;
    let path = stamp_file(dir, &tree, gate).ok_or("cannot find the git directory")?;
    let parent = path.parent().ok_or("bad stamp path")?;
    std::fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
    std::fs::write(&path, if skipped { "skipped\n" } else { "ok\n" })
        .map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(path)
}

/// Whether `gate` is stamped for `tree`.
pub fn has(dir: &Path, tree: &str, gate: &str) -> bool {
    stamp_file(dir, tree, gate).is_some_and(|p| p.is_file())
}

/// Whether `rev` is vouched for by an ancestor (at most 50 back) whose tree has every one of
/// `gates` stamped, with only docs changed since (spec §7: stamps carry over docs-only diffs).
/// `git` runs a git command in the pushing checkout. Not for a ts-pnpm stack (prettier checks
/// Markdown): every stack must be `rust`, as CI's fast path.
pub fn carried(
    root: &Path,
    rev: &str,
    gates: &[String],
    git: &dyn Fn(&[&str]) -> Option<String>,
) -> bool {
    let rust = Config::load(root).is_ok_and(|c| c.stack.iter().all(|s| s.profile == "rust"));
    let Some(revs) = git(&["rev-list", "--max-count=50", rev]).filter(|_| rust) else {
        return false;
    };
    for c in revs.lines().skip(1) {
        let Some(tree) = git(&["rev-parse", &format!("{c}^{{tree}}")]) else {
            return false;
        };
        if gates.iter().all(|g| has(root, &tree, g)) {
            // A diff git cannot print vouches for nothing (the same tree needs no carry-over).
            let diff = git(&[
                "diff",
                "--no-relative",
                "--no-renames",
                "--name-only",
                c,
                rev,
                "--",
            ]);
            return diff.is_some_and(|d| d.lines().all(crate::core::stamps::docs_only));
        }
    }
    false
}

/// The gates a PR must have passed before it leaves draft, for the repo's configured stacks.
pub fn ready_gates(root: &Path) -> Result<Vec<String>, BosunError> {
    let config = Config::load(root)?;
    let mut per_stack = Vec::new();
    for stack in &config.stack {
        let profile = Profile::load(root, &stack.profile)?;
        per_stack.push(profile.gates.keys().cloned().collect::<Vec<_>>());
    }
    Ok(required_gates(&per_stack))
}
