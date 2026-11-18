//! `bosun rules [--write]` (spec §4): the session rules as a marked block in CLAUDE.md (else
//! AGENTS.md), for sessions whose hooks do not load, such as Claude in Slack. Only the block
//! changes; without `--write` nothing is written.

use std::path::{Path, PathBuf};

use serde_json::json;

use crate::hooks::session_rules::{load, markdown, splice_block};
use crate::output::{BosunError, CmdResult, Outcome};

/// The repo root: the git top level, else `dir`.
pub fn root(dir: Option<&Path>) -> PathBuf {
    let dir = dir.map_or_else(|| PathBuf::from("."), Path::to_path_buf);
    crate::ctx::git_out(&dir, &["rev-parse", "--show-toplevel"]).map_or(dir, PathBuf::from)
}

/// CLAUDE.md when it exists, else AGENTS.md when it exists, else CLAUDE.md (created).
pub fn target(root: &Path) -> &'static str {
    match (
        root.join("CLAUDE.md").is_file(),
        root.join("AGENTS.md").is_file(),
    ) {
        (false, true) => "AGENTS.md",
        _ => "CLAUDE.md",
    }
}

/// The file's text with the current block in place, and whether that differs from the file.
/// A symlink, a file that is not UTF-8 or cannot be read is an error, never read as empty.
pub fn rendered(root: &Path, block: &str) -> Result<(String, bool), BosunError> {
    let file = target(root);
    let path = root.join(file);
    let bad = |why: String| {
        BosunError::error(
            format!("{file}: {why}"),
            format!(
                "make {file} a regular UTF-8 file in the repo, then rerun `bosun rules --write`"
            ),
        )
    };
    let old = match std::fs::symlink_metadata(&path) {
        Ok(m) if m.file_type().is_symlink() => return Err(bad("is a symlink".into())),
        Ok(_) => std::fs::read_to_string(&path).map_err(|e| bad(e.to_string()))?,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => return Err(bad(e.to_string())),
    };
    let new = splice_block(&old, block).ok_or_else(|| {
        BosunError::error(
            format!("{file}: the bosun session-rules markers are not one start then one end"),
            format!(
                "leave one start and one end marker line in {file}, then run `bosun rules --write`"
            ),
        )
    })?;
    let changed = new != old;
    Ok((new, changed))
}

pub fn run(dir: Option<&Path>, write: bool) -> CmdResult {
    let root = root(dir);
    let file = target(&root);
    let block = markdown(&load(Some(&root)).0);
    let (new, changed) = rendered(&root, &block)?;
    if write && changed {
        std::fs::write(root.join(file), &new).map_err(|e| {
            BosunError::error(
                format!("{file}: {e}"),
                "check the file's permissions, then rerun",
            )
        })?;
    }
    Ok(Outcome::data(
        json!({"file": file, "changed": changed, "written": write && changed, "block": block}),
    ))
}
