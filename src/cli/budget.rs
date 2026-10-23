//! `bosun budget record`: write the spend of a task to its issue and the registry (spec §9).

use std::io::Read;
use std::path::Path;

use serde_json::{Value, json};

use super::lesson::capture_lesson;
use super::{BudgetCmd, BudgetRecordArgs};
use crate::app::{App, Result, kill_switch, limits_from_default, notes, set_state};
use crate::core::records::{Record, spend_total};
use crate::core::state::{State, state_of};
use crate::output::{BosunError, CmdResult, Outcome};
use crate::registry;
use crate::sessions::usage;

const LESSON_TITLE: &str = "a task went over per_task_usd";
const LESSON_TEXT: &str = "a task went over its per_task_usd cap and was blocked";

pub fn run(dir: Option<&Path>, cmd: &BudgetCmd) -> CmdResult {
    match cmd {
        BudgetCmd::Record(a) => record(dir, a),
    }
}

fn read_input(from: &str) -> Result<Value> {
    let mut text = String::new();
    let read = if from == "-" {
        std::io::stdin().read_to_string(&mut text).map(|_| ())
    } else {
        std::fs::read_to_string(from).map(|t| text = t)
    };
    read.map_err(|e| {
        BosunError::error(
            format!("cannot read {from}: {e}"),
            "pass the get_session result on stdin with --from -, or a file path",
        )
    })?;
    serde_json::from_str(&text).map_err(|e| {
        BosunError::error(
            format!("the get_session result is not JSON: {e}"),
            "pass the unchanged result of the get_session tool",
        )
    })
}

fn usd_of(
    a: &BudgetRecordArgs,
    prices: &std::collections::BTreeMap<String, crate::config::Price>,
) -> Result<f64> {
    if a.provider == "claude-sessions" {
        let from = a.from.as_deref().ok_or_else(|| {
            BosunError::usage(
                "--provider claude-sessions needs --from",
                "pass the get_session result with --from -, or use --provider generic --turns N --model M",
            )
        })?;
        return usage::from_claude_session(&read_input(from)?);
    }
    match (a.turns, a.model.as_deref()) {
        (Some(t), Some(m)) => usage::generic(t, m, prices),
        _ => Err(BosunError::usage(
            "--provider generic needs --turns and --model",
            "use --provider generic --turns N --model M",
        )),
    }
}

fn record(dir: Option<&Path>, a: &BudgetRecordArgs) -> CmdResult {
    let app = App::load(dir)?;
    let (gh, ctx) = (app.gh.as_ref(), &app.ctx);
    kill_switch(gh)?;
    let usd = usd_of(a, &ctx.config.prices)?;
    if !usd.is_finite() || usd < 0.0 {
        return Err(BosunError::error(
            format!("the cost {usd} is negative or not a number"),
            "pass the unchanged get_session result, or use --provider generic --turns N --model M",
        ));
    }
    let limits = limits_from_default(gh, ctx)?;
    let n = a.issue;
    let issue = gh.get_issue(n)?.ok_or_else(|| {
        BosunError::error(
            format!("issue #{n} does not exist"),
            "check the number; `bosun status` lists the tasks",
        )
    })?;
    let spend = Record::Spend {
        session: ctx.session.clone(),
        usd,
        at: ctx.now.to_string(),
        issue: Some(n),
    };
    gh.add_comment(n, &spend.to_line())?;
    let (registry_issue, _) = registry::ensure(gh)?;
    gh.add_comment(registry_issue, &spend.to_line())?;

    let total = spend_total(&notes(gh, n)?, None);
    let over = total > limits.budget.per_task_usd;
    let was_blocked = state_of(&issue.labels) == Some(State::Blocked);
    let blocked = over && set_state(gh, n, &issue, State::Blocked).is_ok();
    // One lesson for "a task went over its cap" (the amounts and the task number do not enter
    // the signature: each task adds a hit). Only the record that blocks the task captures, so
    // later records on a blocked task add no hit; a task blocked for another reason first, or
    // one that cannot move to blocked, gets no budget lesson. The spend is already recorded, so
    // a failed capture must not fail the command: a retry would record the spend twice.
    let lesson = if blocked && !was_blocked {
        match capture_lesson(
            gh,
            &ctx.session,
            "budget",
            LESSON_TITLE,
            LESSON_TEXT,
            Some(n),
        ) {
            Ok(c) => json!({"issue": c.issue, "created": c.created, "hits": c.hits}),
            Err(e) => {
                eprintln!(
                    "bosun: warning: the budget lesson for #{n} was not captured: {}",
                    e.message
                );
                json!({"error": e.message, "fix": format!(
                    "run `bosun lesson capture --kind budget --issue {n} --title '{LESSON_TITLE}'` with the text '{LESSON_TEXT}' on stdin; do not record the spend again"
                )})
            }
        }
    } else {
        Value::Null
    };
    Ok(Outcome::data(json!({
        "issue": n, "usd": usd, "provider": a.provider,
        "task_total_usd": total, "over_task_cap": over, "blocked": blocked,
        "lesson": lesson,
    })))
}
