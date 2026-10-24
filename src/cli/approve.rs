//! `bosun approve`: the owner, in the owner's own Claude Code session, lists the PRs that wait
//! for an owner and approves them with the comment `/bosun approve <head>` (spec section 8).
//! Every GitHub call uses `BOSUN_OWNER_TOKEN`, never `GITHUB_TOKEN`: the comment must come from
//! a plain user token, which agent sessions do not have.

use std::path::Path;

use serde_json::{Value, json};

use super::ApproveArgs;
use crate::app::{Result, kill_switch, main_red};
use crate::core::approve::{Readiness, Run, Stat, readiness};
use crate::core::records::{PrHead, Record};
use crate::ctx::Ctx;
use crate::gates::main_open::{is_mainfix, judge};
use crate::github::{Github, IssueState, LiveGithub};
use crate::output::{BosunError, CmdResult, Outcome};
use crate::registry;

const TOKEN_VAR: &str = "BOSUN_OWNER_TOKEN";

struct Ready {
    pr: u64,
    title: String,
    head_sha: String,
    risk: Vec<String>,
    files: Vec<String>,
}

struct Board {
    ready: Vec<Ready>,
    waiting: Vec<(u64, &'static str)>,
}

/// The open PRs whose risk-check waits for an owner, split into ready and waiting, by number.
fn board(gh: &dyn Github) -> Result<Board> {
    let mut pulls = gh.list_pulls(None, IssueState::Open)?;
    pulls.sort_by_key(|p| p.number);
    let mut board = Board {
        ready: Vec::new(),
        waiting: Vec::new(),
    };
    for p in pulls {
        let statuses: Vec<Stat> = gh
            .statuses(&p.head_sha)?
            .into_iter()
            .map(|s| Stat {
                context: s.context,
                state: s.state,
                description: s.description,
            })
            .collect();
        let runs: Vec<Run> = gh
            .check_runs(&p.head_sha)?
            .into_iter()
            .map(|r| Run {
                id: r.id,
                name: r.name,
                conclusion: r.conclusion,
                at: r.completed_at.or(r.started_at).unwrap_or_default(),
            })
            .collect();
        match readiness(&statuses, &runs) {
            Readiness::NotWaiting => {}
            Readiness::Waiting(why) => board.waiting.push((p.number, why)),
            Readiness::Ready(risk) => board.ready.push(Ready {
                pr: p.number,
                title: p.title.clone(),
                files: gh
                    .pull_files(p.number)?
                    .into_iter()
                    .map(|f| f.filename)
                    .collect(),
                head_sha: p.head_sha,
                risk,
            }),
        }
    }
    Ok(board)
}

pub fn run(dir: Option<&Path>, args: &ApproveArgs) -> CmdResult {
    let modes = [
        args.list,
        args.all_ready,
        args.main_open.is_some(),
        !args.prs.is_empty(),
    ];
    if !modes.contains(&true) {
        return Err(BosunError::usage(
            "approve needs PR numbers, --all-ready, --list or --main-open PR",
            "run `bosun approve --list`, then `bosun approve <pr>...` or `bosun approve --all-ready`",
        ));
    }
    let token = std::env::var(TOKEN_VAR)
        .ok()
        .filter(|t| !t.trim().is_empty())
        .ok_or_else(|| {
            BosunError::needs_human(
                format!("the owner's token is missing: {TOKEN_VAR} is not set"),
                format!(
                    "bosun approve runs only in the owner's own Claude Code environment with {TOKEN_VAR} set (see docs/PROGRESS.md, Blocked: Owner approvals); it never uses GITHUB_TOKEN"
                ),
            )
        })?;
    let ctx = Ctx::load(dir)?;
    let gh = LiveGithub::with_token(token.trim(), &ctx.repo.owner, &ctx.repo.name);
    let now = ctx.now.to_string();

    if args.list {
        let b = board(&gh)?;
        return Ok(Outcome::data(json!({
            "ready": b.ready.iter().map(ready_json).collect::<Vec<_>>(),
            "waiting": b.waiting.iter().map(|(pr, why)| json!({"pr": pr, "why": why}))
                .collect::<Vec<_>>(),
        })));
    }
    kill_switch(&gh)?;
    if let Some(n) = args.main_open {
        let head = post_main_open(&gh, n)?;
        write_record(&gh, &[], std::slice::from_ref(&head), &now)?;
        return Ok(Outcome::data(
            json!({"main_open": {"pr": n, "head_sha": head.head_sha}}),
        ));
    }

    let b = board(&gh)?;
    let chosen: Vec<&Ready> = if args.all_ready {
        b.ready.iter().collect()
    } else {
        let mut wanted = args.prs.clone();
        wanted.sort_unstable();
        wanted.dedup();
        let mut not_ready = Vec::new();
        let mut chosen = Vec::new();
        for n in wanted {
            if let Some(r) = b.ready.iter().find(|r| r.pr == n) {
                chosen.push(r);
            } else {
                not_ready.push(json!({"pr": n, "why": why_not(&gh, &b, n)?}));
            }
        }
        if !not_ready.is_empty() {
            return Err(BosunError::conflict(
                "some of the PRs are not ready for an owner; nothing was posted",
                "run `bosun approve --list` and name only PRs in `ready`",
            )
            .with_data(json!({"not_ready": not_ready})));
        }
        chosen
    };
    let mut approved = Vec::new();
    let mut heads = Vec::new();
    for r in chosen {
        let c = gh.add_comment(r.pr, &format!("/bosun approve {}", r.head_sha))?;
        approved.push(json!({"pr": r.pr, "head_sha": r.head_sha, "comment_id": c.id}));
        heads.push(PrHead {
            pr: r.pr,
            head_sha: r.head_sha.clone(),
        });
    }
    if !heads.is_empty() {
        write_record(&gh, &heads, &[], &now)?;
    }
    Ok(Outcome::data(json!({"approved": approved})))
}

fn ready_json(r: &Ready) -> Value {
    json!({"pr": r.pr, "title": r.title, "head_sha": r.head_sha, "risk": r.risk,
        "files": r.files})
}

/// Why `n` is not in the ready list.
fn why_not(gh: &dyn Github, b: &Board, n: u64) -> Result<String> {
    if let Some((_, why)) = b.waiting.iter().find(|(pr, _)| *pr == n) {
        return Ok((*why).to_string());
    }
    Ok(match gh.get_pull(n)? {
        None => "not found".into(),
        Some(p) if p.state != "open" => "closed".into(),
        Some(_) => "not waiting for an owner".into(),
    })
}

/// `bosun gate main-open --pr N`, posted with the owner token: the same `judge`. Exit 4 when
/// main is red and the PR is no mainfix; exit 3 when the PR is closed or does not exist.
fn post_main_open(gh: &dyn Github, n: u64) -> Result<PrHead> {
    let pull = gh
        .get_pull(n)?
        .filter(|p| p.state == "open")
        .ok_or_else(|| {
            BosunError::conflict(
                format!("PR #{n} is not an open PR"),
                "name an open PR; `bosun approve --list` lists the PRs that wait for an owner",
            )
        })?;
    let red = main_red(gh)?;
    let mainfix = red.is_some() && is_mainfix(gh, &pull)?;
    let v = judge(red, mainfix);
    let state = if v.pass { "success" } else { "failure" };
    let description = format!("{} (posted by bosun approve --main-open)", v.description);
    gh.create_status(&pull.head_sha, "main-open", state, &description, None)?;
    if v.pass {
        return Ok(PrHead {
            pr: n,
            head_sha: pull.head_sha,
        });
    }
    let red = red.unwrap_or_default();
    Err(BosunError::halted(
        v.description,
        format!("fix main with a bosun:kind/mainfix PR, or wait for #{red} to close"),
    )
    .with_data(json!({"pr": n, "head_sha": pull.head_sha, "main_red": red, "pass": false})))
}

/// One `approve` record on the registry issue.
fn write_record(gh: &dyn Github, prs: &[PrHead], main_open: &[PrHead], at: &str) -> Result<()> {
    let (issue, _) = registry::ensure(gh)?;
    let record = Record::Approve {
        prs: prs.to_vec(),
        main_open: main_open.to_vec(),
        at: at.into(),
    };
    gh.add_comment(issue, &record.to_line())?;
    Ok(())
}
