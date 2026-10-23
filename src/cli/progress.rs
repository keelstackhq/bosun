//! `bosun progress`: cost per merged task over the last N days (spec §9, UC-M4-4). Read-only.

use std::path::Path;

use serde_json::json;

use super::ProgressArgs;
use crate::app::{App, notes, task_meta};
use crate::cli::lesson::LESSON_LABEL;
use crate::core::cost::{in_window, mean, per_task, round_usd, window_start};
use crate::core::lessons::{hits_by_week, is_mechanized};
use crate::core::records::{Record, parse_record, spend_total};
use crate::github::{Issue, IssueState};
use crate::output::{CmdResult, Outcome};
use crate::registry;
use crate::time::parse_rfc3339;

pub fn run(dir: Option<&Path>, args: &ProgressArgs) -> CmdResult {
    let app = App::load(dir)?;
    if args.lessons {
        return lessons_view(&app);
    }
    let (gh, now) = (app.gh.as_ref(), app.ctx.now);
    let since = window_start(now, args.days);
    // (issue, title, closed_at, usd) of the task issues merged in the window.
    let mut tasks: Vec<(u64, String, String, f64)> = Vec::new();
    for issue in gh.list_issues(&[], IssueState::All)? {
        if task_meta(&issue).is_none() {
            continue;
        }
        // Merged means closed as completed: `not_planned` and `duplicate` did not ship.
        let shipped = !matches!(
            issue.state_reason.as_deref(),
            Some("not_planned" | "duplicate")
        );
        let done = issue.state == "closed" && shipped;
        let Some(closed_at) = issue.closed_at.clone().filter(|_| done) else {
            continue;
        };
        if !parse_rfc3339(&closed_at).is_some_and(|t| in_window(t, since, now)) {
            continue;
        }
        let usd = spend_total(&notes(gh, issue.number)?, None);
        tasks.push((issue.number, issue.title, closed_at, round_usd(usd)));
    }
    tasks.sort_by_key(|t| t.0);
    let spend_usd = match registry::load(gh)? {
        // Records in [since, now]: a record stamped after now (clock skew) is left out.
        Some((_, n)) => {
            round_usd(spend_total(&n, Some(since)) - spend_total(&n, Some(now.plus_secs(1))))
        }
        None => 0.0,
    };
    let costs: Vec<f64> = tasks.iter().map(|t| t.3).collect();
    let list: Vec<_> = tasks
        .iter()
        .map(|(issue, title, closed_at, usd)| {
            json!({"issue": issue, "title": title, "closed_at": closed_at, "usd": usd})
        })
        .collect();
    Ok(Outcome::data(json!({
        "days": args.days,
        "since": since.to_string(),
        "merged": tasks.len(),
        "spend_usd": spend_usd,
        "cost_per_merged_task_usd": per_task(spend_usd, tasks.len()),
        "task_cost_avg_usd": mean(&costs),
        "tasks": list,
    })))
}

/// `progress --lessons`: the state of the learning loop (spec §6). Hits are the `lesson-seen`
/// and `lesson-hit` records on lesson issues, by comment time, for the last 4 weeks.
fn lessons_view(app: &App) -> CmdResult {
    let (gh, now) = (app.gh.as_ref(), app.ctx.now);
    let all = gh.list_issues(&[LESSON_LABEL], IssueState::All)?;
    let open: Vec<&Issue> = all.iter().filter(|i| i.state == "open").collect();
    let blocking = open
        .iter()
        .filter(|i| i.labels.iter().any(|l| l == "bosun:lesson/blocking"))
        .count();
    let mechanized = all
        .iter()
        .filter(|i| is_mechanized(&i.state, i.state_reason.as_deref(), &i.labels))
        .count();
    let advice = crate::cli::lesson::load(&app.ctx.dir)?.lesson.len();
    let mut times = Vec::new();
    for issue in &all {
        for n in notes(gh, issue.number)? {
            if matches!(
                parse_record(&n.body),
                Some(Record::LessonSeen { .. } | Record::LessonHit { .. })
            ) {
                times.push(n.created_at);
            }
        }
    }
    let weeks = hits_by_week(&times, now, 4);
    let last = weeks.last().map_or(0, |w| w.1);
    let by_week: Vec<_> = weeks
        .iter()
        .map(|(start, hits)| json!({"week_start": start.to_string(), "hits": hits}))
        .collect();
    Ok(Outcome::data(json!({"lessons": {
        "open": open.len(),
        "blocking": blocking,
        "mechanized": mechanized,
        "advice": advice,
        "hits_last_7_days": last,
        "hits_by_week": by_week,
    }})))
}
