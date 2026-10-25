//! `bosun result`: a child hands its result to its parent (spec D14).

use std::path::Path;

use serde_json::json;

use super::ResultArgs;
use crate::app::{App, end_lease, notes, set_state, sync_merged};
use crate::core::lease::{leases_of, members};
use crate::core::meta::linked_issues;
use crate::core::records::Record;
use crate::core::state::{State, state_of, transition_allowed};
use crate::output::{BosunError, CmdResult, Outcome};
use crate::registry;

pub fn run(dir: Option<&Path>, args: &ResultArgs) -> CmdResult {
    // No kill switch check: reporting a result is how a halted child hands back.
    let app = App::load(dir)?;
    let (gh, n, session) = (app.gh.as_ref(), args.epic, &app.ctx.session);
    let Some(epic) = gh.get_issue(n)? else {
        return Err(BosunError::error(
            format!("epic #{n} does not exist"),
            "check the epic number; it is the issue in your prompt",
        ));
    };

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

    // `merged` is only a report: the merge itself moves the state (D13). The other outcomes set
    // their state, as this ends the lease and nothing else would move the epic (spec §5).
    let mut state = state_of(&epic.labels);
    let to = match args.outcome.as_str() {
        "blocked" => Some(State::Blocked),
        "needs-human" => Some(State::NeedsHuman),
        _ => None,
    };
    if let Some(to) = to
        && state != Some(to)
        && state.is_none_or(|from| transition_allowed(from, to))
    {
        set_state(gh, n, &epic, to)?;
        state = Some(to);
    }

    // A merged PR moves the tasks it closes (the merge moves state, D13); an unmerged one is a report.
    let mut tasks_merged = Vec::new();
    if args.outcome == "merged"
        && let Some(pr) = args.pr
        && let Some(pull) = gh.get_pull(pr)?
        && pull.merged
    {
        for task in linked_issues(&pull.body) {
            if let Some(issue) = gh.get_issue(task)?
                && sync_merged(gh, task, &issue, Some(session))?
            {
                tasks_merged.push(task);
            }
        }
    }

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
        "task_state": state.map(State::name), "tasks_merged": tasks_merged,
    })))
}
