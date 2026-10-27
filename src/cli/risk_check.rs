//! `bosun risk-check <pr>`: judge a PR with the risk rules of its base branch (spec section 8).
//! The rules, the owners and the manifests at base are read at the base SHA, never from the head.

use std::path::Path;

use serde_json::json;

use super::PrArg;
use super::carry_over::{candidates, carry_approval, carry_verdict, has_verdict, is_verdict};
use super::verdict::CONTEXT as VERDICT;
use crate::app::{App, Result, default_branch, default_ref, halt_reason, notes, task_meta};
use crate::core::approve::{
    APPROVERS_PATH, ApproverKey, Role, parse_approvers, signature_lines, signed_message,
    signed_message_v2, trust_files_changed, verify_ed25519,
};
use crate::core::lease::leases_of;
use crate::core::meta::linked_issues;
use crate::core::risk::{
    self, Approval, ChangedFile, FileView, Rules, VERDICTS_NEEDED, added_lines, apply_floor,
    capability_labels, clean_verdicts, comment_approves, evaluate, lease_sessions,
    new_dependencies, parse_owners, plain, replace_note, requirement, risk_note, two_verdicts,
    unseen_diff,
};
use crate::core::touch::{Changed, outside};
use crate::ctx::RepoId;
use crate::github::{Comment, Github, PrFile, Pull, Review, b64};
use crate::output::{BosunError, CmdResult, Exit, Outcome};
use crate::time::{Timestamp, parse_rfc3339};

const RULES_PATH: &str = ".bosun/risk.toml";
pub const OWNERS_PATH: &str = "governance/owners.toml";
const CONTEXT: &str = "risk-check";
/// A base with this file carries bosun's own approval code, which is then critical too.
const BOSUN_CODE_MARKER: &str = "src/cli/risk_check.rs";

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
    // In flight, a required check is pending, so auto-merge waits for this run's verdict.
    if let Err(e) = gh.create_status(&pull.head_sha, CONTEXT, "pending", PENDING, None) {
        eprintln!(
            "bosun: risk-check: cannot post the pending status: {}",
            e.message
        );
    }
    let out = judge(&app, pull.clone());
    // A run that cannot finish must not leave `pending` behind (exit 5 posted its verdict).
    if out.as_ref().is_err_and(|e| e.exit != Exit::NeedsHuman) {
        _ = gh.create_status(&pull.head_sha, CONTEXT, "error", UNFINISHED, None);
    }
    out
}

const PENDING: &str = "risk-check is judging this head";
const UNFINISHED: &str = "risk-check could not finish; fix the error and re-run the risk workflow";

fn judge(app: &App, pull: Pull) -> CmdResult {
    let gh = app.gh.as_ref();
    let n = pull.number;
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
    let mut task = None;
    for i in linked_issues(&pull.body) {
        if let Some(issue) = gh.get_issue(i)?
            && let Some(meta) = task_meta(&issue)
        {
            capabilities.extend(meta.capabilities);
            task.get_or_insert(i);
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
    // The critical floor: a delegate never approves the approval system, secrets or prod.
    let critical = if matches.iter().any(|m| m.approval == Approval::Delegate) {
        let bosun_code = gh.file_at(BOSUN_CODE_MARKER, base)?.is_some();
        risk::critical(&matches, &files, bosun_code)
    } else {
        Vec::new()
    };
    apply_floor(&mut matches, &critical);
    let level = risk::level(&matches);
    let high = level == "high";
    let out_of_touch = out_of_touch(app, task, &pr_files, pull.changed_files);

    let warning = out_of_touch.as_deref().and_then(touch_warning);
    if let Some(w) = &warning {
        eprintln!("bosun: risk-check: {w}");
    }
    // A warning only: the status description, the level and the state stay as they are. The
    // PR body carries it where it already carries a Risk note (high risk).
    let note = high.then(|| {
        let note = risk_note(&matches, head);
        match &warning {
            Some(w) => note.replacen(
                "## Risk: high\n\n",
                &format!("## Risk: high\n\n- **warning:** {w}\n"),
                1,
            ),
            None => note,
        }
    });
    let body = replace_note(&pull.body, note.as_deref());
    if body != pull.body {
        gh.update_pull_body(n, &body)?;
    }

    let owners = if high {
        owner_logins(gh, base)?
    } else {
        Vec::new()
    };
    // Key and owner changes need the owner's own GitHub approval, never a signature.
    let trust = trust_files_changed(
        files
            .iter()
            .map(|f| (f.path.as_str(), f.previous_path.as_deref())),
    );
    if !trust.is_empty() {
        eprintln!(
            "bosun: risk-check: the PR changes {}: signed approvals do not count",
            trust.join(", ")
        );
    }
    // An unseen diff may hide a trust file: no signature counts then either.
    let unseen = matches.iter().any(|m| m.rule == "R-unseen");
    if unseen {
        eprintln!(
            "bosun: risk-check: bosun could not see the whole diff: signed approvals do not count"
        );
    }
    let signed_ok = trust.is_empty() && !unseen;
    let requirement = requirement(&matches);
    let linked = linked_issues(&pull.body);
    // Without a linked task there is no lease to check the reviewers against: fail closed.
    let unlinked =
        linked.is_empty() && matches!(requirement, Approval::TwoVerdicts | Approval::Delegate);
    let need = Need {
        repo: &app.ctx.repo,
        pr: n,
        base_ref: &pull.base_ref,
        owners: &owners,
        signed_ok,
        requirement,
        linked: &linked,
        unlinked,
    };
    let found = if high {
        approval_on(gh, &need, head)?
    } else {
        Found::default()
    };
    if let Some(why) = &found.delegate_blocked {
        eprintln!("bosun: risk-check: a delegate cannot approve: {why}");
    }
    let passed_on_head = !high || found.passed();
    // An older head's approval carries over a clean merge of main (design §3 C7), judged by `need`.
    // The delegate carry comes in a later PR with its tests.
    let carried = if passed_on_head || requirement == Approval::Delegate {
        None
    } else {
        default_branch(gh).ok().and_then(|branch| {
            let approve = |h: &str| approval_on(gh, &need, h).ok();
            carry_approval(gh, &pull, &branch, &owners, &approve)
        })
    };
    let Found {
        mut approved_by,
        verdicts,
        delegate_verdicts,
        delegate_blocked,
    } = found;
    let carried_from = carried.as_ref().map(|(h, _)| h.clone());
    if let Some((_, who)) = carried {
        approved_by = Some(who);
    }
    let by_verdicts = verdicts.len() >= VERDICTS_NEEDED;
    let passed = passed_on_head || carried_from.is_some();

    let ids: Vec<&str> = matches.iter().map(|m| m.rule.as_str()).collect();
    let tag = |h: &String| format!(" (carried from {})", &h[..7]); // a full sha (`candidates`)
    let from: String = carried_from.iter().map(tag).collect();
    let description = match (&approved_by, high) {
        (_, false) => "normal risk".to_string(),
        (Some(who), _) => format!("high risk ({}), approved by {who}{from}", ids.join(", ")),
        (None, _) if by_verdicts => format!(
            "high risk ({}), approved by two verdicts: {}",
            ids.join(", "),
            verdicts.join(", ")
        ),
        (None, _) if unlinked => format!(
            "high risk, owner approval needed (links no task, so verdicts cannot count): {}",
            ids.join(", ")
        ),
        (None, _) if !critical.is_empty() => format!(
            "high risk, owner approval needed (critical: {}): {}",
            critical.join(", "),
            ids.join(", ")
        ),
        (None, _) if requirement == Approval::Delegate => format!(
            "high risk, owner approval needed (or a delegate): {}",
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
    // After the check is posted, and never an error: a doubt or a failed read is no carry.
    let carried = carry_status(gh, &pull, &linked);

    let mut data = json!({"pr": n, "head_sha": head, "level": level, "matches": matches,
        "approved_by": approved_by, "out_of_touch": out_of_touch,
        "verdict_carried_from": carried, "approval_carried_from": carried_from});
    if high {
        data["requirement"] = json!(requirement.name());
        if requirement == Approval::TwoVerdicts {
            data["verdicts"] = json!(verdicts);
        }
        if requirement == Approval::Delegate {
            data["delegate_verdicts"] = json!(delegate_verdicts);
            data["delegate_blocked"] = json!(delegate_blocked);
        }
        if !critical.is_empty() {
            data["critical"] = json!(critical);
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
    } else if requirement == Approval::Delegate {
        format!(
            "get one independent review verdict (approve, confidence >= {}, no major finding) on head {head} and a delegate's signed approval (`bosun approve {n}@{head} --sign-key <pem> --as delegate`), or an owner approval: {owner_fix}",
            risk::VERDICT_MIN_CONFIDENCE
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

/// What an approval is judged by: the checked head's own diff, the rules and owners at base.
struct Need<'a> {
    repo: &'a RepoId,
    pr: u64,
    base_ref: &'a str,
    owners: &'a [String],
    signed_ok: bool,
    requirement: Approval,
    linked: &'a [u64],
    unlinked: bool,
}

/// How a head is approved: by an owner or delegate (`approved_by`), or by two verdicts.
#[derive(Default)]
pub(crate) struct Found {
    pub(crate) approved_by: Option<String>,
    pub(crate) verdicts: Vec<String>,
    delegate_verdicts: Vec<String>,
    delegate_blocked: Option<String>,
}

impl Found {
    pub(crate) fn passed(&self) -> bool {
        self.approved_by.is_some() || self.verdicts.len() >= VERDICTS_NEEDED
    }

    /// The sessions whose verdicts count in this approval; `None` when an owner approved.
    pub(crate) fn counted(&self) -> Option<&[String]> {
        self.approved_by.is_none().then_some(&self.verdicts[..])
    }
}

/// The approval of a high-risk PR on `sha`: an owner, two verdicts, or a delegate.
fn approval_on(gh: &dyn Github, need: &Need, sha: &str) -> Result<Found> {
    let Need {
        repo, pr, owners, ..
    } = *need;
    let mut found = Found {
        approved_by: approver(gh, repo, pr, sha, owners, need.signed_ok)?,
        ..Found::default()
    };
    let open = found.approved_by.is_none() && !need.unlinked;
    // Two verdicts only matter when the owner has not approved and the class allows them.
    if open && need.requirement == Approval::TwoVerdicts {
        let (comments, excluded) = verdict_inputs(gh, pr, need.linked)?;
        found.verdicts = two_verdicts(
            comments.iter().map(|c| (c.user.as_str(), c.body.as_str())),
            sha,
            &excluded,
        );
    }
    // A delegate decides only with one clean independent verdict on the head (spec §8).
    if open && need.requirement == Approval::Delegate {
        found.delegate_blocked = delegation_blocked(gh, need.base_ref, need.linked)?;
        if found.delegate_blocked.is_none() && need.signed_ok {
            let (comments, excluded) = verdict_inputs(gh, pr, need.linked)?;
            found.delegate_verdicts = clean_verdicts(
                comments.iter().map(|c| (c.user.as_str(), c.body.as_str())),
                sha,
                &excluded,
            );
            if !found.delegate_verdicts.is_empty() && !owners_withdrawn(gh, pr, sha, owners)? {
                let v1 = |after: Option<u64>| after.is_none();
                let key = signed_approver(gh, repo, pr, sha, &comments, Role::Delegate, &v1)?;
                found.approved_by = key.map(|id| format!("delegate {id}"));
            }
        }
    }
    Ok(found)
}

/// Carries `review-verdict` of an older head onto exactly `pull.head_sha` (design §3 C7), checking
/// its statuses before and after posting (verdict.yml is not serialized with us). Never an error.
fn carry_status(gh: &dyn Github, pull: &Pull, linked: &[u64]) -> Option<String> {
    let head = pull.head_sha.as_str();
    // A carried status on top of a real verdict (a run was cancelled mid-post) is repaired first.
    let top = gh
        .statuses(head)
        .ok()
        .and_then(|l| l.into_iter().find(is_verdict));
    if let Some(s) = top.filter(|s| s.description.starts_with(CARRIED)) {
        settle(gh, head, &s.description);
        return None;
    }
    let branch = default_branch(gh).ok()?;
    if candidates(gh, pull, &branch).is_empty() {
        return None;
    }
    let owners = owner_logins(gh, &pull.base_sha).ok()?;
    let h = carry_verdict(gh, pull, &branch, linked, &owners)?;
    let short: String = h.chars().take(7).collect();
    let text = format!("{CARRIED}{short}: clean merge of the default branch");
    if has_verdict(gh, head) {
        return None;
    }
    gh.create_status(head, VERDICT, "success", &text, None)
        .ok()?;
    settle(gh, head, &text).then_some(h)
}

const CARRIED: &str = "carried from ";
const UNSURE: &str = "carry withdrawn: could not confirm no verdict raced it";

/// True only when `ours` stands unchallenged on `head`. Another review-verdict is a real verdict
/// that raced us: the newest is posted again. If that fails, or the history is unreadable: withdraw.
fn settle(gh: &dyn Github, head: &str, ours: &str) -> bool {
    let post = |state: &str, text: &str| gh.create_status(head, VERDICT, state, text, None);
    let seen = gh.status_history(head).map(|l| {
        l.into_iter()
            .find(|s| is_verdict(s) && s.description != ours && !s.description.starts_with(CARRIED))
    });
    match seen {
        Ok(None) => return true,
        Ok(Some(f)) if post(&f.state, &f.description).is_ok() => {}
        _ => _ = post("failure", UNSURE),
    }
    eprintln!("bosun: risk-check: a verdict raced the carry-over; it was not kept");
    false
}

/// "N files outside the task's lease: a, b" (ten names at most, then "and N more"); `None` when
/// there are none.
fn touch_warning(out: &[String]) -> Option<String> {
    if out.is_empty() {
        return None;
    }
    // File names are repo data in a PR body: quoted, and `plain`, so one cannot end the note.
    let quote = |n: &String| format!("`{}`", plain(n));
    let mut names = out
        .iter()
        .take(10)
        .map(quote)
        .collect::<Vec<_>>()
        .join(", ");
    if out.len() > 10 {
        names.push_str(&format!(" and {} more", out.len() - 10));
    }
    let n = out.len();
    Some(format!(
        "{n} file{} outside the task's lease: {names}",
        if n == 1 { "" } else { "s" }
    ))
}

/// The PR files outside the linked task's newest live lease (design C3), for a warning. `None`
/// when the PR links no task, the task has no live lease with a touch, the file list is cut or
/// the comments cannot be read. The task is the first linked issue with task metadata; humans'
/// PRs have no lease and so get `None`.
fn out_of_touch(
    app: &App,
    task: Option<u64>,
    pr_files: &[PrFile],
    changed_files: u64,
) -> Option<Vec<String>> {
    // GitHub lists at most 3000 files; a shorter list than it reports is cut.
    if (pr_files.len() as u64) < changed_files || pr_files.len() >= 3000 {
        return None;
    }
    let task = task?;
    // Only a warning: a failed read costs the warning, never the check.
    let notes = match notes(app.gh.as_ref(), task) {
        Ok(notes) => notes,
        Err(e) => {
            eprintln!(
                "bosun: risk-check: no touch warning, cannot read the comments of #{task}: {}",
                e.message
            );
            return None;
        }
    };
    let lease = leases_of(task, &notes)
        .into_iter()
        .filter(|l| l.is_live(app.ctx.now) && !l.touch.is_empty())
        .max_by_key(|l| l.comment_id)?;
    let changed: Vec<Changed> = pr_files
        .iter()
        .map(|f| Changed {
            path: f.filename.clone(),
            status: f.status.clone(),
            previous: f.previous_filename.clone(),
        })
        .collect();
    Some(outside(&changed, &lease.touch))
}

/// `owner`'s newest approve, request-changes or dismissed review on `head`.
fn decisive<'a>(reviews: &'a [Review], owner: &str, head: &str) -> Option<&'a Review> {
    reviews.iter().rev().find(|r| {
        r.user.eq_ignore_ascii_case(owner)
            && r.commit_id == head
            && matches!(
                r.state.as_str(),
                "APPROVED" | "CHANGES_REQUESTED" | "DISMISSED"
            )
    })
}

/// The id of `owner`'s decisive review on `head` when it was dismissed.
pub fn dismissed_review(reviews: &[Review], owner: &str, head: &str) -> Option<u64> {
    decisive(reviews, owner, head)
        .filter(|r| r.state == "DISMISSED")
        .map(|r| r.id)
}

/// The state of `owner`'s newest approve, request-changes or dismissed review on `head`.
pub fn decisive_review(reviews: &[Review], owner: &str, head: &str) -> Option<String> {
    decisive(reviews, owner, head).map(|r| r.state.clone())
}

/// The owner who approved `head`: by a GitHub review (their newest decisive review on `head` is
/// an approval) or by an unedited PR comment `/bosun approve <head>`.
fn approver(
    gh: &dyn Github,
    repo: &RepoId,
    pr: u64,
    head: &str,
    owners: &[String],
    signed_ok: bool,
) -> Result<Option<String>> {
    let reviews = gh.list_reviews(pr)?;
    let state = |o: &str| decisive_review(&reviews, o, head);
    let by_review = owners
        .iter()
        .find(|o| state(o).as_deref() == Some("APPROVED"));
    if let Some(o) = by_review {
        return Ok(Some(o.clone()));
    }
    let comments = gh.list_comments(pr)?;
    // Only a comment written with a plain user token counts: agent sessions act as the same
    // login through an app, and GitHub marks that in `performed_via_github_app` (spec §8).
    // An edited comment does not count: users with write access can edit others' comments. An
    // owner whose newest decisive review on `head` requests changes, or was dismissed, has
    // withdrawn the approval; after a dismissal only a comment made later than the dismissal
    // (its time on the issue timeline, not the review's) clears it.
    let by_comment = owners.iter().find(|o| match state(o).as_deref() {
        Some("DISMISSED") => {
            let newer = newer_than(&comments, dismissed_at(gh, pr, &reviews, o, head));
            owner_comment(&newer, o, head).is_some()
        }
        s if withdrawn(s) => false,
        _ => owner_comment(&comments, o, head).is_some(),
    });
    if let Some(o) = by_comment {
        return Ok(Some(o.clone()));
    }
    // A signed approval stands for the owner: with no owners it counts for no one, and any
    // owner's request for changes withdraws it. A v1 signature carries no time, so a re-posted
    // old one could pass for new: it never clears a dismissal. Each dismissed owner review needs
    // an owner v2 signature naming its id, and the timeline must show the dismissal.
    let changes = |o: &String| state(o).as_deref() == Some("CHANGES_REQUESTED");
    if !signed_ok || owners.is_empty() || owners.iter().any(changes) {
        return Ok(None);
    }
    let sign = |accept: &dyn Fn(Option<u64>) -> bool| {
        signed_approver(gh, repo, pr, head, &comments, Role::Owner, accept)
    };
    let ids: Vec<u64> = (owners.iter())
        .filter_map(|o| dismissed_review(&reviews, o, head))
        .collect();
    if ids.is_empty() {
        return Ok(sign(&|_| true)?.map(|id| format!("key {id}")));
    }
    let mut by = None;
    for id in ids {
        let seen = (gh.review_dismissed_at(pr, id))
            .map_err(|e| eprintln!("bosun: risk-check: cannot read the timeline: {}", e.message))
            .ok()
            .flatten();
        if id == 0 || seen.and_then(|t| parse_rfc3339(&t)).is_none() {
            return Ok(None);
        }
        let Some(key) = sign(&|after| after == Some(id))? else {
            return Ok(None);
        };
        by.get_or_insert(key);
    }
    Ok(by.map(|id| format!("key {id}")))
}

/// A decisive review in this state withdraws the owner's approvals (anyone with write access can
/// dismiss a review; GitHub then reports DISMISSED).
pub fn withdrawn(state: Option<&str>) -> bool {
    matches!(state, Some("CHANGES_REQUESTED" | "DISMISSED"))
}

/// When `owner`'s decisive review was dismissed (issue timeline); `None` when unknown.
fn dismissed_at(
    gh: &dyn Github,
    pr: u64,
    reviews: &[Review],
    owner: &str,
    head: &str,
) -> Option<Timestamp> {
    let id = decisive(reviews, owner, head)?.id;
    parse_rfc3339(&gh.review_dismissed_at(pr, id).ok()??)
}

/// The comments created after `since`; with no time nothing is newer (fail closed).
fn newer_than(comments: &[Comment], since: Option<Timestamp>) -> Vec<Comment> {
    let Some(since) = since else {
        return Vec::new();
    };
    comments
        .iter()
        .filter(|c| parse_rfc3339(&c.created_at).is_some_and(|t| t > since))
        .cloned()
        .collect()
}

/// Why a delegate cannot approve now, if it cannot: bosun is halted (or cannot tell), the PR
/// targets another branch than the default one, or a linked issue is not an open bosun task
/// with a lease record (a decoy link would hide the real task's capabilities and its lease
/// holders). An unreadable task issue is a reason; an error listing its comments errors the
/// check (fail closed, as every other read here).
pub(crate) fn delegation_blocked(
    gh: &dyn Github,
    base_ref: &str,
    linked: &[u64],
) -> Result<Option<String>> {
    match halt_reason(gh) {
        Ok(None) => {}
        Ok(Some((why, _))) => return Ok(Some(format!("bosun is halted ({why})"))),
        Err(e) => {
            return Ok(Some(format!(
                "cannot tell whether bosun is halted: {}",
                e.message
            )));
        }
    }
    let branch = default_branch(gh)?;
    if base_ref != branch {
        return Ok(Some(format!(
            "the PR targets {base_ref}, not the default branch {branch}"
        )));
    }
    for i in linked {
        let issue = match gh.get_issue(*i) {
            Ok(Some(issue)) => issue,
            Ok(None) => return Ok(Some(format!("linked #{i} does not exist"))),
            Err(e) => return Ok(Some(format!("cannot read linked #{i}: {}", e.message))),
        };
        if issue.pull_request || issue.state != "open" || task_meta(&issue).is_none() {
            return Ok(Some(format!("linked #{i} is not an open bosun task")));
        }
        let notes = gh.list_comments(*i)?;
        if lease_sessions(notes.iter().map(|c| c.body.as_str())).is_empty() {
            return Ok(Some(format!("linked task #{i} has no lease record")));
        }
    }
    Ok(None)
}

/// Whether a signed approval of `head` cannot count: no owners, or an owner's newest decisive
/// review on `head` requests changes or was dismissed.
fn owners_withdrawn(gh: &dyn Github, pr: u64, head: &str, owners: &[String]) -> Result<bool> {
    let reviews = gh.list_reviews(pr)?;
    Ok(owners.is_empty()
        || owners
            .iter()
            .any(|o| withdrawn(decisive_review(&reviews, o, head).as_deref())))
}

/// The PR's comments oldest first, and the sessions that cannot review it independently: every
/// session with a lease record (past or live) on a linked issue. `bosun pr` records no session
/// for the PR author, so the lease is the only trace of who wrote the change.
pub fn verdict_inputs(gh: &dyn Github, n: u64, ls: &[u64]) -> Result<(Vec<Comment>, Vec<String>)> {
    let mut comments = gh.list_comments(n)?;
    comments.sort_by_key(|c| c.id);
    let mut excluded = Vec::new();
    for i in ls {
        let notes = gh.list_comments(*i)?;
        excluded.extend(lease_sessions(notes.iter().map(|c| c.body.as_str())));
    }
    Ok((comments, excluded))
}

/// The id of an enrolled key that signed an approval of `head` on this PR (spec §8, signed
/// approvals); `accept` takes the review a v2 names (`None`: v1). Only an owner key signs v2.
/// The keys come from the default branch only, so a PR cannot enroll its own. Any author and app
/// count: the signature is the proof. A missing or malformed key file, an unknown key or a bad
/// signature is no approval, logged to stderr, never an error.
fn signed_approver(
    gh: &dyn Github,
    repo: &RepoId,
    pr: u64,
    head: &str,
    comments: &[Comment],
    role: Role,
    accept: &dyn Fn(Option<u64>) -> bool,
) -> Result<Option<String>> {
    let sigs: Vec<(u64, Option<u64>, &str, &str)> = comments
        .iter()
        .flat_map(|c| {
            signature_lines(&c.body, head)
                .into_iter()
                .map(move |(after, id, sig)| (c.id, after, id, sig))
        })
        .collect();
    if sigs.is_empty() {
        return Ok(None);
    }
    let keys = match enrolled_keys(gh) {
        Ok(keys) => keys,
        Err(why) => {
            eprintln!("bosun: risk-check: signed approvals are off: {why}");
            return Ok(None);
        }
    };
    for (comment, after, id, sig) in sigs {
        let msg = match after {
            None => signed_message(role, &repo.owner, &repo.name, pr, head),
            Some(r) if role == Role::Owner => {
                signed_message_v2(&repo.owner, &repo.name, pr, head, r)
            }
            Some(_) => continue,
        };
        if !accept(after) {
            continue;
        }
        let Some(key) = keys.iter().find(|k| k.id == id) else {
            eprintln!("bosun: risk-check: comment {comment}: unknown key {id:?}, not counted");
            continue;
        };
        if key.role != role {
            eprintln!(
                "bosun: risk-check: comment {comment}: key {id:?} is a {} key, not counted as {}",
                key.role.name(),
                role.name()
            );
            continue;
        }
        let ok = b64::decode(&key.ed25519)
            .zip(b64::decode(sig))
            .is_some_and(|(k, s)| verify_ed25519(&k, msg.as_bytes(), &s));
        if ok {
            return Ok(Some(key.id.clone()));
        }
        eprintln!(
            "bosun: risk-check: comment {comment}: signature of key {id:?} does not verify for {msg:?}, not counted"
        );
    }
    Ok(None)
}

/// The keys of `.bosun/approvers.toml` on the default branch, read at `refs/heads/<branch>` so
/// a tag or another ref of the same name cannot stand in for it. `Err` says why signed
/// approvals are off: the file is missing, unreadable or malformed (a key that is not 32 bytes
/// of base64 makes the whole file malformed). Never an error of the check.
pub fn enrolled_keys(gh: &dyn Github) -> std::result::Result<Vec<ApproverKey>, String> {
    let git_ref = default_ref(gh).map_err(|e| format!("no default branch: {}", e.message))?;
    let text = match gh.file_at(APPROVERS_PATH, &git_ref) {
        Ok(Some(text)) => text,
        Ok(None) => return Err(format!("{APPROVERS_PATH} is missing on {git_ref}")),
        Err(e) => return Err(format!("cannot read {APPROVERS_PATH}: {}", e.message)),
    };
    let keys = parse_approvers(&text)
        .map_err(|e| format!("{APPROVERS_PATH} on {git_ref} is malformed: {e}"))?;
    let raw: Vec<Option<Vec<u8>>> = keys.iter().map(|k| b64::decode(&k.ed25519)).collect();
    if let Some(k) = keys
        .iter()
        .zip(&raw)
        .enumerate()
        .find(|(i, (_, r))| r.is_some() && raw[..*i].contains(r))
        .map(|(_, (k, _))| k)
    {
        return Err(format!(
            "{APPROVERS_PATH} on {git_ref} is malformed: the public key of {:?} is enrolled twice",
            k.id
        ));
    }
    match keys
        .iter()
        .find(|k| b64::decode(&k.ed25519).is_none_or(|b| b.len() != 32))
    {
        Some(k) => Err(format!(
            "{APPROVERS_PATH} on {git_ref} is malformed: key {:?} is not a base64 32-byte ed25519 key",
            k.id
        )),
        None => Ok(keys),
    }
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
