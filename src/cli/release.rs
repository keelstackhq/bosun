//! `bosun release`: end this session's lease on a task and move the task on.

use std::path::Path;

use serde_json::json;

use super::ReleaseArgs;
use crate::app::{App, end_lease, notes, set_state};
use crate::core::lease::leases_of;
use crate::core::state::State;
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
    if !leases_of(n, &notes(gh, n)?)
        .iter()
        .any(|l| &l.session == session)
    {
        return Err(BosunError::conflict(
            format!("no lease of {session} on #{n}"),
            "only the session that claimed a task can release it; `bosun status` shows the holder",
        ));
    }
    let comment_id = end_lease(gh, n, session, "released")?;
    set_state(gh, n, &issue, to)?;
    Ok(Outcome::data(
        json!({"issue": n, "state": to.name(), "comment_id": comment_id}),
    ))
}
