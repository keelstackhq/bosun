//! `bosun budget report`: spend per PR for one UTC month (spec §9). Read-only on GitHub; with
//! `--out` it writes `YYYY-MM.md` and `YYYY-MM.json` into a directory.

use std::path::Path;

use serde_json::json;

use super::BudgetReportArgs;
use crate::app::{App, notes, task_meta, trusted_notes};
use crate::core::budget_report::{Row, facts, in_month, markdown, outcome, parse_month, report};
use crate::core::cost::revert_target;
use crate::core::records::{Record, parse_record};
use crate::github::IssueState;
use crate::output::{BosunError, CmdResult, Outcome};

pub fn run(dir: Option<&Path>, args: &BudgetReportArgs) -> CmdResult {
    let app = App::load(dir)?;
    let (gh, now) = (app.gh.as_ref(), app.ctx.now);
    let month = match &args.month {
        Some(m) => parse_month(m).ok_or_else(|| {
            BosunError::usage(
                format!("--month {m:?} is not YYYY-MM"),
                "pass a month like --month 2026-11",
            )
        })?,
        None => now.to_string()[..7].to_string(),
    };
    let mut rows = Vec::new();
    for issue in gh.list_issues(&[], IssueState::All)? {
        if task_meta(&issue).is_none() {
            continue;
        }
        let task_notes = notes(gh, issue.number)?;
        let (sessions, mut models, usd, spends) = facts(&task_notes);
        if spends == 0 {
            continue;
        }
        // The task's PR only from writers' records, as `bosun progress` reads it.
        let pr = trusted_notes(gh, issue.number)?.iter().rev().find_map(|n| {
            match parse_record(&n.body) {
                Some(Record::TaskPr { pr, .. }) => Some(pr),
                _ => None,
            }
        });
        let Some(pull) = pr.map(|n| gh.get_pull(n)).transpose()?.flatten() else {
            continue;
        };
        let merged_at = pull.merged_at.clone().filter(|_| pull.merged);
        let touched = [&merged_at, &pull.closed_at, &pull.updated_at];
        if !touched
            .iter()
            .any(|t| t.as_deref().is_some_and(|t| in_month(&month, t)))
        {
            continue;
        }
        models.extend(facts(&notes(gh, pull.number)?).1);
        models.sort();
        models.dedup();
        let mut reverted = false;
        if pull.merged {
            let head = format!("{}:bosun/revert-{}", app.ctx.repo.owner, pull.number);
            for p in gh.list_pulls(Some(&head), IssueState::Closed)? {
                if revert_target(&p.head_ref) == Some(pull.number)
                    && gh.get_pull(p.number)?.is_some_and(|p| p.merged)
                {
                    reverted = true;
                    break;
                }
            }
        }
        rows.push(Row {
            pr: pull.number,
            title: Some(pull.title.clone()),
            issue: issue.number,
            sessions,
            models,
            usd,
            tokens: None,
            outcome: outcome(&pull.state, pull.merged, reverted),
            merged_at,
        });
    }
    let data = report(&month, &rows);
    let mut written = Vec::new();
    if let Some(out) = &args.out {
        let write = |name: String, text: String| -> Result<String, BosunError> {
            let path = out.join(&name);
            std::fs::create_dir_all(out)
                .and_then(|()| std::fs::write(&path, text))
                .map_err(|e| {
                    BosunError::error(
                        format!("cannot write {}: {e}", path.display()),
                        "check that --out is a writable directory",
                    )
                })?;
            Ok(path.display().to_string())
        };
        let pretty = serde_json::to_string_pretty(&data).unwrap_or_default() + "\n";
        written.push(write(format!("{month}.md"), markdown(&data))?);
        written.push(write(format!("{month}.json"), pretty)?);
    }
    let mut data = data;
    data["written"] = json!(written);
    Ok(Outcome::data(data))
}
