//! The registry: one issue labelled `bosun:registry`, pinned by `.bosun/registry` on the
//! default branch or else the lowest-numbered labelled issue in any state; each session has one
//! `join` comment there that heartbeats edit in place (spec §4).

use crate::app::{App, Result, trusted_notes};
use crate::core::lease::{Member, members};
use crate::core::records::{Note, Record, SlackSeat, replace_record};
use crate::ctx::Origin;
use crate::github::{Github, IssueState};
use crate::output::BosunError;

pub const LABEL: &str = "bosun:registry";

/// The registry issue and its writers' comments; `None` before the first join.
pub fn load(gh: &dyn Github) -> Result<Option<(u64, Vec<Note>)>> {
    load_with(gh, true)
}

/// As [`load`], with every comment. Only for a session's own records (`spawn --record` finds
/// its own join to edit in place): it must work when the permission lookup fails, and a forged
/// join under its id only makes the in-place edit fail.
pub fn load_own(gh: &dyn Github) -> Result<Option<(u64, Vec<Note>)>> {
    load_with(gh, false)
}

/// Where the pin file lives, on the default branch.
pub const PIN_FILE: &str = ".bosun/registry";

/// The registry issue number, the one choice every reader and writer shares (read once per
/// process): the number in `.bosun/registry` on the default branch, which is the trust root and
/// names an issue in any state, else the lowest-numbered issue with the label in any state. A
/// newer labelled issue never counts. Fails closed when the default branch cannot be read.
pub fn pick(gh: &dyn Github) -> Result<Option<u64>> {
    static CACHE: std::sync::OnceLock<u64> = std::sync::OnceLock::new();
    if let Some(n) = CACHE.get() {
        return Ok(Some(*n));
    }
    let picked = pick_uncached(gh)?;
    if let Some(n) = picked {
        let _ = CACHE.set(n);
    }
    Ok(picked)
}

fn pick_uncached(gh: &dyn Github) -> Result<Option<u64>> {
    let git_ref = crate::app::default_ref(gh)?;
    if let Some(text) = gh.file_at(PIN_FILE, &git_ref)? {
        let bad = |why: String| {
            BosunError::error(
                format!("{PIN_FILE} on {git_ref} {why}"),
                format!(
                    "the owner fixes {PIN_FILE} on the default branch (an R-gov PR) to hold the number of the registry issue"
                ),
            )
        };
        let n: u64 = text
            .trim()
            .parse()
            .map_err(|_| bad(format!("is not an issue number: {:?}", text.trim())))?;
        return match gh.get_issue(n)? {
            Some(_) => Ok(Some(n)),
            None => Err(bad(format!("names #{n}, which does not exist"))),
        };
    }
    Ok(labelled(gh)?.into_iter().min())
}

fn labelled(gh: &dyn Github) -> Result<Vec<u64>> {
    Ok(gh
        .list_issues(&[LABEL], IssueState::All)?
        .iter()
        .map(|i| i.number)
        .collect())
}

/// What `bosun doctor` warns about: no committed pin (the fallback can be swapped by relabelling
/// issues), or several labelled issues. `None` when the pin file exists.
pub fn pin_advice(gh: &dyn Github) -> Result<Option<String>> {
    let git_ref = crate::app::default_ref(gh)?;
    if gh.file_at(PIN_FILE, &git_ref)?.is_some() {
        return Ok(None);
    }
    let n = labelled(gh)?.len();
    Ok(Some(format!(
        "no {PIN_FILE} on the default branch ({n} issue(s) labelled {LABEL}): the registry is the lowest-numbered labelled issue, which relabelling can swap; commit {PIN_FILE} with the registry issue number (an R-gov PR the owner approves)"
    )))
}

fn load_with(gh: &dyn Github, trusted: bool) -> Result<Option<(u64, Vec<Note>)>> {
    match pick(gh)? {
        Some(n) if trusted => Ok(Some((n, trusted_notes(gh, n)?))),
        Some(n) => Ok(Some((n, crate::app::notes(gh, n)?))),
        None => Ok(None),
    }
}

/// The registry comments written by admin or write users; empty before the first join. Spend
/// is summed only from these: a forged `spend` record by anyone else must not stop the agents.
pub fn trusted_spend_notes(gh: &dyn Github) -> Result<Vec<Note>> {
    match pick(gh)? {
        Some(n) => crate::app::spend_notes(gh, n),
        None => Ok(Vec::new()),
    }
}

pub fn ensure(gh: &dyn Github) -> Result<(u64, Vec<Note>)> {
    ensure_with(gh, true)
}

/// As [`ensure`], with every comment (see [`load_own`]).
pub fn ensure_own(gh: &dyn Github) -> Result<(u64, Vec<Note>)> {
    ensure_with(gh, false)
}

fn ensure_with(gh: &dyn Github, trusted: bool) -> Result<(u64, Vec<Note>)> {
    if let Some(found) = load_with(gh, trusted)? {
        return Ok(found);
    }
    gh.ensure_label(LABEL, "5319e7", "bosun session registry")?;
    let issue = gh.create_issue(
        "bosun registry",
        "Sessions register here with one `join` comment each; bosun edits it in place.",
        &[LABEL],
    )?;
    Ok((issue.number, Vec::new()))
}

/// What a session asks for in `join`; `None` keeps the value of an earlier join.
#[derive(Debug, Clone, Default)]
pub struct JoinSpec {
    pub origin: Option<Origin>,
    pub area: Option<String>,
    pub parent: Option<String>,
    pub depth: Option<u32>,
    pub slack: Option<SlackSeat>,
}

/// Post this session's `join` record, or edit its existing one in place. Returns the registry
/// issue and the member as written.
pub fn join(app: &App, spec: JoinSpec) -> Result<(u64, Member)> {
    let (issue, notes) = ensure(app.gh.as_ref())?;
    let session = &app.ctx.session;
    let mine = members(&notes).into_iter().find(|m| &m.session == session);
    let expires = app.ctx.now.plus_secs(crate::core::lease::TTL_SECS);
    let prior = mine.as_ref();
    if spec.parent.as_ref() == Some(session) {
        return Err(BosunError::error(
            format!("--parent {session} is this session itself"),
            "pass the id of the session that spawned this one, or leave out --parent for a root session",
        ));
    }
    // A record that names itself as parent (written before this check) loses that parent but
    // keeps its own depth as the floor, like an unknown parent: it is never reset to a root.
    let self_parented = prior.and_then(|m| m.parent.as_deref()) == Some(session.as_str());
    let parent = prior
        .and_then(|m| m.parent.clone())
        .filter(|p| p != session)
        .or(spec.parent);
    // An unknown or expired parent keeps an existing record's depth as the floor.
    let floor = match parent.as_deref() {
        None if self_parented => prior.map_or(1, |m| m.depth),
        None => 1,
        Some(p) => members(&notes)
            .iter()
            .find(|m| m.session == p && m.expires > app.ctx.now)
            .map_or(prior.map_or(1, |m| m.depth), |m| m.depth + 1),
    };
    // A child or routine session cannot relabel itself as Slack.
    if spec.origin == Some(Origin::Slack)
        && let Some(o) = prior
            .map(|m| m.origin.as_str())
            .filter(|o| matches!(*o, "spawned" | "routine"))
    {
        return Err(BosunError::needs_human(
            format!("this session's join record says origin {o}; it cannot join as slack"),
            "join without --origin slack; only a Claude Tag session joins as slack",
        ));
    }
    // A plain re-join of a slack session keeps origin slack; the seat survives only there.
    let origin = match (spec.origin, prior) {
        (Some(o), _) => o.as_str().to_string(),
        (None, Some(m)) if m.origin == "slack" => m.origin.clone(),
        (None, _) => app.ctx.origin.as_str().to_string(),
    };
    let slack = spec
        .slack
        .or_else(|| prior.and_then(|m| m.slack.clone()))
        .filter(|_| origin == "slack");
    let member = Member {
        session: session.clone(),
        origin,
        area: spec.area.or_else(|| prior.and_then(|m| m.area.clone())),
        // The parent is fixed at the first join (`spawn --record` writes it). A later `--depth`
        // sets the depth, but never below its parent's depth + 1 (1 for a root), so a root that
        // joined too deep can come back to 1 and a child can never spawn deeper than allowed.
        parent: parent.clone(),
        depth: match (spec.depth, prior) {
            (Some(d), _) => d.max(floor),
            (None, Some(m)) => m.depth,
            (None, None) => 1.max(floor),
        },
        expires,
        slack,
        comment_id: prior.map_or(0, |m| m.comment_id),
    };
    let comment_id = write(app.gh.as_ref(), issue, &notes, prior, &member)?;
    Ok((
        issue,
        Member {
            comment_id,
            ..member
        },
    ))
}

/// Post a member's `join` record, or edit `prior` (its existing comment) in place.
pub fn write(
    gh: &dyn Github,
    issue: u64,
    notes: &[Note],
    prior: Option<&Member>,
    member: &Member,
) -> Result<u64> {
    let record = record_of(member);
    Ok(match prior {
        Some(m) => {
            let body = notes
                .iter()
                .find(|n| n.id == m.comment_id)
                .map_or_else(String::new, |n| n.body.clone());
            gh.update_comment(m.comment_id, &replace_record(&body, &record))?
                .id
        }
        None => gh.add_comment(issue, &record.to_line())?.id,
    })
}

fn record_of(m: &Member) -> Record {
    Record::Join {
        session: m.session.clone(),
        origin: m.origin.clone(),
        area: m.area.clone(),
        parent: m.parent.clone(),
        depth: m.depth,
        expires: m.expires.to_string(),
        slack: m.slack.clone(),
    }
}

/// Edit a member's `join` comment (`body` is its current text) to expire at its `expires`.
pub fn rewrite(gh: &dyn Github, m: &Member, body: &str) -> Result<()> {
    gh.update_comment(m.comment_id, &replace_record(body, &record_of(m)))?;
    Ok(())
}

/// The live Claude Tag seat of `session`, from join records written by users with write access
/// only; an expired or forged join gives none. Read-only.
pub fn seat_of(
    gh: &dyn Github,
    session: &str,
    now: crate::time::Timestamp,
) -> Result<Option<SlackSeat>> {
    let Some(issue) = pick(gh)? else {
        return Ok(None);
    };
    Ok(members(&trusted_notes(gh, issue)?)
        .into_iter()
        .find(|m| m.session == session && m.expires > now)
        .and_then(|m| m.slack))
}
