//! `bosun release`: end this session's lease on a task and move the task on.

use std::path::Path;

use serde_json::json;

use super::ReleaseArgs;
use crate::app::{App, Result, end_lease, notes, set_state};
use crate::core::lease::leases_of;
use crate::core::records::Note;
use crate::core::state::State;
use crate::github::{Github, Issue};
use crate::output::{BosunError, CmdResult, Outcome};

pub fn run(dir: Option<&Path>, args: &ReleaseArgs) -> CmdResult {
    let app = App::load(dir)?;
    let (gh, n, session) = (app.gh.as_ref(), args.issue, &app.ctx.session);
    let to: State = args
        .to
        .parse()
        .map_err(|m: String| BosunError::usage(m, "use --to ready, blocked or review"))?;
    let issue = gh.get_issue(n)?.ok_or_else(|| {
        BosunError::error(
            format!("issue #{n} does not exist"),
            "check the number; `bosun status` lists the tasks",
        )
    })?;
    let comment_id = release_to(gh, n, &issue, session, &notes(gh, n)?, to)?;
    Ok(Outcome::data(
        json!({"issue": n, "state": to.name(), "comment_id": comment_id}),
    ))
}

/// Conflict (exit 3) unless `session` holds a lease on `n` in `notes`.
pub(crate) fn require_lease(n: u64, session: &str, notes: &[Note]) -> Result<()> {
    if leases_of(n, notes).iter().any(|l| l.session == session) {
        return Ok(());
    }
    Err(BosunError::conflict(
        format!("no lease of {session} on #{n}"),
        "only the session that claimed a task can release it; `bosun status` shows the holder",
    ))
}

/// End the lease of `session` on `n` (`notes` are its comments) and move the task to `to`.
/// Returns the id of the `lease-end` comment.
pub(crate) fn release_to(
    gh: &dyn Github,
    n: u64,
    issue: &Issue,
    session: &str,
    notes: &[Note],
    to: State,
) -> Result<u64> {
    require_lease(n, session, notes)?;
    let comment_id = end_lease(gh, n, session, "released")?;
    set_state(gh, n, issue, to)?;
    Ok(comment_id)
}
