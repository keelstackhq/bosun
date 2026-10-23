//! `bosun fail <issue> --signature S [--changed-lines N]`: record a failed fix round and apply
//! the stop rules (spec §4). A stop sets `blocked` and escalates one model tier; with opus used
//! up it sets `needs-human` (exit 5). The signature is in `data` for the lessons (M5).

use std::path::Path;

use serde_json::json;

use super::FailArgs;
use super::lesson::capture_lesson;
use crate::app::{App, kill_switch, limits_from_default, notes, set_state};
use crate::core::records::Record;
use crate::core::state::{State, state_of};
use crate::core::stop::{current_tier, evaluate, next_tier, streak, tier_index};
use crate::github::Github;
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
        add_lesson(&mut data, gh, &app.ctx.session, n);
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
    add_lesson(&mut data, gh, &app.ctx.session, n);
    Ok(Outcome::data(data))
}

/// Open or hit the `stop-rule` lesson for this stop (spec §4, §6). The text holds only the rule
/// and the signature, so the same stop on another task hits the same lesson. A failure here must
/// not hide the stop: it becomes `lesson: null` and `lesson_error`.
fn add_lesson(data: &mut serde_json::Value, gh: &dyn Github, session: &str, n: u64) {
    let rule = data["rule"].as_str().unwrap_or_default().to_string();
    let sig = data["signature"].as_str().unwrap_or_default().to_string();
    // GitHub caps issue titles at 256 characters; the full signature stays in the text.
    let short: String = sig.chars().take(120).collect();
    let title = format!("Stop rule {rule} on #{n}: {short}");
    let text = format!("Stop rule {rule}\nFailure signature: {sig}");
    match capture_lesson(gh, session, "stop-rule", &title, &text, Some(n)) {
        Ok(c) => {
            data["lesson"] = json!({"issue": c.issue, "created": c.created, "hits": c.hits});
        }
        Err(e) => {
            eprintln!("bosun: warning: lesson capture failed: {}", e.message);
            data["lesson"] = serde_json::Value::Null;
            data["lesson_error"] = json!(e.message);
        }
    }
}
