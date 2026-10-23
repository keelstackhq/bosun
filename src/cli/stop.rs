//! `bosun fail <issue> --signature S [--changed-lines N]`: record a failed fix round and apply
//! the stop rules (spec §4). A stop sets `blocked` and escalates one model tier; with opus used
//! up it sets `needs-human` (exit 5). The signature is in `data` for the lessons (M5).

use std::path::Path;

use serde_json::json;

use super::FailArgs;
use crate::app::{App, kill_switch, limits_from_default, notes, set_state};
use crate::core::records::Record;
use crate::core::state::{State, state_of};
use crate::core::stop::{current_tier, evaluate, next_tier, streak, tier_index};
use crate::output::{BosunError, CmdResult, Outcome};

pub fn run(dir: Option<&Path>, args: &FailArgs) -> CmdResult {
    let app = App::load(dir)?;
    let (gh, n) = (app.gh.as_ref(), args.issue);
    kill_switch(gh)?;
    let signature = args.signature.trim();
    if signature.is_empty() {
        return Err(BosunError::usage(
            "the failure signature is empty",
            "pass a short stable text, for example --signature \"E0308 in src/x.rs\"",
        ));
    }
    let issue = gh.get_issue(n)?.ok_or_else(|| {
        BosunError::error(
            format!("issue #{n} does not exist"),
            "check the number; `bosun status` lists the tasks",
        )
    })?;
    // Check the state before writing, so a refused call leaves no comment behind.
    if matches!(state_of(&issue.labels), None | Some(State::Merged)) {
        return Err(BosunError::conflict(
            format!("#{n} is not an open task"),
            "record failures only on a task that is not merged; `bosun status` lists the tasks",
        ));
    }
    let record = Record::Failure {
        session: app.ctx.session.clone(),
        signature: signature.to_string(),
        changed_lines: args.changed_lines,
    };
    gh.add_comment(
        n,
        &format!("Failed fix round: {signature}\n\n{}", record.to_line()),
    )?;
    let all = notes(gh, n)?;
    let Some(stop) = evaluate(&all) else {
        let (_, count) = streak(&all);
        return Ok(Outcome::data(
            json!({"issue": n, "stopped": false, "streak": count, "signature": signature}),
        ));
    };
    let limits = limits_from_default(gh, &app.ctx)?;
    // `--model` never goes below the tier of the newest escalation.
    let build = &limits.models.build;
    let recorded = current_tier(&all, build);
    let tier = match &args.model {
        Some(m) if tier_index(m, build) >= tier_index(&recorded, build) => m.clone(),
        _ => recorded,
    };
    let mut data = json!({
        "issue": n, "stopped": true, "rule": stop.rule,
        "signature": stop.signature, "streak": stop.streak,
    });
    let Some(to) = next_tier(&tier, &limits.models.build) else {
        set_state(gh, n, &issue, State::NeedsHuman)?;
        data["state"] = json!("needs-human");
        return Err(BosunError::needs_human(
            format!(
                "#{n} stopped by rule {} and escalation is used up",
                stop.rule
            ),
            format!("the owner must look at #{n}; escalation is used up"),
        )
        .with_data(data));
    };
    set_state(gh, n, &issue, State::Blocked)?;
    let esc = Record::Escalate {
        session: app.ctx.session.clone(),
        from: tier.clone(),
        to: to.to_string(),
        reason: stop.rule.to_string(),
    };
    gh.add_comment(
        n,
        &format!(
            "Blocked by rule {}: escalating {tier} to {to}.\n\n{}",
            stop.rule,
            esc.to_line()
        ),
    )?;
    data["state"] = json!("blocked");
    data["escalate"] = json!({"from": tier, "to": to});
    Ok(Outcome::data(data))
}
