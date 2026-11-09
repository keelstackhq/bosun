//! `bosun budget report`: spend per PR for one UTC month (spec §9). Read-only on GitHub; with
//! `--out` it writes `YYYY-MM.md` and `YYYY-MM.json` into a directory.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use serde_json::json;

use super::BudgetReportArgs;
use crate::app::{App, spend_notes, task_meta, trusted_notes};
use crate::core::budget_report::{Row, facts, in_month, markdown, outcome, parse_month, report};
use crate::core::cost::revert_target;
use crate::core::records::{Note, Record, parse_record};
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
    // Task issues by number; the PR each names in a writers' `task-pr` record. A PR keeps the
    // lowest task number that names it. Tasks with spend and no PR record are listed as skipped.
    let mut issues = gh.list_issues(&[], IssueState::All)?;
    issues.sort_by_key(|i| i.number);
    let mut task_of: BTreeMap<u64, (u64, Vec<Note>)> = BTreeMap::new();
    let mut skipped: Vec<u64> = Vec::new();
    for issue in issues.iter().filter(|i| task_meta(i).is_some()) {
        let trusted = trusted_notes(gh, issue.number)?;
        let pr = trusted
            .iter()
            .rev()
            .find_map(|n| match parse_record(&n.body) {
                Some(Record::TaskPr { pr, .. }) => Some(pr),
                _ => None,
            });
        match pr {
            Some(pr) => {
                task_of.entry(pr).or_insert((issue.number, trusted));
            }
            None if facts(&spend_notes(gh, issue.number)?).3 > 0 => skipped.push(issue.number),
            None => {}
        }
    }
    // Every PR opened, merged or closed in the month. The list is newest update first and stops
    // at the first page that ends before the month: a PR with an event in the month was updated
    // in or after it. The same list tells which merged PRs a merged `bosun/revert-<n>` undid.
    let pulls = gh.list_pulls_updated_since(&format!("{month}-01T00:00:00Z"))?;
    // Only this repo's own branches count, as for `bosun progress` (a fork's branch has another owner).
    let repo = app.ctx.repo.to_string();
    let undone: BTreeSet<u64> = pulls
        .iter()
        .filter(|p| p.merged_at.is_some() && p.head_repo.eq_ignore_ascii_case(&repo))
        .filter_map(|p| revert_target(&p.head_ref))
        .collect();
    let mut rows = Vec::new();
    for pull in &pulls {
        let events = [&pull.created_at, &pull.merged_at, &pull.closed_at];
        if !events
            .iter()
            .any(|t| t.as_deref().is_some_and(|t| in_month(&month, t)))
        {
            continue;
        }
        // Lists do not carry `merged`: a merge time says it.
        let merged = pull.merged_at.is_some();
        let task = task_of.get(&pull.number);
        let (mut sessions, mut usd) = (None, None);
        let mut models = task.map_or_else(Vec::new, |(_, n)| facts(n).1);
        if let Some((issue, _)) = task {
            let (s, _, u, spends) = facts(&spend_notes(gh, *issue)?);
            if spends > 0 {
                (sessions, usd) = (Some(s), Some(u));
            }
        }
        models.extend(facts(&trusted_notes(gh, pull.number)?).1);
        models.sort();
        models.dedup();
        rows.push(Row {
            pr: pull.number,
            title: pull.title.clone(),
            issue: task.map(|t| t.0),
            sessions,
            models: (!models.is_empty()).then_some(models),
            usd,
            tokens: None,
            outcome: outcome(&pull.state, merged, undone.contains(&pull.number)),
            merged_at: pull.merged_at.clone(),
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
    data["skipped_no_pr"] = json!(skipped);
    Ok(Outcome::data(data))
}
