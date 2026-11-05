//! PR-time backstop of D-016: a PR that closes several tasks never carries gated files. Plan
//! import, wave and claim keep gated work out of bundles; this catches a bundle PR whose files
//! say otherwise (a subtree touch that grew a gated file, a hand-made PR).

use crate::app::{Result, gated_from_default};
use crate::core::meta::closing_issues;
use crate::github::{Github, Pull};
use crate::output::BosunError;

/// Refuse (exit 3) a PR that closes more than one task and changes a gated file, or whose file
/// list is incomplete (fail closed). Rules come from the default branch.
pub(crate) fn refuse_gated_bundle(gh: &dyn Github, pull: &Pull) -> Result<()> {
    let closes = closing_issues(&pull.body).len();
    if closes <= 1 {
        return Ok(());
    }
    let globs = gated_from_default(gh)?;
    let files = gh.pull_files(pull.number)?;
    let total = gh
        .get_pull(pull.number)?
        .map_or(pull.changed_files, |p| p.changed_files);
    if total > 0 && (files.len() as u64) < total {
        return Err(BosunError::conflict(
            format!(
                "PR #{} closes {closes} tasks and its file list is incomplete ({} of {total} files): \
                 gated files cannot be ruled out",
                pull.number,
                files.len()
            ),
            "split the PR so each task has its own PR, then run `bosun pr` again",
        ));
    }
    let gated = |name: &str| {
        let low = name.to_lowercase();
        globs.iter().any(|g| g.matches(name) || g.matches(&low))
    };
    let hit: Vec<&str> = (files.iter())
        .flat_map(|f| std::iter::once(&f.filename).chain(f.previous_filename.as_ref()))
        .filter(|n| gated(n))
        .map(String::as_str)
        .collect();
    if hit.is_empty() {
        return Ok(());
    }
    let more = hit.len().saturating_sub(5);
    let mut list = hit[..hit.len().min(5)].join(", ");
    if more > 0 {
        list.push_str(&format!(" and {more} more"));
    }
    Err(BosunError::conflict(
        format!(
            "PR #{} closes {closes} tasks and changes gated file(s) {list}: gated work needs its own single-task PR (D-016)",
            pull.number
        ),
        "move the gated changes to their own task and PR, then run `bosun pr` again",
    ))
}
