//! `bosun heartbeat`: push the expiry of this session's leases and join record to now + TTL.
//! Everything is edited in place; no comment is added.

use std::path::Path;

use serde_json::json;

use crate::app::{App, active_issues, kill_switch, renew_lease};
use crate::core::lease::{TTL_SECS, leases_of, members};
use crate::output::{CmdResult, Outcome};
use crate::registry;

pub fn run(dir: Option<&Path>) -> CmdResult {
    let app = App::load(dir)?;
    let gh = app.gh.as_ref();
    kill_switch(gh)?;
    let (session, now) = (&app.ctx.session, app.ctx.now);
    let expires = now.plus_secs(TTL_SECS);

    let mut extended = Vec::new();
    for (issue, notes) in active_issues(gh)? {
        for lease in leases_of(issue.number, &notes) {
            if &lease.session != session || !lease.is_live(now) {
                continue;
            }
            renew_lease(gh, &lease, &notes, expires)?;
            extended.push(issue.number);
        }
    }

    let mut registered = false;
    if let Some((_, notes)) = registry::load(gh)?
        && let Some(mut m) = members(&notes).into_iter().find(|m| &m.session == session)
    {
        m.expires = expires;
        let body = notes
            .iter()
            .find(|n| n.id == m.comment_id)
            .map_or("", |n| n.body.as_str());
        registry::rewrite(gh, &m, body)?;
        registered = true;
    }
    Ok(Outcome::data(
        json!({"leases": extended, "registry": registered}),
    ))
}
