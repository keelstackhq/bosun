//! The registry: the one open issue labelled `bosun:registry`; each session has one `join`
//! comment there that heartbeats edit in place (spec §4).

use crate::app::{App, Result, notes};
use crate::core::lease::{Member, members};
use crate::core::records::{Note, Record, replace_record};
use crate::ctx::Origin;
use crate::github::{Github, IssueState};
use crate::output::BosunError;

pub const LABEL: &str = "bosun:registry";

/// The registry issue and its comments; `None` before the first join.
pub fn load(gh: &dyn Github) -> Result<Option<(u64, Vec<Note>)>> {
    match gh.list_issues(&[LABEL], IssueState::Open)?.first() {
        Some(i) => Ok(Some((i.number, notes(gh, i.number)?))),
        None => Ok(None),
    }
}

pub fn ensure(gh: &dyn Github) -> Result<(u64, Vec<Note>)> {
    if let Some(found) = load(gh)? {
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
    // A record that names itself as parent (written before this check) is a root.
    let parent = prior
        .and_then(|m| m.parent.clone())
        .filter(|p| p != session)
        .or(spec.parent);
    // An unknown or expired parent keeps an existing record's depth as the floor.
    let floor = match parent.as_deref() {
        None => 1,
        Some(p) => members(&notes)
            .iter()
            .find(|m| m.session == p && m.expires > app.ctx.now)
            .map_or(prior.map_or(1, |m| m.depth), |m| m.depth + 1),
    };
    let member = Member {
        session: session.clone(),
        origin: spec.origin.unwrap_or(app.ctx.origin).as_str().to_string(),
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
    }
}

/// Edit a member's `join` comment (`body` is its current text) to expire at its `expires`.
pub fn rewrite(gh: &dyn Github, m: &Member, body: &str) -> Result<()> {
    gh.update_comment(m.comment_id, &replace_record(body, &record_of(m)))?;
    Ok(())
}
