//! `bosun handoff [--from FILE]`: a root writes what the next root of its lineage needs to
//! continue (open PRs and leases from GitHub, the rest from the file) as one `handoff` record
//! on the registry. A handoff is never a lease: nothing here claims, renews or edits one
//! (spec §5.1).

use std::io::Read;
use std::path::Path;

use serde_json::json;

use super::HandoffArgs;
use crate::app::{App, Result, active_issues, kill_switch};
use crate::core::handoff::{MAX_LINE, MAX_LINES, clean_line, is_child, parse_lines};
use crate::core::lease::leases_of;
use crate::core::records::Record;
use crate::core::text::inert;
use crate::ctx::Origin;
use crate::github::IssueState;
use crate::output::{BosunError, CmdResult, Outcome};
use crate::registry;

pub fn run(dir: Option<&Path>, args: &HandoffArgs) -> CmdResult {
    let app = App::load(dir)?;
    let gh = app.gh.as_ref();
    kill_switch(gh)?;
    let (session, now) = (&app.ctx.session, app.ctx.now);

    let child = app.ctx.origin == Origin::Spawned
        || registry::load(gh)?.is_some_and(|(_, notes)| is_child(&notes, session));
    if child {
        return Err(BosunError::error(
            format!("{session} is a child session; only a root hands off"),
            "report to your parent with `bosun result` instead",
        ));
    }

    let mut lines: Vec<String> = Vec::new();
    for p in gh.list_pulls(None, IssueState::Open)? {
        lines.push(clean_line(&format!(
            "pr: #{} {} {}",
            p.number, p.head_ref, p.head_sha
        )));
    }
    for (issue, notes) in active_issues(gh)? {
        for l in leases_of(issue.number, &notes) {
            if l.is_live(now) {
                lines.push(clean_line(&format!(
                    "lease: #{} {} {} until {}",
                    l.issue,
                    l.session,
                    l.touch.join(" "),
                    l.expires
                )));
            }
        }
    }
    if let Some(from) = &args.from {
        let text = read_input(from)?;
        lines.extend(parse_lines(&text).map_err(|e| {
            BosunError::usage(
                format!("{from}: {e}"),
                "write one `<kind>: <text>` line per item (kinds: wave, pr, lease, next, blocker)",
            )
        })?);
    }
    if lines.len() > MAX_LINES {
        return Err(BosunError::limit(
            format!(
                "the handoff has {} lines; the limit is {MAX_LINES}",
                lines.len()
            ),
            "cut the handoff to at most 40 lines: keep one line per item, drop done items",
        )
        .with_data(json!({"lines": lines.len(), "max": MAX_LINES})));
    }

    let (issue, _) = registry::ensure(gh)?;
    let record = Record::Handoff {
        session: session.clone(),
        at: now.to_string(),
        lines: lines.clone(),
    };
    let list: String = lines
        .iter()
        .map(|l| format!("- {}\n", inert(l, MAX_LINE)))
        .collect();
    let body = format!(
        "{} hands off to the next root ({} lines):\n\n{list}\n{}",
        inert(session, 100),
        lines.len(),
        record.to_line()
    );
    let comment = gh.add_comment(issue, &body)?;
    Ok(Outcome::data(json!({"session": session, "issue": issue,
        "comment": comment.id, "lines": lines, "count": lines.len()})))
}

fn read_input(from: &str) -> Result<String> {
    let mut text = String::new();
    let read = if from == "-" {
        std::io::stdin().read_to_string(&mut text).map(|_| ())
    } else {
        std::fs::read_to_string(from).map(|t| text = t)
    };
    read.map_err(|e| {
        BosunError::usage(
            format!("cannot read {from}: {e}"),
            "pass the handoff lines on stdin with --from -, or a file path",
        )
    })?;
    Ok(text)
}
