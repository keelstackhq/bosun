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
use crate::core::meta::TaskMeta;
use crate::core::mode::BudgetFeature;
use crate::core::records::{Record, parse_record, spend_total};
use crate::core::state::{State, state_of};
use crate::github::{Issue, IssueState};
use crate::output::{BosunError, CmdResult, Outcome};
use crate::registry;
use crate::sessions::usage;

const LESSON_TITLE: &str = "a task went over per_task_usd";
const LESSON_TEXT: &str = "a task went over its per_task_usd cap and was blocked";

pub fn run(dir: Option<&Path>, cmd: &BudgetCmd) -> CmdResult {
    match cmd {
        BudgetCmd::Record(a) => record(dir, a),
        BudgetCmd::Check => check(dir),
        BudgetCmd::Raise(a) => super::budget_raise::run(dir, a),
        BudgetCmd::Report(a) => super::budget_report::run(dir, a),
    }
}

/// Plan paths as markers and binds spell them: `./plans/x.md` is `plans/x.md`.
pub(super) fn same_path(a: &str, b: &str) -> bool {
    a.trim_start_matches("./") == b.trim_start_matches("./")
}

/// What the epic and its tasks have spent: the tasks its trusted bind lists and the issues whose
/// body names it, each once.
fn feature_spend(app: &App, epic: u64, all: &[Issue], bound: &[u64]) -> Result<f64> {
    let gh = app.gh.as_ref();
    let mut total = spend_total(&spend_notes(gh, epic)?, None);
    for i in all {
        let body = task_meta(i).is_some_and(|m| m.epic == Some(epic));
        if body || bound.contains(&i.number) {
            total += spend_total(&spend_notes(gh, i.number)?, None);
        }
    }
    Ok(total)
}

/// The trusted `bind` records of issue `n` with their comment ids, oldest first.
fn binds_of(gh: &dyn crate::github::Github, n: u64) -> Result<Vec<(u64, Record)>> {
    let notes = crate::app::trusted_notes(gh, n)?;
    let all = notes
        .iter()
        .filter_map(|c| Some((c.id, parse_record(&c.body)?)));
    Ok(all.filter(|r| matches!(r.1, Record::Bind { .. })).collect())
}

/// An epic's binding: the first trusted bind's plan and item, the union of all listed tasks.
pub(crate) struct EpicBind {
    /// The comment id of the first bind.
    pub id: u64,
    pub plan: String,
    pub item: String,
    pub tasks: Vec<u64>,
}

pub(crate) fn epic_bind(gh: &dyn crate::github::Github, n: u64) -> Result<Option<EpicBind>> {
    let mut out: Option<EpicBind> = None;
    for (id, r) in binds_of(gh, n)? {
        if let Record::Bind {
            plan,
            item,
            task: None,
            epic: None,
            tasks,
            ..
        } = r
        {
            let b = out.get_or_insert(EpicBind {
                id,
                plan,
                item,
                tasks: vec![],
            });
            b.tasks.extend(tasks);
        }
    }
    Ok(out)
}

type Tb = (u64, u64, String, String);

/// A task's binding: the first trusted bind's comment id, epic, item and task.
fn task_bind(gh: &dyn crate::github::Github, n: u64) -> Result<Option<Tb>> {
    Ok(binds_of(gh, n)?.into_iter().find_map(|(id, r)| match r {
        Record::Bind {
            item,
            task: Some(t),
            epic: Some(e),
            ..
        } => Some((id, e, item, t)),
        _ => None,
    }))
}

/// Write the binds of one imported item (modes 7b): each task issue without a trusted bind, and
/// the epic when it has none or its binds lack some of the tasks.
pub(crate) fn write_binds(
    gh: &dyn crate::github::Github,
    session: &str,
    plan: &str,
    item: &crate::core::plan::Item,
    epic: u64,
    tasks: &[(String, u64)],
) -> Result<()> {
    let bind = |task: Option<&str>, e: Option<u64>, tasks: Vec<u64>| Record::Bind {
        session: session.into(),
        plan: plan.into(),
        item: item.id.clone(),
        task: task.map(String::from),
        epic: e,
        tasks,
    };
    // A first bind that differs from the plan is forged (it would stay the binding for good):
    // write nothing and ask the owner to delete it.
    let have = epic_bind(gh, epic)?;
    let mut bad: Vec<(u64, u64)> = vec![];
    for (t, n) in tasks {
        if let Some((id, e, i, tt)) = task_bind(gh, *n)?
            && (e, &i, &tt) != (epic, &item.id, t)
        {
            bad.push((*n, id));
        }
    }
    if let Some(b) = have
        .as_ref()
        .filter(|b| (&b.plan, &b.item) != (&plan.to_string(), &item.id))
    {
        bad.push((epic, b.id));
    }
    if let Some(&(n, id)) = bad.first() {
        let list: Vec<String> = bad
            .iter()
            .map(|(n, id)| format!("comment {id} on #{n}"))
            .collect();
        return Err(BosunError::needs_human(
            format!(
                "bind records are forged or moved ({}): the import stopped here; items before it were written",
                list.join(", ")
            ),
            format!(
                "the owner deletes the forged bind comment {id} on #{n}, then reruns `bosun plan amend {plan}`"
            ),
        ));
    }
    for (t, n) in tasks {
        if task_bind(gh, *n)?.is_none() {
            gh.add_comment(*n, &bind(Some(t), Some(epic), vec![]).to_line())?;
        }
    }
    let known = have.as_ref().map(|b| b.tasks.clone()).unwrap_or_default();
    let missing: Vec<u64> = tasks
        .iter()
        .map(|t| t.1)
        .filter(|n| !known.contains(n))
        .collect();
    if have.is_none() || !missing.is_empty() {
        gh.add_comment(epic, &bind(None, None, missing).to_line())?;
    }
    Ok(())
}

/// The registry issue's first trusted `epic-index` for (plan, item): the registry issue, its
/// comment id and the epic. `None` without a registry issue or an index.
fn epic_index(
    gh: &dyn crate::github::Github,
    plan: &str,
    item: &str,
) -> Result<Option<(u64, u64, u64)>> {
    let Some((reg, notes)) = registry::load(gh)? else {
        return Ok(None);
    };
    Ok(first_index(&notes, plan, item).map(|(id, e)| (reg, id, e)))
}

fn first_index(notes: &[crate::core::records::Note], plan: &str, item: &str) -> Option<(u64, u64)> {
    notes.iter().find_map(|c| match parse_record(&c.body)? {
        Record::EpicIndex {
            plan: p,
            item: i,
            epic,
            ..
        } if same_path(&p, plan) && i == item => Some((c.id, epic)),
        _ => None,
    })
}

/// Write the epic index of one imported item on the registry issue (modes 7d) unless it has
/// one; a first index naming another epic is forged or moved. No registry issue: nothing is
/// written, and the guard refuses until `bosun plan amend` runs after a session has joined.
pub(crate) fn write_index(
    gh: &dyn crate::github::Github,
    session: &str,
    plan: &str,
    item: &str,
    epic: u64,
) -> Result<()> {
    let Some((reg, notes)) = registry::load(gh)? else {
        return Ok(());
    };
    match first_index(&notes, plan, item) {
        Some((id, e)) if e != epic => Err(BosunError::needs_human(
            format!("the epic index of {plan}/{item} names #{e}, not #{epic} (forged or moved)"),
            format!(
                "the owner deletes the index comment {id} on the registry issue #{reg}, then reruns `bosun plan amend {plan}`"
            ),
        )),
        Some(_) => Ok(()),
        None => {
            let r = Record::EpicIndex {
                session: session.into(),
                plan: plan.into(),
                item: item.into(),
                epic,
            };
            gh.add_comment(reg, &r.to_line())?;
            Ok(())
        }
    }
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
pub(super) struct Source {
    pub(super) budget: f64,
    item: Option<String>,
    plan: Option<(String, crate::core::plan::Plan)>,
    /// The tasks the epic's trusted bind lists.
    tasks: Vec<u64>,
}

pub(super) fn source(app: &App, epic: u64) -> Result<Source> {
    let gh = app.gh.as_ref();
    let body = gh.get_issue(epic)?.map(|e| e.body_text().to_string());
    let body = body.unwrap_or_default();
    let per_task = limits_from_default(gh, &app.ctx)?.budget.per_task_usd;
    let bind = epic_bind(gh, epic)?;
    let tasks = bind.as_ref().map(|b| b.tasks.clone()).unwrap_or_default();
    let (mp, mi) = (epic_plan(&body), epic_item(&body));
    if let (Some(b), Some(p), Some(i)) = (&bind, &mp, &mi)
        && (same_path(&b.plan, p), &b.item) != (true, i)
    {
        return Err(BosunError::needs_human(
            format!(
                "the bind of epic #{epic} names {}/{}, its marker {p}/{i}",
                b.plan, b.item
            ),
            format!("the owner deletes the bind comment {} on #{epic}", b.id),
        )
        .with_data(json!({"check": "budget_feature", "name": "budget_feature", "epic": epic})));
    }
    let (file, item) = match bind {
        Some(b) => (Some(b.plan), Some(b.item)),
        None => (mp, mi),
    };
    let Some(f) = file else {
        // No marker: one task's worth, which a dummy issue cannot inflate.
        let budget = feature_estimate(1, per_task);
        return Ok(Source {
            budget,
            item,
            plan: None,
            tasks,
        });
    };
    let branch = crate::app::default_branch(gh)?;
    let parsed = gh
        .file_at(&f, &branch)?
        .and_then(|t| crate::core::plan::parse(&t).ok());
    let found = parsed.as_ref().zip(item.as_ref()).and_then(|(p, id)| {
        p.items.iter().find(|i| &i.id == id).map(|i| {
            i.budget_usd
                .unwrap_or(feature_estimate(i.tasks.len(), per_task))
        })
    });
    match (found, parsed) {
        (Some(budget), Some(p)) => Ok(Source {
            budget,
            item,
            plan: Some((f, p)),
            tasks,
        }),
        _ => Err(BosunError::needs_human(
            format!(
                "epic #{epic} names {f}, which is missing, unreadable or lacks its item on {branch}"
            ),
            format!("ask the orchestrator to run `bosun plan amend {f}`"),
        )
        .with_data(json!({"check": "budget_feature", "name": "budget_feature", "epic": epic}))),
    }
}

/// The feature-budget guard of `epic` for claim, spawn and heal. Knob off (default branch): no
/// more calls. Under 80 %: `None`. At 80 % a warning; at 100 % a warning under `warn`, exit 5
/// under `ask`. `task` is the claimed task and whether its body or label says mainfix: the plan's
// `kind:` alone exempts mainfix (the flag only lets a mainfix pass a bad config), and the epic
/// marker must name the task's item (a missing marker refuses).
pub(crate) fn feature_guard(
    app: &App,
    epic: u64,
    task: Option<(u64, &TaskMeta, bool)>,
) -> Result<Option<Value>> {
    let knob = match knob_from_default(app.gh.as_ref()) {
        Ok(k) => k,
        Err(_) if task.is_some_and(|t| t.2) => return Ok(None),
        Err(e) => return Err(e),
    };
    if knob == BudgetFeature::Off {
        return Ok(None);
    }
    // Feature spend and indexes come from the registry: with the knob on (any value but off) it
    // must be pinned, for every task: a mainfix flag can come from an editable label or body.
    if !crate::registry::pinned(app.gh.as_ref())? {
        return Err(BosunError::needs_human(
            format!(
                "budget_feature is on but {} is not on the default branch, so the registry issue can be swapped",
                crate::registry::PIN_FILE
            ),
            "commit `.bosun/registry` with the registry issue number (an R-gov PR the owner approves)",
        )
        .with_data(json!({"check": "budget_feature", "name": "budget_feature", "epic": epic})));
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
    let all = app.gh.list_issues(&[], IssueState::All)?;
    // The registry's first trusted index names the canonical epic of this plan item; another
    // epic is refused even for mainfix, a missing index only after the mainfix exemption.
    let mut canon = None;
    if src.plan.is_some()
        && let Some((_, id, c)) = epic_index(app.gh.as_ref(), file, item)?
    {
        if c != epic {
            let msg = format!("#{c} is the epic of {file}/{item}, not #{epic}");
            let fix = format!(
                "work under #{c} (reopen it if it is closed); if the index is forged the owner deletes the index comment {id} on the registry issue and reruns `bosun plan amend {file}` (until then a forged index also blocks the mainfix tasks of the real epic)"
            );
            return Err(refuse(msg, fix, 0.0));
        }
        canon = Some(c);
    }
    if let Some((n, m, _)) = task {
        // Each issue once: body metas of the epic and the tasks its bind lists, by (item, task).
        let live =
            |i: &Issue| !matches!(i.state_reason.as_deref(), Some("duplicate" | "not_planned"));
        let (mut alive, mut binds) = (0, vec![]);
        for i in &all {
            let by_body = task_meta(i)
                .is_some_and(|x| x.epic == Some(epic) && (&x.item, &x.task) == (&m.item, &m.task));
            let by_bind = src.tasks.contains(&i.number)
                && task_bind(app.gh.as_ref(), i.number)?
                    .is_some_and(|(_, e, it, t)| (e, &it, &t) == (epic, &m.item, &m.task));
            alive += usize::from((by_body || by_bind) && live(i));
            if by_bind {
                binds.push(i.number);
            }
        }
        if alive > 1 {
            let msg = format!("{}/{} names more than one task issue", m.item, m.task);
            let fix = "the orchestrator closes the extra issues as duplicates".to_string();
            return Err(refuse(msg, fix, 0.0));
        }
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
        // One live copy, and when any copy has a bind it is this one: no borrowing an id.
        if alive == 1
            && (binds.is_empty() || binds.contains(&n))
            && kind.is_some_and(|k| k == "mainfix")
        {
            return Ok(None);
        }
    }
    if src.plan.is_some() && canon.is_none() {
        let msg = format!("{file}/{item} has no epic index on the registry issue");
        let fix =
            format!("the orchestrator runs `bosun plan amend {file}` (writes the epic index)");
        return Err(refuse(msg, fix, 0.0));
    }
    let raised: u64 = super::budget_raise::valid_raises(app, epic, src.budget)?
        .iter()
        .fold(0u64, |a, r| a.saturating_add(r.usd));
    let (spent, budget) = (
        feature_spend(app, epic, &all, &src.tasks)?,
        src.budget + raised as f64,
    );
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
                "the owner answers `raise {epic} +<usd>` in `bosun digest`, then the orchestrator runs `bosun budget raise --from -` with the `bosun digest --answer --sign-key` output (or the owner raises `budget_usd:` of item {item} in {file} in an R-gov PR)"
            ),
            spent,
        )),
    }
}

/// Every epic of the registry's epic index (the first trusted index of each plan item; closed
/// epics skipped) whose feature spend is at 80 % of its budget or more, as `{epic, spent_usd,
/// budget_usd, level}` with level `warn` or `over` (`over` also has `raise_room_usd`: what a
/// signed raise may still add today). Read-only and never refuses: an epic that cannot be read
/// (its plan is missing or unreadable, GitHub fails on it) is in the second list as `{epic,
/// error}`. Callers read the knob. Every indexed open epic costs a few reads.
pub(crate) fn feature_levels(app: &App) -> Result<(Vec<Value>, Vec<Value>)> {
    let gh = app.gh.as_ref();
    let Some((_, notes)) = registry::load(gh)? else {
        return Ok((vec![], vec![]));
    };
    let (mut seen, mut epics): (Vec<(String, String)>, Vec<u64>) = (Vec::new(), Vec::new());
    for n in &notes {
        if let Some(Record::EpicIndex {
            plan, item, epic, ..
        }) = parse_record(&n.body)
            && !seen.iter().any(|(p, i)| same_path(p, &plan) && i == &item)
        {
            seen.push((plan, item));
            if !epics.contains(&epic) {
                epics.push(epic);
            }
        }
    }
    let all = if epics.is_empty() {
        vec![]
    } else {
        gh.list_issues(&[], IssueState::All)?
    };
    let today = app.ctx.now.to_string();
    let (mut out, mut skipped) = (Vec::new(), Vec::new());
    for epic in epics {
        match one_level(app, epic, &all, &today[..10]) {
            Ok(Some(v)) => out.push(v),
            Ok(None) => {}
            Err(e) => skipped.push(json!({"epic": epic, "error": e.message})),
        }
    }
    Ok((out, skipped))
}

fn one_level(app: &App, epic: u64, all: &[Issue], today: &str) -> Result<Option<Value>> {
    if app.gh.get_issue(epic)?.is_none_or(|i| i.state == "closed") {
        return Ok(None);
    }
    let src = source(app, epic)?;
    let raises = super::budget_raise::valid_raises(app, epic, src.budget)?;
    let raised: u64 = raises.iter().fold(0u64, |a, r| a.saturating_add(r.usd));
    let spent = feature_spend(app, epic, all, &src.tasks)?;
    let budget = src.budget + raised as f64;
    let mut v = json!({"epic": epic, "spent_usd": spent, "budget_usd": budget});
    match feature_level(spent, budget) {
        Feature::Under => return Ok(None),
        Feature::Warn => v["level"] = json!("warn"),
        Feature::Over => {
            v["level"] = json!("over");
            v["raise_room_usd"] = json!(super::budget_raise::room(&raises, src.budget, today));
        }
    }
    Ok(Some(v))
}

/// The guard of task issue `n`: its trusted bind (epic, item, task), when it has one, replaces
/// the body's, so editing the body cannot move or drop the task.
pub(crate) fn task_guard(
    app: &App,
    n: u64,
    m: Option<&TaskMeta>,
    mainfix: bool,
) -> Result<Option<Value>> {
    let gh = app.gh.as_ref();
    if knob_from_default(gh).is_ok_and(|k| k == BudgetFeature::Off) {
        return Ok(None);
    }
    let bind = task_bind(gh, n)?;
    // No task meta and no task bind: only a bound epic has a budget to guard.
    if m.is_none() && bind.is_none() {
        return match epic_bind(gh, n)? {
            Some(_) => feature_guard(app, n, None),
            None => Ok(None),
        };
    }
    // A deleted meta block is rebuilt from the bind.
    let built = bind.as_ref().and_then(|(_, _, i, t)| {
        serde_json::from_value::<TaskMeta>(json!({"item": i, "task": t})).ok()
    });
    let Some(mut m) = m.cloned().or(built) else {
        return Ok(None);
    };
    if let Some((id, epic, item, task)) = bind {
        let eb = epic_bind(gh, epic)?;
        let plan = eb
            .as_ref()
            .map_or("<plan>", |b| b.plan.as_str())
            .to_string();
        let refuse = |msg: String, fix: String| {
            BosunError::needs_human(msg, fix).with_data(json!({"check": "budget_feature",
                "name": "budget_feature", "epic": epic}))
        };
        // The epic must list the task too, or the bind is forged.
        if !eb.is_some_and(|b| b.tasks.contains(&n)) {
            return Err(refuse(
                format!("#{n} is bound to epic #{epic}, which does not list it"),
                format!(
                    "rerun `bosun plan amend {plan}`; if that does not list it, the bind is forged: the owner deletes comment {id} on #{n}"
                ),
            ));
        }
        // A body that disagrees with the bind was edited.
        if (&m.item, &m.task) != (&item, &task) || m.epic.is_some_and(|e| e != epic) {
            return Err(refuse(
                format!(
                    "#{n} is bound to {item}/{task} but its body says {}/{}",
                    m.item, m.task
                ),
                format!(
                    "the owner deletes the bind comment {id} on #{n} (or restores the body, then `bosun plan amend {plan}`)"
                ),
            ));
        }
        (m.epic, m.item, m.task) = (Some(epic), item, task);
    }
    match m.epic {
        Some(e) => feature_guard(app, e, Some((n, &m, mainfix))),
        None => Ok(None),
    }
}

/// The guard of `bosun spawn <n>`. An epic with a trusted bind goes the epic path whatever its
/// body says and has no mainfix-label exemption; a task is guarded like a claim.
pub(crate) fn spawn_guard(
    app: &App,
    n: u64,
    meta: Option<&TaskMeta>,
    mainfix: bool,
) -> Result<Option<Value>> {
    let gh = app.gh.as_ref();
    if knob_from_default(gh).is_ok_and(|k| k == BudgetFeature::Off) {
        return Ok(None);
    }
    if epic_bind(gh, n)?.is_some() {
        return feature_guard(app, n, None);
    }
    if meta.is_some() || task_bind(gh, n)?.is_some() {
        return task_guard(app, n, meta, mainfix);
    }
    match mainfix {
        true => Ok(None),
        false => feature_guard(app, n, None),
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
