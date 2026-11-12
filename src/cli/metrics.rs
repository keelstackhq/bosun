//! `bosun metrics`: red-CI rate, brake, cost, lead time, owner touches, blocks per rule (spec §18).
//! Read-only. Sources: `ci-ok` check runs of the Actions app (PR heads and merge commits of
//! main), merged revert PRs by a writer, PR and task comments. Main-red issues are never read.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use serde_json::json;

use super::MetricsArgs;
use super::risk_check::owner_logins;
use crate::app::{App, Result, default_branch, is_writer, spend_notes, task_meta, trusted_notes};
use crate::core::budget_report::{facts, merge_spend};
use crate::core::cost::{revert_target, round_usd};
use crate::core::meta::closing_issues;
use crate::core::metrics::{CiRun, HeadCi, ci_head, owner_rules, p50, revert_of};
use crate::core::mode::{BrakeWindow, brake};
use crate::core::records::{Record, parse_record};
use crate::github::{Github, Pull};
use crate::output::{BosunError, CmdResult, Outcome};
use crate::time::{Timestamp, parse_rfc3339};

const DAY: i64 = 86_400;
const BRAKE_DAYS: u32 = 7;

/// One own-branch PR with the `ci-ok` facts of its heads and, once merged, of its merge commit.
struct Facts {
    pull: Pull,
    heads: Vec<HeadCi>,
    main_red: bool,
}

fn ts(t: &Option<String>) -> Option<Timestamp> {
    t.as_deref().and_then(parse_rfc3339)
}

fn ci(gh: &dyn Github, sha: &str, excuse: bool) -> Result<Option<HeadCi>> {
    let runs: Vec<CiRun> = gh
        .check_runs_all(sha)?
        .into_iter()
        .map(|r| CiRun {
            completed: r.status == "completed",
            at: r.completed_at.as_deref().and_then(parse_rfc3339),
            name: r.name,
            app: r.app,
            conclusion: r.conclusion,
        })
        .collect();
    Ok(ci_head(&runs, excuse))
}

fn collect(app: &App, since: Timestamp) -> Result<Vec<Facts>> {
    let gh = app.gh.as_ref();
    let repo = app.ctx.repo.to_string();
    let mut out = Vec::new();
    for pull in gh.list_pulls_updated_since(&since.to_string())? {
        // A fork's heads are not ours to count; its merge into main is.
        let mut heads = Vec::new();
        if pull.head_repo.eq_ignore_ascii_case(&repo) {
            let shas = gh.pull_commits(pull.number)?;
            for (i, sha) in shas.iter().enumerate() {
                // Superseded heads (run cancelled by the push) and a draft's head (`ci` not run).
                heads.extend(ci(gh, sha, i + 1 < shas.len() || pull.draft)?);
            }
        } else if pull.merged_at.is_none() {
            continue;
        }
        let main_red = match (&pull.merged_at, &pull.merge_commit_sha) {
            // Pushes to main never cancel each other: any red `ci-ok` there is red.
            (Some(_), Some(sha)) => ci(gh, sha, false)?.is_some_and(|h| h.failing),
            _ => false,
        };
        out.push(Facts {
            pull,
            heads,
            main_red,
        });
    }
    Ok(out)
}

/// The brake inputs over the last 7 days. Later commands call this, then `core::mode::brake`.
pub fn window(app: &App) -> Result<BrakeWindow> {
    let since = app.ctx.now.minus_secs(i64::from(BRAKE_DAYS) * DAY);
    brake_window(app, &collect(app, since)?, since)
}

fn brake_window(app: &App, facts: &[Facts], since: Timestamp) -> Result<BrakeWindow> {
    let mut w = BrakeWindow {
        days: BRAKE_DAYS,
        main_red: 0,
        reverts: 0,
        failing_heads: 0,
        pushed_heads: 0,
    };
    for f in facts {
        let merged = ts(&f.pull.merged_at).filter(|t| *t >= since);
        w.main_red += u32::from(merged.is_some() && f.main_red);
        if merged.is_some()
            // The branch name cannot be edited after the merge; the body can.
            && (revert_target(&f.pull.head_ref).is_some() || revert_of(&f.pull.body).is_some())
            && is_writer(app.gh.as_ref(), &f.pull.user)?
        {
            w.reverts += 1;
        }
        for h in f.heads.iter().filter(|h| h.at.is_none_or(|t| t >= since)) {
            w.pushed_heads += 1;
            w.failing_heads += u32::from(h.failing);
        }
    }
    Ok(w)
}

pub fn run(dir: Option<&Path>, args: &MetricsArgs) -> CmdResult {
    if args.propose {
        return Err(super::not_built("metrics --propose", "8"));
    }
    if args.days == 0 {
        return Err(BosunError::usage(
            "--days must be at least 1",
            "pass --days 30",
        ));
    }
    let app = App::load(dir)?;
    let (gh, now) = (app.gh.as_ref(), app.ctx.now);
    let start = now.minus_secs(i64::from(args.days) * DAY);
    let b7 = now.minus_secs(i64::from(BRAKE_DAYS) * DAY);
    let all = collect(&app, start.min(b7))?;
    let w = brake_window(&app, &all, b7)?;
    let eff = app.ctx.config.mode_effective();
    let state = brake(eff.mode, &w);
    let in_days = |t: &Option<String>| ts(t).is_some_and(|t| t >= start);

    // Cost per merged PR, lead time per epic (first claim to last merge).
    let (mut merged, mut usd) = (0, 0.0);
    let mut epics: BTreeMap<u64, (Option<Timestamp>, Timestamp)> = BTreeMap::new();
    let mut threads: BTreeSet<u64> = BTreeSet::new();
    for f in all.iter().filter(|f| {
        ts(&f.pull.created_at).is_some_and(|t| t >= start)
            || in_days(&f.pull.merged_at)
            || in_days(&f.pull.closed_at)
    }) {
        threads.insert(f.pull.number);
        let task = closing_issues(&f.pull.body).first().copied();
        threads.extend(task);
        let Some(at) = ts(&f.pull.merged_at).filter(|t| *t >= start) else {
            continue;
        };
        merged += 1;
        let task_spend = match task {
            Some(t) => spend_notes(gh, t)?,
            None => Vec::new(),
        };
        usd += facts(&merge_spend(task_spend, spend_notes(gh, f.pull.number)?)).2;
        let Some(task) = task else { continue };
        let epic = gh
            .get_issue(task)?
            .and_then(|i| task_meta(&i))
            .and_then(|m| m.epic);
        if let Some(e) = epic {
            let claim = trusted_notes(gh, task)?
                .iter()
                .filter(|n| matches!(parse_record(&n.body), Some(Record::Lease { .. })))
                .map(|n| n.created_at)
                .min();
            let s = epics.entry(e).or_insert((None, at));
            s.0 = s.0.into_iter().chain(claim).min();
            s.1 = s.1.max(at);
        }
    }
    let mut hours: Vec<f64> = epics
        .values()
        .filter_map(|(c, m)| Some((m.0 - c.as_ref()?.0) as f64 / 3600.0))
        .collect();
    let round2 = |x: f64| (x * 100.0).round() / 100.0;

    // Owner touches: an owner's approve command or answer on a PR or its task.
    let owners = owner_logins(gh, &default_branch(gh)?)?;
    let mut touches = 0;
    for n in threads.iter().filter(|_| !owners.is_empty()) {
        for c in gh.list_comments(*n)? {
            let counts = owners.iter().any(|o| o.eq_ignore_ascii_case(&c.user))
                && in_days(&Some(c.created_at.clone()))
                && (c.body.starts_with("/bosun approve")
                    || matches!(parse_record(&c.body), Some(Record::Answer { .. })));
            touches += u32::from(counts);
        }
    }

    // Blocks per rule: closed unmerged or re-pushed = caught, merged on the first head = passed.
    let mut blocks: BTreeMap<String, (u32, u32, u32)> = BTreeMap::new();
    for f in &all {
        let p = &f.pull;
        let merged = p.merged_at.is_some();
        let (caught, passed) = match (merged, p.state == "closed") {
            (false, true) if in_days(&p.closed_at) => (true, false),
            (true, _) if in_days(&p.merged_at) => (f.heads.len() > 1, f.heads.len() == 1),
            _ => continue,
        };
        for rule in owner_rules(&p.body) {
            let e = blocks.entry(rule).or_default();
            *e = (e.0 + 1, e.1 + u32::from(caught), e.2 + u32::from(passed));
        }
    }
    let rate = (w.pushed_heads > 0)
        .then(|| (f64::from(w.failing_heads) * 1000.0 / f64::from(w.pushed_heads)).round() / 10.0);
    Ok(Outcome::data(json!({
        "days": args.days, "since": start.to_string(), "mode": eff.mode.as_str(),
        "brake": {"knob": eff.knobs.brake.value.as_str(), "on": state.on, "reasons": state.reasons,
            "window": {"days": w.days, "main_red": w.main_red, "reverts": w.reverts,
                "failing_heads": w.failing_heads, "pushed_heads": w.pushed_heads}},
        "red_ci": {"failing_heads": w.failing_heads, "pushed_heads": w.pushed_heads, "rate_pct": rate},
        "merged": merged,
        "cost_per_merged_usd": (merged > 0).then(|| round_usd(usd / f64::from(merged))),
        "lead_time_p50_hours": p50(&mut hours).map(round2),
        "epics": hours.len(),
        "owner_touches": touches,
        "owner_touches_per_day": round2(f64::from(touches) / f64::from(args.days)),
        "blocks": blocks.iter().map(|(r, (b, c, p))| json!({"rule": r, "blocks": b, "caught": c, "passed": p})).collect::<Vec<_>>(),
    })))
}
