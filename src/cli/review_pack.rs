//! `bosun review-pack <pr>`: everything a reviewer needs, as one JSON object (spec §7).

use std::collections::HashSet;
use std::path::Path;

use serde_json::{Value, json};

use super::PrArg;
use crate::app::{App, notes, task_meta};
use crate::core::lessons;
use crate::core::records::{Record, parse_record};
use crate::core::review::use_cases;
use crate::core::verdict::closes_issue;
use crate::output::{BosunError, CmdResult, Outcome};

/// Distinct older heads the delta scan asks about before it gives up.
const MAX_SCAN: usize = 20;
/// GitHub lists at most this many commits of a PR.
const MAX_PR_COMMITS: usize = 250;

const INSTRUCTIONS: &str = "Return JSON for `bosun verdict post` (cloud sessions: `bosun verdict comment`): {\"pr\":N,\"head_sha\":\"<head_sha>\",\
\"verdict\":\"approve\"|\"request-changes\",\"confidence\":0..1,\"findings\":[{\"severity\":\
\"blocker\"|\"major\"|\"minor\"|\"nit\",\"file\":\"\",\"line\":N,\"claim\":\"\",\"evidence\":\"\"}],\
\"lessons_hit\":[\"L-NNN\"],\"escalate_risk\":false}. Approve needs confidence >= 0.85 and no blocker.";

pub fn run(dir: Option<&Path>, args: &PrArg) -> CmdResult {
    let app = App::load(dir)?;
    let gh = app.gh.as_ref();
    let n = args.pr;
    let pull = gh.get_pull(n)?.ok_or_else(|| {
        BosunError::error(
            format!("PR #{n} does not exist"),
            "check the number; `gh pr list` shows the open PRs",
        )
    })?;
    // `since` is the newest older head with a verdict record and a successful `review-verdict`
    // status: the record is a comment anyone with write access can post, the status is not.
    // Records on the current head and on heads without a success status are skipped. Each sha
    // is asked once, and at most MAX_SCAN of them.
    let mut since = None;
    let mut delta_fallback = None;
    let mut seen: HashSet<String> = HashSet::new();
    for c in notes(gh, n)?.iter().rev() {
        if let Some(Record::Verdict { head_sha, .. }) = parse_record(&c.body)
            && head_sha != pull.head_sha
            && seen.insert(head_sha.clone())
        {
            if seen.len() > MAX_SCAN {
                delta_fallback = Some("too many verdict records".to_string());
                break;
            }
            if gh
                .statuses(&head_sha)?
                .iter()
                .any(|s| s.context == "review-verdict" && s.state == "success")
            {
                since = Some(head_sha);
                break;
            }
        }
    }
    // A delta needs since to be an ancestor of the head (`ahead`: the three-dot compare of a
    // behind or diverged sha is empty or misleading). The old sha can also be gone after a
    // force-push. Any problem means a full review, the safe direction.
    let files = match &since {
        Some(s) => {
            let delta: Result<Vec<_>, String> = match gh.compare(s, &pull.head_sha) {
                Ok(c) if c.status == "ahead" => {
                    // An ancestor can still belong to another PR (a stack): it must be one of
                    // this PR's commits. GitHub lists at most 250, so a full list proves nothing.
                    let commits = gh.pull_commits(n)?;
                    if commits.len() < MAX_PR_COMMITS && commits.contains(s) {
                        Ok(c.files)
                    } else {
                        Err(format!("{s} is not a commit of this PR"))
                    }
                }
                Ok(c) => Err(format!(
                    "{s} is not an ancestor of the head ({})",
                    if c.status.is_empty() {
                        "unknown"
                    } else {
                        &c.status
                    }
                )),
                Err(e) => Err(format!(
                    "cannot compare {s}...{}: {}",
                    pull.head_sha, e.message
                )),
            };
            match delta {
                Ok(f) => f,
                Err(why) => {
                    eprintln!("bosun: full review instead of a delta: {why}");
                    delta_fallback = Some(why);
                    since = None;
                    gh.pull_files(n)?
                }
            }
        }
        None => gh.pull_files(n)?,
    };
    let names: Vec<String> = files.iter().map(|f| f.filename.clone()).collect();

    let mut task = Value::Null;
    let mut ucs: Vec<String> = vec![];
    if let Some(num) = closes_issue(&pull.body)
        && let Some(issue) = gh.get_issue(num)?
    {
        let meta = task_meta(&issue);
        if let Some(epic) = meta.as_ref().and_then(|m| m.epic)
            && let Some(e) = gh.get_issue(epic)?
        {
            ucs = use_cases(e.body_text());
        }
        task = json!({"issue": num, "title": issue.title, "body": issue.body_text(),
            "meta": meta});
    }
    let store = lessons::parse(gh.file_at("docs/lessons.toml", &pull.base_sha)?.as_deref())
        .map_err(|m| BosunError::error(m, "fix docs/lessons.toml on the base branch"))?;
    let data = json!({
        "pr": n, "title": pull.title, "head_sha": pull.head_sha, "base_sha": pull.base_sha,
        "since_sha": since, "files": files.iter().map(|f| json!({
            "filename": f.filename, "status": f.status, "additions": f.additions,
            "deletions": f.deletions, "patch": f.patch})).collect::<Vec<_>>(),
        "task": task, "ucs": ucs, "lessons": lessons::matching(&store, &names),
        "review_md": gh.file_at("REVIEW.md", &pull.base_sha)?,
        "instructions": INSTRUCTIONS, "delta_fallback": delta_fallback,
    });
    Ok(Outcome::data(data))
}
