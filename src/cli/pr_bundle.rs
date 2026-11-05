//! PR-time backstop of D-016: a PR that closes several tasks never carries gated files. Plan
//! import, wave and claim keep gated work out of bundles; this catches a bundle PR whose files
//! say otherwise (a subtree touch that grew a gated file, a hand-made PR).

use crate::app::{Result, base_files, gated_from_default};
use crate::cli::plan::epic_shape;
use crate::core::meta::{closing_issues_in, parse_task_meta};
use crate::core::shape::meta_gated;
use crate::github::{Github, Issue, PrFile, Pull};
use crate::output::BosunError;

/// The `Closes #N` lines for the issues bundled with `lead` (`bosun pr <lead> --bundle N...`),
/// each on its own line. Every one must be an open task of the lead's epic, whose shape is
/// single, and not gated (D-016); else exit 1 before anything changes.
pub(crate) fn bundle_lines(gh: &dyn Github, lead: &Issue, bundle: &[u64]) -> Result<String> {
    let mut out = String::new();
    if bundle.is_empty() {
        return Ok(out);
    }
    let lead_meta = parse_task_meta(lead.body_text());
    let globs = gated_from_default(gh)?;
    let files = base_files(gh)?;
    let gated = |m: &crate::core::meta::TaskMeta| meta_gated(m, &globs, files.as_deref());
    if lead.state != "open" || lead_meta.as_ref().is_none_or(gated) {
        return Err(BosunError::error(
            format!(
                "#{} is closed or gated: only an open, ungated lead bundles (D-016)",
                lead.number
            ),
            "pass only the `bundle` issues `bosun wave plan` listed",
        ));
    }
    let lead_epic = lead_meta.and_then(|m| m.epic);
    let single = match lead_epic {
        Some(e) => gh
            .get_issue(e)?
            .is_some_and(|i| epic_shape(i.body_text()).as_deref() == Some("single")),
        None => false,
    };
    let refuse = |why: String| {
        BosunError::error(
            why,
            "pass only the `bundle` issues `bosun wave plan` listed",
        )
    };
    if !single {
        return Err(refuse(format!(
            "#{} is not a task of a single-shape epic: only those bundle",
            lead.number
        )));
    }
    for (i, &n) in bundle.iter().enumerate() {
        if n == lead.number || bundle[..i].contains(&n) {
            return Err(refuse(format!("#{n} is listed twice or is the lead")));
        }
        let issue = gh
            .get_issue(n)?
            .filter(|i| !i.pull_request && i.state == "open")
            .ok_or_else(|| refuse(format!("#{n} is not an open issue")))?;
        let meta = parse_task_meta(issue.body_text())
            .ok_or_else(|| refuse(format!("#{n} is not a bosun task")))?;
        if meta.epic != lead_epic {
            return Err(refuse(format!("#{n} belongs to another epic")));
        }
        if gated(&meta) {
            return Err(refuse(format!(
                "#{n} is gated: gated work needs its own single-task PR (D-016)"
            )));
        }
        out.push_str(&format!("Closes #{n}\n"));
    }
    Ok(out)
}

/// An existing PR must already close every bundled issue; its body is never edited here (exit 3).
pub(crate) fn require_bundle_closes(pull: &Pull, repo: &str, bundle: &[u64]) -> Result<()> {
    let closed = closing_issues_in(&pull.body, Some(repo));
    if bundle.iter().all(|n| closed.contains(n)) {
        return Ok(());
    }
    Err(BosunError::conflict(
        format!(
            "PR #{} does not close every bundled issue ({})",
            pull.number,
            bundle
                .iter()
                .map(|n| format!("#{n}"))
                .collect::<Vec<_>>()
                .join(", ")
        ),
        "add `Closes #N` for each bundled issue to the PR body, then rerun",
    ))
}

/// The same D-016 check for a PR about to be created: its files are the comparison of the
/// default branch with the pushed branch (at most 300; a longer list fails closed).
pub(crate) fn refuse_gated_new(
    gh: &dyn Github,
    base: &str,
    branch: &str,
    closes: usize,
) -> Result<()> {
    if closes < 2 {
        return Ok(());
    }
    let files = gh.compare_files(base, branch)?;
    let incomplete = files.len() >= 300;
    gated_check(gh, None, closes, &files, incomplete)
}

/// Refuse (exit 3) a PR that closes more than one task and changes a gated file, or whose file
/// list is incomplete (fail closed). Rules come from the default branch.
pub(crate) fn refuse_gated_bundle(gh: &dyn Github, pull: &Pull, repo: &str) -> Result<()> {
    let closes = closing_issues_in(&pull.body, Some(repo)).len();
    if closes <= 1 {
        return Ok(());
    }
    let files = gh.pull_files(pull.number)?;
    let total = gh
        .get_pull(pull.number)?
        .map_or(pull.changed_files, |p| p.changed_files);
    let incomplete = total > 0 && (files.len() as u64) < total;
    gated_check(gh, Some(pull.number), closes, &files, incomplete)
}

fn gated_check(
    gh: &dyn Github,
    pr: Option<u64>,
    closes: usize,
    files: &[PrFile],
    incomplete: bool,
) -> Result<()> {
    let globs = gated_from_default(gh)?;
    let what = pr.map_or("a new PR".to_string(), |n| format!("PR #{n}"));
    if incomplete {
        return Err(BosunError::conflict(
            format!(
                "{what} closes {closes} tasks and its file list is incomplete ({} files): \
                 gated files cannot be ruled out",
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
            "{what} closes {closes} tasks and changes gated file(s) {list}: gated work needs its own single-task PR (D-016)"
        ),
        "move the gated changes to their own task and PR, then run `bosun pr` again",
    ))
}
