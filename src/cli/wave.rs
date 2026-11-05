//! `bosun wave plan <epic>`: the next tasks of an epic that can run in parallel (spec §5, D18).

use std::path::Path;

use serde_json::json;

use super::plan::{epic_plan, epic_shape};
use super::prereqs_gate::{blockers, open_items, plan_ids};
use super::{EpicArg, WaveCmd};
use crate::app::{
    App, active_issues, gated_from_default, limits_from_default, main_red, task_meta,
};
use crate::cli::status::process_red;
use crate::core::lease::live_leases;
use crate::core::shape::touch_gated_in;
use crate::core::state::{State, state_of};
use crate::core::wave::{TaskInfo, plan, plan_single};
use crate::github::IssueState;
use crate::output::{BosunError, CmdResult, Outcome};

pub fn run(dir: Option<&Path>, cmd: &WaveCmd) -> CmdResult {
    let WaveCmd::Plan(EpicArg { epic }) = cmd;
    let app = App::load(dir)?;
    let (gh, now) = (app.gh.as_ref(), app.ctx.now);
    let Some(epic_issue) = gh.get_issue(*epic)? else {
        return Err(BosunError::error(
            format!("issue #{epic} does not exist"),
            "check the epic number; `bosun status` lists the epics",
        ));
    };
    let fixing = |kind: &str| epic_issue.labels.iter().any(|l| l == kind);
    if let Some(red) = main_red(gh)?
        && !fixing("bosun:kind/mainfix")
    {
        return Err(BosunError::halted(
            format!("waves pause while main is red (#{red})"),
            format!("wait for #{red} to close, or plan a mainfix epic"),
        ));
    }
    // Process-red under a feature epic: only its process-fix tasks may run.
    let only_fix = process_red(gh)?
        .filter(|_| !fixing("bosun:kind/mainfix") && !fixing("bosun:kind/process-fix"));
    // Ready process-fix tasks; active tasks stay in the plan so they count against the cap.
    let mut fix_tasks: Vec<u64> = Vec::new();
    let active = |t: &&(TaskInfo, String, String)| t.0.state.is_some_and(State::is_active);
    // Gated is recomputed here from the default branch's rules, never trusted from the task meta.
    let gated_globs = gated_from_default(gh)?;
    let base = crate::app::base_files(gh)?;
    let mut gate_warnings: Vec<String> = Vec::new();
    let mut merged: Vec<String> = Vec::new();
    let mut tasks: Vec<(TaskInfo, String, String)> = Vec::new();
    for issue in gh.list_issues(&[], IssueState::All)? {
        let Some(meta) = task_meta(&issue) else {
            continue;
        };
        let plan_id = format!("{}/{}", meta.item, meta.task);
        let state = state_of(&issue.labels);
        // Closed as not planned (an amendment dropped it) is neither merged nor pending.
        let dropped =
            issue.state == "closed" && issue.state_reason.as_deref() == Some("not_planned");
        if dropped && state != Some(State::Merged) {
            continue;
        }
        if issue.state == "closed" || state == Some(State::Merged) {
            merged.push(plan_id);
        } else if meta.epic == Some(*epic) {
            if meta.kind == "process-fix" && state == Some(State::Ready) {
                fix_tasks.push(issue.number);
            }
            // A wildcard touch is resolved against the default branch's tree; when that cannot
            // be done the task is treated as gated (fail closed) and runs alone.
            let gated = meta.gated
                || meta.risk == "high"
                || touch_gated_in(&meta.touch, &gated_globs, base.as_deref()).unwrap_or_else(|e| {
                    gate_warnings.push(format!("{plan_id}: {e}"));
                    true
                });
            let workspace = format!("berth up {}-{}", meta.item, meta.task).to_lowercase();
            tasks.push((
                TaskInfo {
                    issue: issue.number,
                    plan_id,
                    state,
                    depends: meta.depends,
                    touch: meta.touch,
                    gated,
                },
                issue.title.clone(),
                workspace,
            ));
        }
    }
    if let Some(red) = only_fix
        && fix_tasks.is_empty()
    {
        return Err(BosunError::halted(
            format!("waves pause while process is red (#{red})"),
            format!("close #{red} by merging its process-fix, or plan a process-fix epic"),
        ));
    }
    // Owner prerequisites (design F5): the epic's plan checklist is read once; ready tasks that
    // an open item blocks are left out. A gate error leaves out every ready task of the plan.
    let (mut held, mut left_out) = (Vec::new(), Vec::new());
    let mut warnings = gate_warnings;
    let ready: Vec<&TaskInfo> = (tasks.iter().map(|t| &t.0))
        .filter(|i| i.state == Some(State::Ready))
        .collect();
    if let Some(path) = epic_plan(epic_issue.body_text()).filter(|_| !ready.is_empty()) {
        match open_items(gh, &path, None, &|_| true, None) {
            Ok(Some((_, ticks))) => {
                // open_items just read the plan; if it cannot be read again, all are held.
                let known = plan_ids(gh, &path).unwrap_or_default();
                for &i in &ready {
                    // As claim: a task the plan does not have, or a plan id without `/`, is
                    // held (fail closed).
                    let needs = match i.plan_id.split_once('/') {
                        Some((item, task)) if known.contains(&i.plan_id) => {
                            blockers(&ticks, item, Some(task))
                        }
                        Some(_) => vec![format!("{} is not in {path}", i.plan_id)],
                        None => vec!["unknown task id".to_string()],
                    };
                    if !needs.is_empty() {
                        held.push(json!({"issue": i.issue, "task": i.plan_id, "needs": needs}));
                        left_out.push(i.issue);
                    }
                }
            }
            Ok(None) => {}
            Err(e) => {
                warnings.push(format!("{}; fix: {}", e.message, e.fix));
                left_out.extend(ready.iter().map(|i| i.issue));
            }
        }
    }
    // The epic records its shape at import. Gated tasks never join the bundle (D-016).
    let single = epic_shape(epic_issue.body_text()).as_deref() == Some("single");
    // One PR closes the whole bundle: while a prerequisite holds one ready bundled task, the
    // others wait.
    let mut bundle_held = Vec::new();
    let bundled_out = ready
        .iter()
        .any(|i| !i.gated && left_out.contains(&i.issue));
    if single && bundled_out {
        bundle_held = (ready.iter().filter(|i| !i.gated).map(|i| i.issue))
            .filter(|n| !left_out.contains(n))
            .collect();
        left_out.extend(bundle_held.iter().copied());
    }
    let infos: Vec<TaskInfo> = tasks
        .iter()
        .filter(|t| only_fix.is_none() || fix_tasks.contains(&t.0.issue) || active(t))
        .filter(|t| !left_out.contains(&t.0.issue))
        .map(|t| t.0.clone())
        .collect();
    let mut leased = Vec::new();
    for (i, n) in active_issues(gh)? {
        leased.push((i.number, n));
    }
    let leases = live_leases(&leased, now);
    let max = limits_from_default(gh, &app.ctx)?.waves.max_parallel as usize;
    let wave = if single {
        plan_single(&infos, &merged, &leases, &app.ctx.session, max)
    } else {
        plan(&infos, &merged, &leases, &app.ctx.session, max)
    };
    let find = |n: u64| tasks.iter().find(|t| t.0.issue == n).expect("planned task");
    let run: Vec<_> = wave
        .run
        .iter()
        .map(|&n| {
            let (info, title, workspace) = find(n);
            let mut entry = json!({"issue": n, "title": title, "plan_id": info.plan_id,
                "touch": info.touch, "workspace": workspace});
            if single && !info.gated {
                // One PR for the whole bundle: the lead carries the union of the touch paths.
                let mut touch = info.touch.clone();
                for b in &wave.bundle {
                    for g in &find(*b).0.touch {
                        if !touch.contains(g) {
                            touch.push(g.clone());
                        }
                    }
                }
                let closes: Vec<u64> = std::iter::once(n).chain(wave.bundle.clone()).collect();
                entry["touch"] = json!(touch);
                entry["bundle"] = json!(wave.bundle);
                entry["closes"] = json!(closes);
            }
            entry
        })
        .collect();
    let mut waiting: Vec<_> = wave
        .waiting
        .iter()
        .map(|(n, reason)| json!({"issue": n, "reason": reason}))
        .collect();
    for n in bundle_held {
        let reason = "bundled: waits until no prerequisite holds a task of the item";
        waiting.push(json!({"issue": n, "reason": reason}));
    }
    if let Some(red) = only_fix {
        for (info, _, _) in tasks
            .iter()
            .filter(|t| !fix_tasks.contains(&t.0.issue) && !active(t))
        {
            let reason = format!("process is red (#{red}): only process-fix tasks run");
            waiting.push(json!({"issue": info.issue, "reason": reason}));
        }
    }
    Ok(Outcome::data(
        json!({"epic": epic, "shape": if single { "single" } else { "split" }, "run": run, "waiting": waiting, "held": held, "warnings": warnings}),
    ))
}
