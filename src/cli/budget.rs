//! `bosun budget record`: write the spend of a task to its issue and the registry (spec §9).

use std::io::Read;
use std::path::Path;

use serde_json::{Value, json};

use super::lesson::capture_lesson;
use super::{BudgetCmd, BudgetRecordArgs};
use crate::app::{
    App, Result, kill_switch, limits_from_default, set_state, spend_notes, task_meta,
};
use crate::cli::plan::{epic_item, epic_plan};
use crate::core::budget::{Feature, feature_estimate, feature_level, level, to_stop};
use crate::core::lease::members;
use crate::core::mode::BudgetFeature;
use crate::core::records::{Record, spend_total};
use crate::core::state::{State, state_of};
use crate::github::IssueState;
use crate::output::{BosunError, CmdResult, Outcome};
use crate::registry;
use crate::sessions::usage;

const LESSON_TITLE: &str = "a task went over per_task_usd";
const LESSON_TEXT: &str = "a task went over its per_task_usd cap and was blocked";

pub fn run(dir: Option<&Path>, cmd: &BudgetCmd) -> CmdResult {
    match cmd {
        BudgetCmd::Record(a) => record(dir, a),
        BudgetCmd::Check => check(dir),
        BudgetCmd::Report(a) => super::budget_report::run(dir, a),
    }
}

/// What the tasks of `epic` have spent, over every state (spec §9, feature budget).
pub(crate) fn feature_spend(app: &App, epic: u64) -> Result<f64> {
    let gh = app.gh.as_ref();
    let mut total = spend_total(&spend_notes(gh, epic)?, None);
    for i in gh.list_issues(&[], IssueState::All)? {
        if task_meta(&i).is_some_and(|m| m.epic == Some(epic)) {
            total += spend_total(&spend_notes(gh, i.number)?, None);
        }
    }
    Ok(total)
}

/// The `budget_feature` knob of the default branch's `.bosun/bosun.toml`, as wave and heal read it.
pub(crate) fn knob_from_default(gh: &dyn crate::github::Github) -> Result<BudgetFeature> {
    let branch = crate::app::default_branch(gh)?;
    let file = crate::config::CONFIG_FILE;
    Ok(match gh.file_at(file, &branch)? {
        Some(t) => crate::config::Config::parse(&t, &format!("{file} on {branch}"))?,
        None => crate::config::Config::default(),
    }
    .mode_effective()
    .knobs
    .budget_feature
    .value)
}

/// Where the budget of an epic comes from. The epic body is editable by anyone with write
/// access, so its `budget_usd` is display only: the plan on the default branch decides.
struct Source {
    budget: f64,
    item: Option<String>,
    plan: Option<(String, crate::core::plan::Plan)>,
}

fn source(app: &App, epic: u64) -> Result<Source> {
    let gh = app.gh.as_ref();
    let body = gh.get_issue(epic)?.map(|e| e.body_text().to_string());
    let body = body.unwrap_or_default();
    let per_task = limits_from_default(gh, &app.ctx)?.budget.per_task_usd;
    let (file, item) = (epic_plan(&body), epic_item(&body));
    let branch = crate::app::default_branch(gh)?;
    let parsed = match &file {
        Some(f) => gh
            .file_at(f, &branch)?
            .and_then(|t| crate::core::plan::parse(&t).ok()),
        None => None,
    };
    let found = parsed.as_ref().zip(item.as_ref()).and_then(|(p, id)| {
        p.items.iter().find(|i| &i.id == id).map(|i| {
            i.budget_usd
                .unwrap_or(feature_estimate(i.tasks.len(), per_task))
        })
    });
    let budget = match found {
        Some(b) => b,
        None => {
            let all = gh.list_issues(&[], IssueState::All)?;
            let n = all
                .iter()
                .filter(|i| task_meta(i).is_some_and(|m| m.epic == Some(epic)))
                .count();
            feature_estimate(n.max(1), per_task)
        }
    };
    Ok(Source {
        budget,
        item,
        plan: file.zip(parsed),
    })
}

/// The feature-budget guard of `epic` for claim, spawn and heal. Knob off (default branch): no
/// more calls. Under 80 %: `None`. At 80 % a warning; at 100 % a warning under `warn`, exit 5
/// under `ask`. `task` is the claimed task and whether its body or label says mainfix: the plan's
// `kind:` alone exempts mainfix (the flag only lets a mainfix pass a bad config), and the epic
/// marker must name the task's item (a missing marker refuses).
pub(crate) fn feature_guard(
    app: &App,
    epic: u64,
    task: Option<(&crate::core::meta::TaskMeta, bool)>,
) -> Result<Option<Value>> {
    let knob = match knob_from_default(app.gh.as_ref()) {
        Ok(k) => k,
        Err(_) if task.is_some_and(|t| t.1) => return Ok(None),
        Err(e) => return Err(e),
    };
    if knob == BudgetFeature::Off {
        return Ok(None);
    }
    let src = source(app, epic)?;
    let file = src
        .plan
        .as_ref()
        .map(|p| p.0.as_str())
        .unwrap_or("its plan");
    let item = src.item.as_deref().unwrap_or("?");
    let refuse = |msg: String, fix: String, spent: f64| {
        BosunError::needs_human(msg, fix).with_data(json!({"check": "budget_feature",
            "name": "budget_feature", "epic": epic, "spent_usd": spent, "budget_usd": src.budget}))
    };
    if let Some((m, _)) = task {
        if src.item.as_deref() != Some(m.item.as_str()) {
            let msg = format!("the epic #{epic} marker names item {item}, not {}", m.item);
            return Err(refuse(
                msg,
                format!("ask the orchestrator to run `bosun plan amend {file}`"),
                0.0,
            ));
        }
        let kind = src.plan.as_ref().and_then(|(_, p)| {
            let it = p.items.iter().find(|i| i.id == m.item)?;
            it.tasks
                .iter()
                .find(|t| t.id == m.task)
                .map(|t| t.kind.clone())
        });
        // Only the plan exempts: a body or label kind is editable.
        if kind.is_some_and(|k| k == "mainfix") {
            return Ok(None);
        }
    }
    let (spent, budget) = (feature_spend(app, epic)?, src.budget);
    let warn = |level: &str| {
        Ok(Some(
            json!({"epic": epic, "spent_usd": spent, "budget_usd": budget, "level": level}),
        ))
    };
    match feature_level(spent, budget) {
        Feature::Under => Ok(None),
        Feature::Warn => warn("warn"),
        Feature::Over if knob == BudgetFeature::Warn => warn("over"),
        Feature::Over => Err(refuse(
            format!("epic #{epic} spent ${spent:.2} of its ${budget:.2} feature budget"),
            format!(
                "the owner raises `budget_usd:` of item {item} in {file} on the default branch (an R-gov PR); digest raises come in a later release"
            ),
            spent,
        )),
    }
}

pub(super) fn read_input(from: &str) -> Result<Value> {
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

pub(super) fn usd_of(
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
    if !a.pr.is_empty() {
        return super::budget_pr::run(dir, a);
    }
    let Some(n) = a.issue else {
        return Err(BosunError::usage(
            "budget record needs an issue number or --pr",
            "pass the task issue, or --pr <n> for each PR the session made",
        ));
    };
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
        model: None,
    };
    gh.add_comment(n, &spend.to_line())?;
    let (registry_issue, _) = registry::ensure(gh)?;
    gh.add_comment(registry_issue, &spend.to_line())?;

    let total = spend_total(&crate::app::spend_notes(gh, n)?, None);
    let over = total > limits.budget.per_task_usd;
    let was_blocked = state_of(&issue.labels) == Some(State::Blocked);
    let blocked = over && set_state(gh, n, &issue, State::Blocked).is_ok();
    // One lesson for "a task went over its cap" (the amounts and the task number do not enter
    // the signature: each task adds a hit). Only the record that blocks the task captures, so
    // later records on a blocked task add no hit; a task blocked for another reason first, or
    // one that cannot move to blocked, gets no budget lesson; two records racing on one task can
    // both capture (one extra hit). The spend is already recorded, so a failed capture must not
    // fail the command: a retry would record the spend twice.
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
        spent = spend_total(
            &registry::trusted_spend_notes(gh)?,
            Some(ctx.now.day_start()),
        );
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
