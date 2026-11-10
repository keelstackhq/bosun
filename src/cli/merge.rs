//! `bosun merge <pr>` (spec D13 as amended by org decision D-019, §4): the orchestrator re-checks
//! every gate on the PR's current head, then merges with `sha = head`, so a push after the
//! checks makes GitHub refuse (exit 3). Without CI the gates are the ready summary, risk-check,
//! a clean verdict from another session (the owner's chat approval when gated) and main-open;
//! with CI they are the required checks. Every other agent merge path stays refused by the hook.

use std::path::Path;

use serde_json::json;

use crate::app::{App, default_branch, kill_switch, main_red, sync_merged};
use crate::cli::MergeArgs;
use crate::config::CiProvider;
use crate::core::approve::{is_orchestrator, parse_ready_summary};
use crate::core::meta::linked_issues_in;
use crate::gates::main_open::judge_local;
use crate::github::{Github, Pull};
use crate::output::{BosunError, CmdResult, Exit, Outcome};

pub fn run(dir: Option<&Path>, args: &MergeArgs) -> CmdResult {
    let app = App::load(dir)?;
    let gh = app.gh.as_ref();
    let pr = args.pr;
    kill_switch(gh)?;
    let session = &app.ctx.session;
    let members = crate::registry::load(gh)?
        .map(|(_, notes)| crate::core::lease::members(&notes))
        .unwrap_or_default();
    if !is_orchestrator(&members, session) {
        return Err(BosunError::needs_human(
            format!(
                "session {session} is not an orchestrator in the registry; only an orchestrator merges (spec D13)"
            ),
            format!(
                "hand PR #{pr} to the orchestrator with `bosun handoff-pr`; it runs `bosun merge {pr}`"
            ),
        ));
    }
    let pull = gh.get_pull(pr)?.ok_or_else(|| {
        BosunError::error(format!("PR #{pr} does not exist"), "check the PR number")
    })?;
    // Only a merge into the default branch lands a task (as `bosun pr` counts it), and the gates
    // (main-open, required checks) judge that branch.
    let base = default_branch(gh)?;
    if pull.base_ref != base {
        return Err(BosunError::conflict(
            format!(
                "PR #{pr} targets {}, not the default branch {base}; nothing was merged",
                pull.base_ref
            ),
            format!(
                "merge its base PR first and retarget PR #{pr} to {base}, then rerun `bosun merge {pr}`"
            ),
        ));
    }
    if pull.merged {
        let tasks = sync_tasks(&app, &pull)?;
        return Ok(Outcome::data(json!({
            "pr": pr, "head_sha": pull.head_sha, "merged": true, "already_merged": true, "tasks": tasks,
        })));
    }
    if pull.state != "open" {
        return Err(BosunError::conflict(
            format!("PR #{pr} is closed and not merged"),
            "reopen it, or name an open PR",
        ));
    }
    let provider = super::pr_no_ci::provider_at(gh, &pull.base_sha)?;
    let head = pull.head_sha.clone();
    if provider == CiProvider::None {
        check_no_ci(&app, &pull)?;
    } else {
        check_ci(gh, &pull, &base)?;
    }
    gh.merge_pull(pr, &head, &args.method)
        .map_err(|e| match e.exit {
            Exit::Conflict => e,
            _ if http_status(&e.message, 403) => BosunError::needs_human(
                e.message,
                format!("this session cannot merge through the API: the owner taps Merge on PR #{pr} (head {head}) in the GitHub app"),
            ),
            _ => e,
        })?;
    let mut fresh = pull.clone();
    fresh.merged = true;
    let tasks = sync_tasks(&app, &fresh)?;
    Ok(Outcome::data(json!({
        "pr": pr, "head_sha": head, "merged": true, "method": args.method,
        "ci_provider": if provider == CiProvider::None { "none" } else { "ci" }, "tasks": tasks,
    })))
}

/// No CI: a writer's ready summary for this head, then the same local checks `pr --ready` ran,
/// re-run now (the owner's chat answer may have come after the summary).
fn check_no_ci(app: &App, pull: &Pull) -> Result<(), BosunError> {
    let gh = app.gh.as_ref();
    let (pr, head) = (pull.number, pull.head_sha.as_str());
    let mut stale = false;
    let mut found = false;
    for c in gh.list_comments(pr)?.iter().rev() {
        if !c.body.starts_with("/bosun ready ") {
            continue;
        }
        if !matches!(gh.permission(&c.user)?.as_deref(), Some("admin" | "write")) {
            continue;
        }
        if parse_ready_summary(&c.body, head) {
            found = true;
            break;
        }
        stale = true;
    }
    if !found {
        let fix = format!(
            "the author's session pushes nothing more and reruns `bosun pr <task> --ready` on head {head}, then rerun `bosun merge {pr}`"
        );
        return Err(if stale {
            BosunError::conflict(
                format!(
                    "PR #{pr} head moved to {head} after its ready summary; nothing was merged"
                ),
                fix,
            )
        } else {
            BosunError::needs_human(
                format!("PR #{pr} has no ready summary for head {head}"),
                fix,
            )
        });
    }
    super::risk_check::evaluate_local(app, pull)?;
    if super::pr_no_ci::clean_by_other(app, pull)?.is_empty() {
        return Err(BosunError::needs_human(
            format!("PR #{pr} has no clean verdict on head {head} from another session"),
            format!(
                "a reviewer session runs `bosun review-pack {pr}` and posts `bosun verdict comment --from -`, then rerun `bosun merge {pr}`"
            ),
        ));
    }
    let main = judge_local(gh, pull, main_red(gh)?)?;
    if !main.pass {
        return Err(BosunError::halted(
            main.description,
            format!(
                "fix main with a bosun:kind/mainfix PR, or wait for main-red to close, then rerun `bosun merge {pr}`"
            ),
        ));
    }
    Ok(())
}

/// With CI: every required check green on the head, `risk-check` and `review-verdict` posted
/// by GitHub Actions (the case where GitHub cannot turn on auto-merge: already clean).
fn check_ci(gh: &dyn Github, pull: &Pull, base: &str) -> Result<(), BosunError> {
    let (pr, head) = (pull.number, pull.head_sha.as_str());
    let mut bad = super::pr::not_green(gh, base, head)?;
    for (ctx, state) in super::pr::merge_gates(gh, head)? {
        if state != "success" && !bad.iter().any(|b| b.0 == ctx) {
            bad.push((ctx, state));
        }
    }
    if bad.is_empty() {
        return Ok(());
    }
    let list: Vec<String> = bad.iter().map(|(c, s)| format!("{c} {s}")).collect();
    Err(BosunError::needs_human(
        format!("PR #{pr} head {head} is not green: {}", list.join(", ")),
        format!("`bosun wait --pr {pr}` until the checks pass (the owner approves a gated PR first), then rerun `bosun merge {pr}`"),
    )
    .with_data(json!({"pr": pr, "head_sha": head, "pending_checks": list})))
}

/// Move every linked task to merged and end its leases (as `bosun pr` does for a merged PR).
fn sync_tasks(app: &App, pull: &Pull) -> Result<Vec<u64>, BosunError> {
    let gh = app.gh.as_ref();
    let mut moved = Vec::new();
    for n in linked_issues_in(&pull.body, &app.ctx.repo.to_string()) {
        if let Some(issue) = gh.get_issue(n)?
            && sync_merged(gh, n, &issue, None)?
        {
            moved.push(n);
        }
    }
    Ok(moved)
}

/// Whether a GitHub error message (`GitHub PUT <path>: <status> <message>`, trimmed) names `status`.
pub(crate) fn http_status(message: &str, status: u16) -> bool {
    let code = format!(": {status}");
    message.contains(&format!("{code} ")) || message.ends_with(&code)
}
