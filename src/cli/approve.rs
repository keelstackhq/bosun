//! `bosun approve`: the owner, in the owner's own Claude Code session, lists the PRs that wait
//! for an owner and approves them with the comment `/bosun approve <head>` (spec section 8).
//! Every GitHub call uses `BOSUN_OWNER_TOKEN`, never `GITHUB_TOKEN`: the comment must come from
//! a plain user token, which agent sessions do not have. The owner names the head sha of each
//! PR (`<pr>@<sha>`, from `--list`), so a push after the listing is never approved unseen.

use std::path::Path;

use serde_json::{Value, json};

use super::ApproveArgs;
use crate::app::{Result, kill_switch, main_red};
use crate::core::approve::{
    Readiness, Run, Stat, api_url_ok, approve_command, parse_target, readiness,
};
use crate::core::records::{PrHead, Record};
use crate::ctx::Ctx;
use crate::gates::main_open::{is_mainfix, judge};
use crate::github::{Github, IssueState, LiveGithub};
use crate::output::{BosunError, CmdResult, Outcome};
use crate::registry;

const TOKEN_VAR: &str = "BOSUN_OWNER_TOKEN";
const MAIN_OPEN_WORKFLOW: &str = ".github/workflows/main-open.yml";

struct Ready {
    pr: u64,
    title: String,
    head_sha: String,
    risk: Vec<String>,
    files: Vec<String>,
}

/// GitHub's compare API lists at most this many files and does not say when it cut the list.
const COMPARE_FILES_CAP: usize = 300;

/// Shown with every list: an approval counts only as a plain owner comment, which this command
/// posts. The Claude GitHub tool posts through an app and adds a footer, so its comments never count.
const LIST_HINT: &str =
    "post approvals only with this command; comments from Claude's GitHub tool never count";

struct Board {
    ready: Vec<Ready>,
    waiting: Vec<(u64, &'static str)>,
}

/// The open PRs whose risk-check waits for an owner, split into ready and waiting, by number.
/// Statuses, check runs and files all describe the head sha read with the PR.
fn board(gh: &dyn Github) -> Result<Board> {
    let mut pulls = gh.list_pulls(None, IssueState::Open)?;
    pulls.sort_by_key(|p| p.number);
    let mut board = Board {
        ready: Vec::new(),
        waiting: Vec::new(),
    };
    for p in pulls {
        let statuses: Vec<Stat> = gh
            .status_history(&p.head_sha)?
            .into_iter()
            .map(|s| Stat {
                context: s.context,
                state: s.state,
                description: s.description,
                creator: s.creator,
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
                app: r.app,
            })
            .collect();
        match readiness(&statuses, &runs) {
            Readiness::NotWaiting => {}
            Readiness::Waiting(why) => board.waiting.push((p.number, why)),
            Readiness::Ready(risk) => board.ready.push(Ready {
                pr: p.number,
                title: p.title.clone(),
                files: gh
                    .compare_files(&p.base_sha, &p.head_sha)?
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
    if !args.list && args.main_open.is_none() && args.prs.is_empty() {
        return Err(BosunError::usage(
            "approve needs PR@HEAD arguments, --list or --main-open PR",
            "run `bosun approve --list`, then the command in its data.approve_command",
        ));
    }
    let mut targets = Vec::new();
    for a in &args.prs {
        targets.push(parse_target(a).ok_or_else(|| {
            BosunError::usage(
                format!("{a:?} is not <pr>@<40-hex head sha>"),
                "run `bosun approve --list` and run the command in its data.approve_command",
            )
        })?);
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
    let token = token.trim().to_string();
    let api = std::env::var("BOSUN_GITHUB_API").unwrap_or_else(|_| "https://api.github.com".into());
    if !api_url_ok(&api) {
        return Err(BosunError::error(
            "BOSUN_GITHUB_API is not https://, so the owner token is not sent",
            "unset BOSUN_GITHUB_API (or set it to an https:// URL) in the owner's environment",
        ));
    }
    let ctx = Ctx::load(dir)?;
    let gh = LiveGithub::with_token(&token, &ctx.repo.owner, &ctx.repo.name);
    let now = ctx.now.to_string();
    redact(&token, exec(&gh, args, targets, &now))
}

fn exec(
    gh: &dyn Github,
    args: &ApproveArgs,
    mut targets: Vec<(u64, String)>,
    now: &str,
) -> CmdResult {
    if args.list {
        let b = board(gh)?;
        let heads: Vec<(u64, String)> =
            b.ready.iter().map(|r| (r.pr, r.head_sha.clone())).collect();
        return Ok(Outcome::data(json!({
            "ready": b.ready.iter().map(ready_json).collect::<Vec<_>>(),
            "waiting": b.waiting.iter().map(|(pr, why)| json!({"pr": pr, "why": why}))
                .collect::<Vec<_>>(),
            "approve_command": approve_command(&heads),
            "hint": LIST_HINT,
        })));
    }
    kill_switch(gh)?;
    if let Some(n) = args.main_open {
        let head = post_main_open(gh, n)?;
        write_record(gh, &[], std::slice::from_ref(&head), now)?;
        return Ok(Outcome::data(
            json!({"main_open": {"pr": n, "head_sha": head.head_sha}}),
        ));
    }

    let b = board(gh)?;
    targets.sort_unstable();
    targets.dedup();
    let mut not_ready = Vec::new();
    for (n, sha) in &targets {
        let why = match b.ready.iter().find(|r| r.pr == *n) {
            Some(r) if r.head_sha == *sha => continue,
            Some(r) => format!("head changed (now {})", r.head_sha),
            None => why_not(gh, &b, *n)?,
        };
        not_ready.push(json!({"pr": n, "why": why}));
    }
    if !not_ready.is_empty() {
        return Err(BosunError::conflict(
            "some of the PRs are not ready at the head you named; nothing was posted",
            "run `bosun approve --list` again and run the command in its data.approve_command",
        )
        .with_data(json!({"not_ready": not_ready})));
    }
    let mut approved = Vec::new();
    let mut heads = Vec::new();
    let mut failed = None;
    for (pr, sha) in targets {
        match gh.add_comment(pr, &format!("/bosun approve {sha}")) {
            Ok(c) => {
                approved.push(json!({"pr": pr, "head_sha": sha, "comment_id": c.id}));
                heads.push(PrHead { pr, head_sha: sha });
            }
            Err(e) => {
                failed = Some(e);
                break;
            }
        }
    }
    // The comments that were posted are recorded even when a later one failed.
    let recorded = if heads.is_empty() {
        Ok(())
    } else {
        write_record(gh, &heads, &[], now)
    };
    if let Some(e) = failed {
        return Err(e.with_data(json!({"approved": approved})));
    }
    recorded.map_err(|e| e.with_data(json!({"approved": approved})))?;
    Ok(Outcome::data(json!({"approved": approved})))
}

fn ready_json(r: &Ready) -> Value {
    json!({"pr": r.pr, "title": r.title, "head_sha": r.head_sha,
        "risk_from_status": r.risk, "files": r.files,
        "files_truncated": r.files.len() >= COMPARE_FILES_CAP})
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

/// `bosun gate main-open --pr N`, posted with the owner token: the same `judge`. Only for a PR
/// that changes the main-open workflow (the base workflow cannot judge it). Exit 4 when main is
/// red and the PR is no mainfix; exit 3 when the PR is closed, missing or changes no such file.
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
    if !gh
        .pull_files(n)?
        .iter()
        .any(|f| f.filename == MAIN_OPEN_WORKFLOW)
    {
        return Err(BosunError::conflict(
            format!("PR #{n} does not change {MAIN_OPEN_WORKFLOW}"),
            "the main-open workflow posts this status for other PRs; re-run it, or wait for it",
        ));
    }
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

/// One `approve` record on the registry issue: an audit hint, never authority.
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

/// Belt and braces: a server that echoes the token in a message or a title never gets it to
/// stdout or stderr.
fn redact(token: &str, result: CmdResult) -> CmdResult {
    let scrub = |s: &str| s.replace(token, "[redacted]");
    let scrub_json = |v: Value| serde_json::from_str(&scrub(&v.to_string())).unwrap_or(Value::Null);
    match result {
        Ok(mut o) => {
            // `next` is not scrubbed: approve never sets it.
            debug_assert!(o.next.is_empty(), "approve returns no next items");
            o.data = scrub_json(o.data);
            Ok(o)
        }
        Err(mut e) => {
            e.message = scrub(&e.message);
            e.fix = scrub(&e.fix);
            e.data = e.data.map(scrub_json);
            Err(e)
        }
    }
}
