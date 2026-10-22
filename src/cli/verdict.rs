//! `bosun verdict post`: record the reviewer's verdict as the `review-verdict` status (spec §7).

use std::io::Read;
use std::path::Path;

use serde_json::json;

use super::{FromArg, VerdictCmd};
use crate::app::{App, kill_switch, set_state};
use crate::core::records::Record;
use crate::core::state::{State, state_of, transition_allowed};
use crate::core::verdict::{Verdict, closes_issue, effective};
use crate::output::{BosunError, CmdResult, Outcome};

pub const CONTEXT: &str = "review-verdict";

fn read_input(from: &str) -> Result<String, BosunError> {
    let mut text = String::new();
    let res = if from == "-" {
        std::io::stdin().read_to_string(&mut text).map(|_| ())
    } else {
        std::fs::read_to_string(from).map(|t| text = t)
    };
    res.map_err(|e| {
        BosunError::usage(
            format!("cannot read the verdict from {from}: {e}"),
            "pass the reviewer's JSON on stdin or with --from FILE",
        )
    })?;
    Ok(text)
}

pub fn run(dir: Option<&Path>, cmd: &VerdictCmd) -> CmdResult {
    let VerdictCmd::Post(FromArg { from }) = cmd;
    let app = App::load(dir)?;
    let gh = app.gh.as_ref();
    kill_switch(gh)?;
    let verdict = Verdict::parse(&read_input(from)?).map_err(|m| {
        BosunError::usage(
            format!("invalid verdict: {m}"),
            "return the JSON shape in spec §7 (see `bosun review-pack` instructions)",
        )
    })?;
    let n = verdict.pr;
    let pull = gh.get_pull(n)?.ok_or_else(|| {
        BosunError::error(
            format!("PR #{n} does not exist"),
            "check the number in the verdict",
        )
    })?;
    if pull.head_sha != verdict.head_sha {
        return Err(BosunError::conflict(
            format!(
                "stale verdict for {}; PR #{n} is now at {}",
                verdict.head_sha, pull.head_sha
            ),
            format!("review the new head: run `bosun review-pack {n}` again"),
        ));
    }
    let mut eff = effective(&verdict);
    if verdict.escalate_risk {
        // An escalation is for the owner: never a green verdict.
        eff.approve = false;
        eff.reasons.push("escalated to the owner".into());
    }
    let state = if eff.approve { "success" } else { "failure" };
    let description = if verdict.escalate_risk {
        format!("escalated to the owner; {}", verdict.summary())
    } else {
        verdict.summary()
    };
    gh.create_status(&verdict.head_sha, CONTEXT, state, &description, None)?;
    let record = Record::Verdict {
        session: app.ctx.session.clone(),
        head_sha: verdict.head_sha.clone(),
        approved: eff.approve,
    };
    let human = format!("Review verdict: {}.", verdict.summary());
    gh.add_comment(n, &format!("{human}\n\n{}", record.to_line()))?;

    let target = if verdict.escalate_risk {
        Some(State::NeedsHuman)
    } else if !eff.approve {
        Some(State::Fixing)
    } else {
        None
    };
    let mut task_state = None;
    if let (Some(to), Some(issue_no)) = (target, closes_issue(&pull.body))
        && let Some(issue) = gh.get_issue(issue_no)?
        && state_of(&issue.labels).is_none_or(|from| transition_allowed(from, to))
    {
        set_state(gh, issue_no, &issue, to)?;
        task_state = Some(to.name());
    }
    Ok(Outcome::data(json!({
        "pr": n, "head_sha": verdict.head_sha, "approved": eff.approve,
        "reasons": eff.reasons, "status": state, "task_state": task_state,
    })))
}
