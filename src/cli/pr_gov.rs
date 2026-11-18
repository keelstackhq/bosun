//! `bosun pr --pr <n> [--ready]`: the draft-to-auto-merge flow for a PR that has no task (a
//! governance PR such as the one `bosun migrate` opens). The gates are the task path's: the local
//! HEAD is the PR head, every ready gate is stamped, the ruleset requires the gates. No task state
//! moves and no queue comment is written. `risk-check` stays a required check, so auto-merge waits
//! for the owner on R-gov and R-weaken paths. bosun never merges (D13). The paths it may change
//! are R-gov's (`governance_globs`) plus the base's `[approvals] oneoff_allow`.

use std::path::Path;

use serde_json::json;

use super::pr::{check_local_gates, enforced_checks, merge_gates, not_green, update_pull_call};
use crate::app::{App, default_branch, is_writer, kill_switch};
use crate::core::glob::Glob;
use crate::core::meta::linked_issues_in;
use crate::core::risk::{Approval, governance_globs, matches_globs};
use crate::core::touch::plan_tick_file;
use crate::github::{Pull, enable_auto_merge_any, is_already_clean, is_ccr_refusal, is_unstable};
use crate::output::{BosunError, CmdResult, Outcome};

fn pr_json(p: &Pull) -> serde_json::Value {
    json!({"number": p.number, "head_sha": p.head_sha, "draft": p.draft})
}

pub fn run(dir: Option<&Path>, n: u64, ready: bool) -> CmdResult {
    let app = App::load(dir)?;
    let gh = app.gh.as_ref();
    kill_switch(gh)?;
    let mut pull = gh.get_pull(n)?.ok_or_else(|| {
        BosunError::error(
            format!("PR #{n} does not exist"),
            "check the number of the PR, then rerun",
        )
    })?;
    let rerun = format!("bosun pr --pr {n} --ready");
    if pull.state != "open" {
        return Err(BosunError::conflict(
            format!("PR #{n} is {}, not open", pull.state),
            "open a new PR for the change; a closed or merged PR is never queued",
        ));
    }
    if !pull
        .head_repo
        .eq_ignore_ascii_case(&app.ctx.repo.to_string())
    {
        return Err(BosunError::conflict(
            format!("PR #{n} comes from another repo ({:?})", pull.head_repo),
            "a fork's PR is never queued by bosun: the owner reviews it on GitHub",
        ));
    }
    let base = default_branch(gh)?;
    if pull.base_ref != base {
        return Err(BosunError::conflict(
            format!("PR #{n} targets {}, not {base}", pull.base_ref),
            format!("retarget the PR to {base}, then rerun `{rerun}`"),
        ));
    }
    let repo = app.ctx.repo.to_string();
    let linked = linked_issues_in(&pull.body, &repo);
    if pull.body.contains("<!-- bosun:task-pr") || !linked.is_empty() {
        let fix = match linked.first() {
            Some(k) => format!("PR #{n} belongs to task #{k}: run `bosun pr {k} --ready` instead"),
            None => format!(
                "PR #{n} carries a task marker: run `bosun pr <task> --ready` with that task's number"
            ),
        };
        return Err(BosunError::conflict(
            format!("PR #{n} is a task's PR, not a task-less one"),
            fix,
        ));
    }
    // Only a governance PR of a repo writer goes this way; anything else is a task's work.
    let not_governance = |why: String| {
        BosunError::conflict(
            format!("PR #{n} is not a governance PR: {why}"),
            "open it for a task: `bosun pr <issue>`",
        )
    };
    if !is_writer(gh, &pull.user)? {
        return Err(not_governance(format!(
            "its author {} is not a repo writer",
            pull.user
        )));
    }
    // A governance path (R-gov's built-in list) that the base's own R-gov also gates with the
    // owner's approval: risk-check judges by the base rules, so a path they miss would merge with
    // no owner (fail closed). Plus the owner's `[approvals] oneoff_allow` on the base; risk-check
    // still asks the owner for any gated one.
    let builtin = governance_globs();
    let rules = super::risk_check::base_rules(gh, &base)?;
    let gov: Vec<Glob> = (rules.rules.iter())
        .filter(|r| r.id == "R-gov" && r.approval == Approval::Owner)
        .flat_map(|r| r.paths.clone())
        .collect();
    let extra = super::risk_check::oneoff_allow(gh, &base)?;
    let outside = |f: &str| {
        !(matches_globs(&builtin, f) && matches_globs(&gov, f) || matches_globs(&extra, f))
    };
    for f in gh.pull_files(n)? {
        let moved = f.previous_filename.as_deref();
        // risk-check skips a tick-only plan file, so here it would merge with no owner.
        let p = f.patch.as_deref();
        if plan_tick_file(&f.filename, &f.status, moved, p) {
            return Err(not_governance(format!("{} only ticks a plan", f.filename)));
        }
        if outside(&f.filename) || moved.is_some_and(&outside) {
            return Err(not_governance(format!(
                "it changes {}, outside the governance paths the base R-gov gates for the owner and `[approvals] oneoff_allow`",
                f.filename
            )));
        }
    }
    let trailers = super::trailers::check_pr(gh, n, app.ctx.config.trailers.require)?;
    if !ready {
        return Ok(Outcome::data(json!({
            "pr": pr_json(&pull), "task": null, "ready": !pull.draft, "auto_merge": false,
            "trailers": trailers,
        })));
    }
    check_local_gates(&app, &rerun, &pull)?;
    let pending = not_green(gh, &base, &pull.head_sha)?;
    let gates = merge_gates(gh, &pull.head_sha)?;
    let enforced = enforced_checks(gh, &base, &pull, &rerun)?;
    let line: Vec<String> = gates.iter().map(|(c, s)| format!("{c}={s}")).collect();
    eprintln!(
        "bosun: auto-merge on PR #{n} relies on the gates {}; enforcement is {enforced}",
        line.join(", ")
    );
    let mut marked_ready = false;
    if pull.draft {
        match gh.set_draft(&pull.node_id, false) {
            Ok(()) => {
                pull.draft = false;
                marked_ready = true;
            }
            Err(e) if is_ccr_refusal(&e) => {
                let note = format!(
                    "this session cannot call GraphQL: mark PR #{n} ready with the tool call in `next`, then rerun `{rerun}`"
                );
                eprintln!("bosun: {note}");
                let mut out = Outcome::data(json!({
                    "pr": pr_json(&pull), "task": null, "ready": "pending", "auto_merge": false,
                    "enforced_checks": enforced, "note": note, "trailers": trailers,
                }));
                out.next.push(update_pull_call(&app, n, false));
                return Ok(out);
            }
            Err(e) => return Err(e),
        }
    }
    let route = enable_auto_merge_any(gh, &pull).map_err(|e| {
        let mut e = if is_already_clean(&e) {
            let clean = BosunError::needs_human(
                format!("PR #{n} is already mergeable with every required check green, so GitHub cannot turn on auto-merge"),
                format!("the orchestrator merges PR #{n} (hand it over; you never merge)"),
            )
            .with_data(json!({"pr": n, "head_sha": pull.head_sha, "already_clean": true}));
            super::result::clean_handoff(&app, clean, n, &pull.head_sha)
        } else if is_unstable(&e) {
            BosunError::error(
                format!("PR #{n} has a failing check, so GitHub cannot turn on auto-merge"),
                format!(
                    "`bosun wait --pr {n}` names the failing checks: fix them, push, then rerun `{rerun}`"
                ),
            )
            .with_data(json!({"pr": n, "head_sha": pull.head_sha, "unstable": true}))
        } else {
            e
        };
        if marked_ready {
            let mut data = e.data.take().unwrap_or_else(|| json!({}));
            data["ready"] = json!(true);
            e.data = Some(data);
            e.fix = format!("PR #{n} is now ready (it was a draft); {}", e.fix);
        }
        e
    })?;
    Ok(Outcome::data(json!({
        "pr": pr_json(&pull), "task": null, "ready": true, "auto_merge": true,
        "auto_merge_route": route, "enforced_checks": enforced, "trailers": trailers,
        "gates": gates.iter().map(|(c, s)| json!({"context": c, "state": s})).collect::<Vec<_>>(),
        "pending_checks": pending.iter().map(|(c, s)| json!({"context": c, "state": s}))
            .collect::<Vec<_>>(),
    })))
}
