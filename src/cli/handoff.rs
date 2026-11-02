//! `bosun handoff [--from FILE]`: a root writes what the next root of its lineage needs to
//! continue (open PRs and leases from GitHub, the rest from the file) as one `handoff` record
//! on the registry. A handoff is never a lease: nothing here claims, renews or edits one
//! (spec §5.1).

use std::io::Read;
use std::path::Path;

use serde_json::json;

use super::HandoffArgs;
use crate::app::{App, Result, active_issues, kill_switch, trusted_notes};
use crate::core::handoff::{MAX_LINE, MAX_LINES, clean_line, is_root, parse_lines};
use crate::core::lease::leases_of;
use crate::core::meta::linked_issues;
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

    let root = app.ctx.origin != Origin::Spawned
        && match registry::load(gh)? {
            Some((number, _)) => is_root(&trusted_notes(gh, number)?, session),
            None => false,
        };
    if !root {
        return Err(BosunError::error(
            format!("{session} is not a joined root session; only a root hands off"),
            "run `bosun join` as a root (no --parent, an id without `/`), or report to your parent with `bosun result`",
        ));
    }

    // Only this session's own live leases, and the open PRs of those tasks.
    let mut leases = Vec::new();
    for (issue, notes) in active_issues(gh)? {
        for l in leases_of(issue.number, &notes) {
            if &l.session == session && l.is_live(now) {
                let touch = l.touch.join(" ");
                leases.push((
                    l.issue,
                    clean_line(&format!(
                        "lease: #{} {session} {touch} until {}",
                        l.issue, l.expires
                    )),
                ));
            }
        }
    }
    let mut lines: Vec<String> = Vec::new();
    for p in gh.list_pulls(None, IssueState::Open)? {
        if leases
            .iter()
            .any(|(i, _)| linked_issues(&p.body).contains(i))
        {
            let (n, r, h) = (p.number, &p.head_ref, &p.head_sha);
            lines.push(clean_line(&format!("pr: #{n} {r} {h}")));
        }
    }
    lines.extend(leases.into_iter().map(|(_, l)| l));
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
