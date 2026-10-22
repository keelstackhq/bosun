//! `bosun wait`: block until a PR, an issue or a child's result changes (spec §5, D14). One
//! call polls GitHub with backoff; it never uses send_later or send_message.

use std::collections::BTreeMap;
use std::path::Path;
use std::time::{Duration, Instant};

use serde_json::{Value, json};

use super::WaitArgs;
use crate::app::{App, kill_switch, notes};
use crate::core::records::{Record, parse_record};
use crate::core::state::state_of;
use crate::github::{Github, IssueState};
use crate::output::{BosunError, CmdResult, Outcome};

/// Base delay between polls in milliseconds; `BOSUN_WAIT_BASE_MS` overrides it (tests).
const BASE_MS: u64 = 5000;
/// The delay stops doubling at 12 times the base (5 s becomes 60 s).
const CAP_FACTOR: u64 = 12;

enum Target {
    Pr(u64),
    Issue(u64),
    Child(String),
}

pub fn run(dir: Option<&Path>, args: &WaitArgs) -> CmdResult {
    let target = match (args.pr, args.issue, &args.child) {
        (Some(n), None, None) => Target::Pr(n),
        (None, Some(n), None) => Target::Issue(n),
        (None, None, Some(s)) => Target::Child(s.clone()),
        _ => {
            return Err(BosunError::usage(
                "wait needs exactly one of --pr, --issue, --child",
                "run `bosun wait --pr N`, `--issue N` or `--child SESSION`",
            ));
        }
    };
    let app = App::load(dir)?;
    let gh = app.gh.as_ref();
    kill_switch(gh)?;
    let base = std::env::var("BOSUN_WAIT_BASE_MS")
        .ok()
        .and_then(|v| v.trim().parse::<u64>().ok())
        .unwrap_or(BASE_MS);
    let (mut delay, cap) = (base, base * CAP_FACTOR);
    let deadline = Instant::now() + Duration::from_secs(args.timeout);

    let before = snapshot(gh, &target)?;
    let (mut polls, mut after, mut changed) = (1u64, before.clone(), false);
    while Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(delay).min(deadline - Instant::now()));
        after = snapshot(gh, &target)?;
        polls += 1;
        if after != before {
            changed = true;
            break;
        }
        delay = (delay * 2).min(cap);
    }
    Ok(Outcome::data(json!({
        "changed": changed, "polls": polls, "before": before, "after": after,
    })))
}

fn snapshot(gh: &dyn Github, target: &Target) -> Result<Value, BosunError> {
    match target {
        Target::Pr(n) => {
            let pr = gh.get_pull(*n)?.ok_or_else(|| {
                BosunError::error(
                    format!("PR #{n} does not exist"),
                    "check the PR number; `bosun status` lists the open PRs",
                )
            })?;
            let statuses: BTreeMap<String, String> = gh
                .statuses(&pr.head_sha)?
                .into_iter()
                .map(|s| (s.context, s.state))
                .collect();
            let checks: BTreeMap<String, Option<String>> = gh
                .check_runs(&pr.head_sha)?
                .into_iter()
                .map(|c| (c.name, c.conclusion))
                .collect();
            Ok(
                json!({"state": pr.state, "merged": pr.merged, "head_sha": pr.head_sha,
                "statuses": statuses, "checks": checks}),
            )
        }
        Target::Issue(n) => {
            let issue = gh.get_issue(*n)?.ok_or_else(|| {
                BosunError::error(
                    format!("issue #{n} does not exist"),
                    "check the issue number",
                )
            })?;
            Ok(json!({"state": state_of(&issue.labels).map(|s| s.name()),
                "open": issue.state == "open",
                "comments": gh.list_comments(*n)?.len()}))
        }
        Target::Child(session) => {
            let mut newest = Value::Null;
            for issue in gh.list_issues(&[], IssueState::Open)? {
                for note in notes(gh, issue.number)? {
                    if let Some(Record::Result {
                        session: s,
                        outcome,
                        pr,
                    }) = parse_record(&note.body)
                        && &s == session
                    {
                        newest = json!({"issue": issue.number, "comment_id": note.id,
                            "outcome": outcome, "pr": pr});
                    }
                }
            }
            Ok(json!({"result": newest}))
        }
    }
}
