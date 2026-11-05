//! Reading and writing local gate stamps: `<git common dir>/bosun/gates/<tree>/<gate>`, where
//! `<tree>` is the tree of the working state (HEAD plus `git add -A`, built in a temporary index). The pure naming is in `core::stamps`.

use std::path::{Path, PathBuf};
use std::process::Command;

use crate::config::Config;
use crate::core::stamps::{rel_path, required_gates, safe_name};
use crate::gates::profile::Profile;
use crate::output::BosunError;

/// Stdout of `git <args>` in `dir`, trimmed; `None` when git fails or prints nothing.
fn git_out(dir: &Path, args: &[&str]) -> Option<String> {
    let out = Command::new("git")
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
        Command::new("git")
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

/// Write the stamp of a passing `gate` for the current working state; the path written.
/// Errors are for stderr only: a gate never fails because it could not stamp.
pub fn write(dir: &Path, gate: &str) -> Result<PathBuf, String> {
    let tree = working_tree(dir).ok_or("cannot read the tree of the working state")?;
    let path = stamp_file(dir, &tree, gate).ok_or("cannot find the git directory")?;
    let parent = path.parent().ok_or("bad stamp path")?;
    std::fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
    std::fs::write(&path, "ok\n").map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(path)
}

/// Whether `gate` is stamped for `tree`.
pub fn has(dir: &Path, tree: &str, gate: &str) -> bool {
    stamp_file(dir, tree, gate).is_some_and(|p| p.is_file())
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
