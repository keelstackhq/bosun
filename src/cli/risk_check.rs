//! `bosun risk-check <pr>`: judge a PR with the risk rules of its base branch (spec section 8).
//! The rules, the owners and the manifests at base are read at the base SHA, never from the head.

use std::path::Path;

use serde_json::json;

use super::PrArg;
use crate::app::{App, Result, task_meta};
use crate::core::meta::linked_issues;
use crate::core::risk::{
    self, Approval, ChangedFile, FileView, Rules, VERDICTS_NEEDED, added_lines, capability_labels,
    comment_approves, evaluate, lease_sessions, new_dependencies, parse_owners, replace_note,
    requirement, risk_note, two_verdicts, unseen_diff,
};
use crate::github::{Comment, Github};
use crate::output::{BosunError, CmdResult, Outcome};

const RULES_PATH: &str = ".bosun/risk.toml";
pub const OWNERS_PATH: &str = "governance/owners.toml";
const CONTEXT: &str = "risk-check";

/// Owner logins from `governance/owners.toml` at `git_ref`; empty when the file is missing.
pub fn owner_logins(gh: &dyn Github, git_ref: &str) -> Result<Vec<String>> {
    let Some(text) = gh.file_at(OWNERS_PATH, git_ref)? else {
        return Ok(Vec::new());
    };
    let owners = parse_owners(&text).map_err(|e| {
        BosunError::error(
            e,
            format!("fix {OWNERS_PATH}: one `[[owner]]` table with `github = \"login\"` per owner"),
        )
    })?;
    Ok(owners.into_iter().map(|o| o.github).collect())
}

/// Slack IDs of the owners in `governance/owners.toml` at `git_ref`; empty when none is set.
pub fn owner_slack_ids(gh: &dyn Github, git_ref: &str) -> Result<Vec<String>> {
    let Some(text) = gh.file_at(OWNERS_PATH, git_ref)? else {
        return Ok(Vec::new());
    };
    let owners = parse_owners(&text).map_err(|e| {
        BosunError::error(
            e,
            format!("fix {OWNERS_PATH}: one `[[owner]]` table with `github = \"login\"` per owner"),
        )
    })?;
    Ok(owners.into_iter().filter_map(|o| o.slack).collect())
}

fn is_manifest(path: &str) -> bool {
    matches!(path.rsplit('/').next(), Some("Cargo.toml" | "package.json"))
}

pub fn run(dir: Option<&Path>, args: &PrArg) -> CmdResult {
    let app = App::load(dir)?;
    let gh = app.gh.as_ref();
    let n = args.pr;
    let pull = gh.get_pull(n)?.ok_or_else(|| {
        BosunError::error(
            format!("PR #{n} does not exist"),
            "check the number; `gh pr list` lists the open PRs",
        )
    })?;
    let (base, head) = (pull.base_sha.as_str(), pull.head_sha.as_str());

    let rules = match gh.file_at(RULES_PATH, base)? {
        Some(text) => Rules::parse(&text).map_err(|e| {
            BosunError::error(
                format!("{RULES_PATH} at base {base}: {e}"),
                "fix the file on the base branch in a PR; a PR cannot change the rules that judge it",
            )
        })?,
        None => Rules::defaults(),
    };

    let pr_files = gh.pull_files(n)?;
    let files: Vec<ChangedFile> = pr_files
        .iter()
        .map(|f| ChangedFile {
            path: f.filename.clone(),
            previous_path: f.previous_filename.clone(),
            added_lines: f.patch.as_deref().map(added_lines).unwrap_or_default(),
        })
        .collect();

    let mut manifests = Vec::new();
    for f in pr_files.iter().filter(|f| is_manifest(&f.filename)) {
        let old_path = f.previous_filename.as_deref().unwrap_or(&f.filename);
        let old = if f.status == "added" {
            None
        } else {
            gh.file_at(old_path, base)?
        };
        let new = if f.status == "removed" {
            None
        } else {
            gh.file_at(&f.filename, head)?
        };
        manifests.push((f.filename.clone(), old, new));
    }
    let new_deps = new_dependencies(&manifests);

    let mut capabilities = capability_labels(&pull.labels);
    // Every linked issue counts: the agent writes the PR body, so a decoy first link must not
    // hide the real task's capabilities.
    for i in linked_issues(&pull.body) {
        if let Some(issue) = gh.get_issue(i)?
            && let Some(meta) = task_meta(&issue)
        {
            capabilities.extend(meta.capabilities);
        }
    }

    let mut matches = evaluate(&rules, &files, &new_deps, &capabilities);
    let views: Vec<FileView> = pr_files
        .iter()
        .map(|f| FileView {
            path: f.filename.clone(),
            status: f.status.clone(),
            has_patch: f.patch.is_some(),
            changes: f.additions + f.deletions,
        })
        .collect();
    matches.extend(unseen_diff(&views, pull.changed_files));
    let level = risk::level(&matches);
    let high = level == "high";

    let note = high.then(|| risk_note(&matches, head));
    let body = replace_note(&pull.body, note.as_deref());
    if body != pull.body {
        gh.update_pull_body(n, &body)?;
    }

    let owners = if high {
        owner_logins(gh, base)?
    } else {
        Vec::new()
    };
    let approved_by = if high {
        approver(gh, n, head, &owners)?
    } else {
        None
    };
    let requirement = requirement(&matches);
    // Two verdicts only matter when the owner has not approved and the class allows them.
    let linked = linked_issues(&pull.body);
    // Without a linked task there is no lease to check the reviewers against: fail closed.
    let unlinked = linked.is_empty() && requirement == Approval::TwoVerdicts;
    let verdicts =
        if high && approved_by.is_none() && requirement == Approval::TwoVerdicts && !unlinked {
            let mut comments = gh.list_comments(n)?;
            comments.sort_by_key(|c| c.id);
            // The reviewer must not be a session that worked on the task: every session with a
            // lease record (past or live) on a linked issue is out. `bosun pr` records no session
            // for the PR author, so the lease is the only trace of who wrote the change.
            let mut excluded = Vec::new();
            for i in linked {
                let notes = gh.list_comments(i)?;
                excluded.extend(lease_sessions(notes.iter().map(|c| c.body.as_str())));
            }
            two_verdicts(
                comments.iter().map(|c| (c.user.as_str(), c.body.as_str())),
                head,
                &excluded,
            )
        } else {
            Vec::new()
        };
    let by_verdicts = verdicts.len() >= VERDICTS_NEEDED;
    let passed = !high || approved_by.is_some() || by_verdicts;

    let ids: Vec<&str> = matches.iter().map(|m| m.rule.as_str()).collect();
    let description = match (&approved_by, high) {
        (_, false) => "normal risk".to_string(),
        (Some(who), _) => format!("high risk ({}), approved by {who}", ids.join(", ")),
        (None, _) if by_verdicts => format!(
            "high risk ({}), approved by two verdicts: {}",
            ids.join(", "),
            verdicts.join(", ")
        ),
        (None, _) if unlinked => format!(
            "high risk, owner approval needed (links no task, so verdicts cannot count): {}",
            ids.join(", ")
        ),
        (None, _) if requirement == Approval::TwoVerdicts => format!(
            "high risk, two verdicts needed ({}/{VERDICTS_NEEDED}): {}",
            verdicts.len(),
            ids.join(", ")
        ),
        (None, _) => format!("high risk, owner approval needed: {}", ids.join(", ")),
    };
    let state = if passed { "success" } else { "failure" };
    gh.create_status(head, CONTEXT, state, &description, None)?;

    let mut data = json!({"pr": n, "head_sha": head, "level": level, "matches": matches,
        "approved_by": approved_by});
    if high {
        data["requirement"] = json!(requirement.name());
        if requirement == Approval::TwoVerdicts {
            data["verdicts"] = json!(verdicts);
        }
    }
    if passed {
        return Ok(Outcome::data(data));
    }
    let owner_fix = if owners.is_empty() {
        format!("fill {OWNERS_PATH} on the base branch, then an owner must approve head {head}")
    } else {
        format!(
            "an owner in {OWNERS_PATH} must approve head {head}: a GitHub review, or the PR comment `/bosun approve {head}`"
        )
    };
    let fix = if unlinked {
        format!(
            "the PR links no task (`Closes #N`), so verdicts cannot be checked for independence: link the task, or get an owner approval: {owner_fix}"
        )
    } else if requirement == Approval::TwoVerdicts {
        format!(
            "get a second review verdict from a different session (approve, confidence >= {}) on head {head}, or an owner approval: {owner_fix}",
            risk::VERDICT_MIN_CONFIDENCE
        )
    } else {
        owner_fix
    };
    Err(BosunError::needs_human(
        format!(
            "PR #{n} is high risk ({}) and not approved on head {head}",
            ids.join(", ")
        ),
        fix,
    )
    .with_data(data))
}

/// The owner who approved `head`: by a GitHub review (their newest decisive review on `head` is
/// an approval) or by an unedited PR comment `/bosun approve <head>`.
fn approver(gh: &dyn Github, pr: u64, head: &str, owners: &[String]) -> Result<Option<String>> {
    let reviews = gh.list_reviews(pr)?;
    let decisive = |o: &str| {
        reviews
            .iter()
            .rev()
            .filter(|r| r.user.eq_ignore_ascii_case(o) && r.commit_id == head)
            .find(|r| {
                matches!(
                    r.state.as_str(),
                    "APPROVED" | "CHANGES_REQUESTED" | "DISMISSED"
                )
            })
            .map(|r| r.state.clone())
    };
    let by_review = owners
        .iter()
        .find(|o| decisive(o).as_deref() == Some("APPROVED"));
    if let Some(o) = by_review {
        return Ok(Some(o.clone()));
    }
    let comments = gh.list_comments(pr)?;
    // Only a comment written with a plain user token counts: agent sessions act as the same
    // login through an app, and GitHub marks that in `performed_via_github_app` (spec §8).
    // An edited comment does not count: users with write access can edit others' comments. An
    // owner whose newest decisive review on `head` requests changes has withdrawn the approval.
    Ok(owners
        .iter()
        .filter(|o| decisive(o).as_deref() != Some("CHANGES_REQUESTED"))
        .find(|o| owner_comment(&comments, o, head).is_some())
        .cloned())
}

/// The comment that approves `head` for `owner`: unedited, written with a plain user token and
/// exactly `/bosun approve <head>`.
pub fn owner_comment<'a>(comments: &'a [Comment], owner: &str, head: &str) -> Option<&'a Comment> {
    comments.iter().find(|c| {
        c.user.eq_ignore_ascii_case(owner)
            && c.unedited()
            && c.by_user_token()
            && comment_approves(&c.body, head)
    })
}
