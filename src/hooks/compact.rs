//! The bosun state block that survives compaction (spec §4): `bosun hook pre-compact` prints it
//! for the summary, and `bosun hook session-start` with source `compact` adds it back. It never
//! blocks compaction and prints no secret: only ids, globs, branch names and shas. Leases come
//! from the lease cache only; the one GitHub call (the PR lookup) waits at most `PR_WAIT`.

use std::path::Path;
use std::sync::mpsc;
use std::time::Duration;

use super::{cache, git_out, refusal};
use crate::app::App;
use crate::core::lease::Lease;
use crate::core::refusal::{context, mine};
use crate::github::IssueState;
use crate::time::Timestamp;

/// The longest wait for the PR lookup; the settings entry caps the whole hook at 15 s.
const PR_WAIT: Duration = Duration::from_secs(5);

/// PreCompact: plain text, which Claude Code adds to the summary instructions. An error drops
/// only what it touches; a panic prints nothing. The hook always exits 0.
pub fn pre_compact(cwd: Option<&Path>) -> Option<String> {
    let dir = cwd
        .map(Path::to_path_buf)
        .or_else(|| std::env::current_dir().ok())?;
    safe_block(&dir).map(|b| format!("Keep this bosun state verbatim in the summary.\n{b}"))
}

/// `state_block`, or `None` after a panic (logged to stderr), so no hook loses its output.
pub fn safe_block(dir: &Path) -> Option<String> {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(|_| {
        eprintln!("bosun: internal error in the compaction state block; skipping it");
    }));
    let block = std::panic::catch_unwind(|| state_block(dir)).ok();
    std::panic::set_hook(previous);
    block
}

/// An open PR of the branch: number, head sha, draft.
struct OpenPr {
    number: u64,
    head: String,
    draft: bool,
}

/// The block: session, own leases, branch and HEAD, open PR, last refusal, next step.
fn state_block(dir: &Path) -> String {
    let me = refusal::caller();
    let branch = git_out(dir, &["rev-parse", "--abbrev-ref", "HEAD"]);
    let head = git_out(dir, &["rev-parse", "HEAD"]);
    let (mut leases, pr) = remote(dir, &me, branch.as_deref());
    // Newest lease first: the next step names it.
    leases.sort_by_key(|l| std::cmp::Reverse(l.expires.0));
    let branch = branch.map(|b| if b == "HEAD" { "(detached)".into() } else { b });
    let last = refusal::latest(dir);

    let mut lines = vec![format!("bosun state (session {me}):")];
    if leases.is_empty() {
        lines.push("- leases: none".into());
    }
    for l in &leases {
        lines.push(format!(
            "- lease #{} as {} touch {} (expires {})",
            l.issue,
            l.session,
            l.touch.join(" "),
            l.expires
        ));
    }
    let short = head.as_deref().map(|h| &h[..h.len().min(12)]);
    lines.push(format!(
        "- branch {} at {}",
        branch.as_deref().unwrap_or("unknown"),
        short.unwrap_or("unknown")
    ));
    lines.push(match &pr {
        Some(p) => format!(
            "- PR #{} head {}{}",
            p.number,
            p.head,
            if p.draft { " (draft)" } else { "" }
        ),
        None => "- PR: none open for this branch (or GitHub unreachable)".into(),
    });
    if let Some(r) = &last {
        lines.push(format!("- last {}", context(r)));
    }
    lines.push(format!(
        "Next: {}",
        next_step(&leases, pr.as_ref(), last.is_some())
    ));
    lines.join("\n")
}

/// Own leases and the branch's open PR. Leases come from the cache alone; the PR needs GitHub.
fn remote(dir: &Path, me: &str, branch: Option<&str>) -> (Vec<Lease>, Option<OpenPr>) {
    let leases = cache::cached_live(dir, now())
        .into_iter()
        .filter(|l| mine(&l.session, me))
        .collect();
    let pr = branch
        .filter(|b| !["main", "master", "HEAD"].contains(b))
        .and_then(|b| open_pr(dir, b));
    (leases, pr)
}

/// The branch's open PR from GitHub, in a thread so a slow API costs at most `PR_WAIT`.
fn open_pr(dir: &Path, branch: &str) -> Option<OpenPr> {
    let (tx, rx) = mpsc::channel();
    let (dir, branch) = (dir.to_path_buf(), branch.to_string());
    std::thread::spawn(move || {
        let lookup = || -> Result<Option<OpenPr>, String> {
            let app = App::load(Some(&dir)).map_err(|e| e.message)?;
            let head = format!("{}:{branch}", app.ctx.repo.owner);
            let found = app.gh.list_pulls(Some(&head), IssueState::Open);
            Ok(found
                .map_err(|e| e.message)?
                .into_iter()
                .next()
                .map(|p| OpenPr {
                    number: p.number,
                    head: p.head_sha,
                    draft: p.draft,
                }))
        };
        let _ = tx.send(lookup());
    });
    match rx.recv_timeout(PR_WAIT) {
        Ok(Ok(pr)) => pr,
        Ok(Err(e)) => {
            eprintln!("bosun: warning: compaction state: {e}");
            None
        }
        Err(mpsc::RecvTimeoutError::Disconnected) => {
            eprintln!("bosun: warning: compaction state: the PR lookup thread died");
            None
        }
        Err(mpsc::RecvTimeoutError::Timeout) => {
            eprintln!("bosun: warning: compaction state: no PR lookup within {PR_WAIT:?}");
            None
        }
    }
}

fn now() -> Timestamp {
    let env = std::env::var("BOSUN_NOW").ok();
    // A bad BOSUN_NOW falls back to the clock, never to 0 (which would keep expired leases).
    Timestamp::resolve(env.as_deref())
        .or_else(|_| Timestamp::resolve(None))
        .unwrap_or(Timestamp(i64::MAX))
}

/// The next step of the task loop (AGENTS.md) for this state.
fn next_step(leases: &[Lease], pr: Option<&OpenPr>, refused: bool) -> String {
    if refused {
        return "do what the refusal's fix says first, then `bosun status`.".into();
    }
    let Some(l) = leases.first() else {
        return "run `bosun status`, then `bosun claim <issue>` (or post your `bosun result`)."
            .into();
    };
    let n = l.issue;
    match pr {
        None => format!(
            "finish #{n} test first, run the gates (`bosun gate <g>`), push once, then `bosun pr {n}`."
        ),
        Some(p) if p.draft => format!(
            "review PR #{} locally (`bosun review-pack {}`), then `bosun pr {n} --ready`.",
            p.number, p.number
        ),
        Some(p) => format!(
            "PR #{} is ready: run `bosun heartbeat`, wait for checks, then `bosun result {n}`.",
            p.number
        ),
    }
}
