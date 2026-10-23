//! `bosun wave plan <epic>`: the next tasks of an epic that can run in parallel (spec §5, D18).

use std::path::Path;

use serde_json::json;

use super::{EpicArg, WaveCmd};
use crate::app::{App, active_issues, main_red, task_meta};
use crate::cli::status::process_red;
use crate::core::lease::live_leases;
use crate::core::state::{State, state_of};
use crate::core::wave::{TaskInfo, plan};
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
    if let Some(red) = process_red(gh)?
        && !fixing("bosun:kind/mainfix")
        && !fixing("bosun:kind/process-fix")
    {
        return Err(BosunError::halted(
            format!("waves pause while process is red (#{red})"),
            format!("close #{red} by merging its process-fix, or plan a process-fix epic"),
        ));
    }
    let mut merged: Vec<String> = Vec::new();
    let mut tasks: Vec<(TaskInfo, String, String)> = Vec::new();
    for issue in gh.list_issues(&[], IssueState::All)? {
        let Some(meta) = task_meta(&issue) else {
            continue;
        };
        let plan_id = format!("{}/{}", meta.item, meta.task);
        let state = state_of(&issue.labels);
        if issue.state == "closed" || state == Some(State::Merged) {
            merged.push(plan_id);
        } else if meta.epic == Some(*epic) {
            let workspace = format!("berth up {}-{}", meta.item, meta.task).to_lowercase();
            tasks.push((
                TaskInfo {
                    issue: issue.number,
                    plan_id,
                    state,
                    depends: meta.depends,
                    touch: meta.touch,
                },
                issue.title.clone(),
                workspace,
            ));
        }
    }
    let infos: Vec<TaskInfo> = tasks.iter().map(|t| t.0.clone()).collect();
    let mut leased = Vec::new();
    for (i, n) in active_issues(gh)? {
        leased.push((i.number, n));
    }
    let leases = live_leases(&leased, now);
    let max = app.ctx.limits.waves.max_parallel as usize;
    let wave = plan(&infos, &merged, &leases, &app.ctx.session, max);
    let find = |n: u64| tasks.iter().find(|t| t.0.issue == n).expect("planned task");
    let run: Vec<_> = wave
        .run
        .iter()
        .map(|&n| {
            let (info, title, workspace) = find(n);
            json!({"issue": n, "title": title, "plan_id": info.plan_id,
                "touch": info.touch, "workspace": workspace})
        })
        .collect();
    let waiting: Vec<_> = wave
        .waiting
        .iter()
        .map(|(n, reason)| json!({"issue": n, "reason": reason}))
        .collect();
    Ok(Outcome::data(
        json!({"epic": epic, "run": run, "waiting": waiting}),
    ))
}
