//! `bosun budget record`: write the spend of a task to its issue and the registry (spec §9).

use std::io::Read;
use std::path::Path;

use serde_json::{Value, json};

use super::{BudgetCmd, BudgetRecordArgs};
use crate::app::{App, Result, kill_switch, limits_from_default, notes, set_state};
use crate::core::budget::{level, to_stop};
use crate::core::lease::members;
use crate::core::records::{Record, spend_total};
use crate::core::state::State;
use crate::output::{BosunError, CmdResult, Outcome};
use crate::registry;
use crate::sessions::usage;

pub fn run(dir: Option<&Path>, cmd: &BudgetCmd) -> CmdResult {
    match cmd {
        BudgetCmd::Record(a) => record(dir, a),
        BudgetCmd::Check => check(dir),
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
    let blocked = over && set_state(gh, n, &issue, State::Blocked).is_ok();
    // Lesson capture for a task over budget is M5; until then only the state changes.
    Ok(Outcome::data(json!({
        "issue": n, "usd": usd, "provider": a.provider,
        "task_total_usd": total, "over_task_cap": over, "blocked": blocked,
    })))
}

/// `bosun budget check`: the janitor's budget check (UC-M4-2). Read-only on GitHub: it posts
/// nothing and returns one `interrupt_session` per live session to stop (D29).
fn check(dir: Option<&Path>) -> CmdResult {
    let app = App::load(dir)?;
    let (gh, ctx) = (app.gh.as_ref(), &app.ctx);
    // No kill switch check on purpose: a halted repo still burns money while sessions run,
    // and stopping sessions is always allowed.
    let limits = limits_from_default(gh, ctx)?;
    let daily = limits.budget.daily_usd;
    let (mut spent, mut live) = (0.0, Vec::new());
    if let Some((_, notes)) = registry::load(gh)? {
        spent = spend_total(&notes, Some(ctx.now.day_start()));
        live = members(&notes);
    }
    let lvl = level(spent, daily);
    let stopped = to_stop(lvl, &live, &ctx.session, ctx.now);
    let pct = if daily > 0.0 {
        spent / daily * 100.0
    } else {
        0.0
    };
    let mut out = Outcome::data(json!({
        "spent_today_usd": spent, "daily_usd": daily, "pct": pct,
        "level": lvl.name(), "stopped": stopped,
    }));
    for id in &stopped {
        out = out.with_next("interrupt_session", json!({"session_id": id}));
    }
    Ok(out)
}
