//! `bosun result`: a child hands its result to its parent (spec D14).

use std::path::Path;

use serde_json::json;

use super::ResultArgs;
use crate::app::{App, end_lease, notes};
use crate::core::lease::{leases_of, members};
use crate::core::records::Record;
use crate::output::{BosunError, CmdResult, Outcome};
use crate::registry;

pub fn run(dir: Option<&Path>, args: &ResultArgs) -> CmdResult {
    // No kill switch check: reporting a result is how a halted child hands back.
    let app = App::load(dir)?;
    let (gh, n, session) = (app.gh.as_ref(), args.epic, &app.ctx.session);
    if gh.get_issue(n)?.is_none() {
        return Err(BosunError::error(
            format!("epic #{n} does not exist"),
            "check the epic number; it is the issue in your prompt",
        ));
    }

    let pr_text = args.pr.map_or(String::new(), |p| format!(", PR #{p}"));
    let record = Record::Result {
        session: session.clone(),
        outcome: args.outcome.clone(),
        pr: args.pr,
    };
    let body = format!(
        "Result of {session}: {}{pr_text}\n\n{}",
        args.outcome,
        record.to_line()
    );
    let comment_id = gh.add_comment(n, &body)?.id;

    let lease_ended = leases_of(n, &notes(gh, n)?)
        .iter()
        .any(|l| &l.session == session);
    if lease_ended {
        end_lease(gh, n, session, "result")?;
    }

    let mut left = false;
    if let Some((number, reg)) = registry::load(gh)?
        && members(&reg).iter().any(|m| &m.session == session)
    {
        let leave = Record::Leave {
            session: session.clone(),
            reason: "done".into(),
        };
        gh.add_comment(number, &leave.to_line())?;
        left = true;
    }
    Ok(Outcome::data(json!({
        "epic": n, "outcome": args.outcome, "pr": args.pr,
        "comment_id": comment_id, "lease_ended": lease_ended, "left": left,
    })))
}
