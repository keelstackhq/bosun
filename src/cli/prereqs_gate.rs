//! The prerequisites gate (design F5): claim, `wave plan` and spawn leave out or refuse work
//! that an open prerequisite of the task's plan blocks. It fails closed. Ids, blocks and verify
//! kinds come from the plan file on the default branch; the checklist issue only supplies ticks.

use super::plan::epic_plan;
use super::prereqs::verify;
use crate::app::{Result, default_branch};
use crate::core::meta::TaskMeta;
use crate::core::plan::{self, PlanError};
use crate::core::prereqs::{self as pq, CheckItem};
use crate::github::{Github, Issue, IssueState};
use crate::output::BosunError;

/// An open prerequisite: its id and what it blocks, from the plan.
pub(crate) struct Held {
    pub id: String,
    pub blocks: Vec<String>,
}

fn closed(plan_path: &str, why: String) -> BosunError {
    BosunError::needs_human(
        why,
        format!("run `bosun prereqs {plan_path} --post` (or `bosun plan import`)"),
    )
}

/// The open prerequisites of `plan_path` among those whose `blocks` `wants` accepts, and the
/// checklist issue's name for messages, or `None` when the plan has no prerequisites.
/// An item is done when the API finds it, or, for `verify: owner` and when the API cannot say,
/// when its single box in the issue, with a matching sum, is ticked. Anything unreadable or
/// ambiguous fails closed, and so does a `target` (item, task) the plan does not have.
pub(crate) fn open_items(
    gh: &dyn Github,
    plan_path: &str,
    target: Option<(&str, Option<&str>)>,
    wants: &dyn Fn(&[String]) -> bool,
) -> Result<Option<(String, Vec<Held>)>> {
    super::prereqs::check_plan_path(plan_path).map_err(|e| closed(plan_path, e.message))?;
    let branch = default_branch(gh)?;
    let Some(text) = gh.file_at(plan_path, &branch)? else {
        return Err(closed(
            plan_path,
            format!("{plan_path} is not on {branch}, so its prerequisites cannot be read"),
        ));
    };
    // Always parsed: a misspelt heading is a parse error (closed), not "no section" (open).
    let bad = |e: Vec<PlanError>| closed(plan_path, format!("{plan_path}: {}", e[0].message));
    let parsed = plan::parse(&text).map_err(bad)?;
    let prereqs = pq::parse(&text, &parsed).map_err(bad)?;
    if prereqs.is_empty() {
        return Ok(None);
    }
    if let Some((item, task)) = target {
        let it = parsed.items.iter().find(|i| i.id == item);
        let known =
            it.is_some_and(|i| task.is_none_or(|t| t == "P" || i.tasks.iter().any(|x| x.id == t)));
        if !known {
            let name = task.map_or_else(|| item.to_string(), |t| format!("{item}/{t}"));
            return Err(BosunError::needs_human(
                format!("{name} is not in {plan_path} on {branch}"),
                "re-import the plan (bosun plan amend <plan>) so tasks match it",
            ));
        }
    }
    let marker = pq::issue_marker(plan_path);
    let mut found: Vec<Issue> = gh
        .list_issues(&["bosun:prereqs"], IssueState::All)?
        .into_iter()
        .filter(|i| i.body_text().starts_with(&marker))
        .collect();
    if found.len() > 1 {
        return Err(BosunError::needs_human(
            format!("two checklist issues for {plan_path}"),
            "close one of them; the owner checklist is one issue per plan",
        ));
    }
    let issue = found.pop();
    let ticks = issue
        .as_ref()
        .map_or_else(Vec::new, |i| pq::read_issue(i.body_text()));
    let mut open = Vec::new();
    let mut known = Some(branch.clone());
    for p in prereqs.into_iter().filter(|p| wants(&p.blocks)) {
        let sum = CheckItem::from(&p).sum;
        let done = match verify(gh, &p.verify, &mut known)? {
            Some(found) => found,
            None if issue.is_some() => pq::ticked(&ticks, &p.id, &sum),
            None => {
                return Err(closed(
                    plan_path,
                    format!("{plan_path} has prerequisites but no checklist issue"),
                ));
            }
        };
        if !done {
            open.push(Held {
                id: p.id,
                blocks: p.blocks,
            });
        }
    }
    let name = issue.map_or_else(|| plan_path.to_string(), |i| format!("#{}", i.number));
    Ok(Some((name, open)))
}

/// The ids of the open items that block `item`, or `item/task` when `task` is given.
pub(crate) fn blockers(ticks: &[Held], item: &str, task: Option<&str>) -> Vec<String> {
    let one = task.map(|t| format!("{item}/{t}"));
    let hit = |b: &String| b == item || Some(b) == one.as_ref();
    let held = ticks.iter().filter(|t| t.blocks.iter().any(hit));
    held.map(|t| t.id.clone()).collect()
}

/// The plan path in the body of the task's epic; `None` for hand-made work (no epic, or an epic
/// without bosun's epic marker). A named epic that is gone, or an epic marker without a plan,
/// fails closed. Task and epic bodies are agent-editable like the checklist (design F4): the
/// hook refuses writes naming bosun's markers, best effort.
pub(crate) fn plan_of_task(gh: &dyn Github, meta: &TaskMeta) -> Result<Option<String>> {
    let Some(epic) = meta.epic else {
        return Ok(None);
    };
    let broken = |why: String| {
        BosunError::needs_human(
            why,
            "ask the orchestrator to run `bosun plan amend <plan>` for this task",
        )
    };
    let Some(e) = gh.get_issue(epic)? else {
        return Err(broken(format!(
            "epic #{epic} of {}/{} is missing",
            meta.item, meta.task
        )));
    };
    match epic_plan(e.body_text()) {
        None if e.body_text().contains("<!-- bosun:epic") => Err(broken(format!(
            "epic #{epic} has bosun's marker but no plan path"
        ))),
        plan => Ok(plan),
    }
}

/// Exit 5 when an open item of the task's plan blocks it.
pub(crate) fn refuse_if_held(gh: &dyn Github, n: u64, meta: &Option<TaskMeta>) -> Result<()> {
    let Some(meta) = meta else {
        return Ok(());
    };
    let Some(plan_path) = plan_of_task(gh, meta)? else {
        return Ok(());
    };
    let full = format!("{}/{}", meta.item, meta.task);
    let wants = |b: &[String]| b.iter().any(|x| *x == meta.item || *x == full);
    let target = Some((meta.item.as_str(), Some(meta.task.as_str())));
    let Some((issue, ticks)) = open_items(gh, &plan_path, target, &wants)? else {
        return Ok(());
    };
    let ids = blockers(&ticks, &meta.item, Some(&meta.task));
    if ids.is_empty() {
        return Ok(());
    }
    let (item, task, epic) = (&meta.item, &meta.task, meta.epic.unwrap_or_default());
    Err(BosunError::needs_human(
        format!(
            "#{n} ({item}/{task}) waits for owner steps {} of {plan_path}",
            ids.join(", ")
        ),
        format!(
            "take another task (bosun wave plan {epic}); the owner checklist {issue} lists the steps; after they are done the task can be claimed"
        ),
    ))
}
