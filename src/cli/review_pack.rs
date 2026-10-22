//! `bosun review-pack <pr>`: everything a reviewer needs, as one JSON object (spec §7).

use std::path::Path;

use serde_json::{Value, json};

use super::PrArg;
use crate::app::{App, notes, task_meta};
use crate::core::records::{Record, parse_record};
use crate::core::review::{lessons_for, parse_lessons, use_cases};
use crate::core::verdict::closes_issue;
use crate::output::{BosunError, CmdResult, Outcome};

const INSTRUCTIONS: &str = "Return JSON for `bosun verdict post`: {\"pr\":N,\"head_sha\":\"<head_sha>\",\
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
    // The newest verdict on an older head makes this a delta review, but only when that head
    // carries a successful `review-verdict` status: the record is a comment anyone with write
    // access can post, the status is not.
    let mut since = None;
    for c in notes(gh, n)?.iter().rev() {
        if let Some(Record::Verdict { head_sha, .. }) = parse_record(&c.body)
            && head_sha != pull.head_sha
        {
            if gh
                .statuses(&head_sha)?
                .iter()
                .any(|s| s.context == "review-verdict" && s.state == "success")
            {
                since = Some(head_sha);
            }
            break;
        }
    }
    let files = match &since {
        Some(s) => gh.compare_files(s, &pull.head_sha)?,
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
    let lessons = parse_lessons(gh.file_at("docs/lessons.toml", &pull.base_sha)?.as_deref())
        .map_err(|m| BosunError::error(m, "fix docs/lessons.toml on the base branch"))?;
    Ok(Outcome::data(json!({
        "pr": n, "title": pull.title, "head_sha": pull.head_sha, "base_sha": pull.base_sha,
        "since_sha": since, "files": files.iter().map(|f| json!({
            "filename": f.filename, "status": f.status, "additions": f.additions,
            "deletions": f.deletions, "patch": f.patch})).collect::<Vec<_>>(),
        "task": task, "ucs": ucs, "lessons": lessons_for(lessons, &names),
        "review_md": gh.file_at("REVIEW.md", &pull.base_sha)?,
        "instructions": INSTRUCTIONS,
    })))
}
