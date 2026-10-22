//! `bosun ci-log <pr>`: the failed jobs of the PR head with the tail of each log.

use std::path::Path;

use serde_json::json;

use super::CiLogArgs;
use crate::app::App;
use crate::output::{BosunError, CmdResult, Outcome};

pub fn run(dir: Option<&Path>, args: &CiLogArgs) -> CmdResult {
    let app = App::load(dir)?;
    let gh = app.gh.as_ref();
    let n = args.pr;
    let pull = gh.get_pull(n)?.ok_or_else(|| {
        BosunError::error(
            format!("PR #{n} does not exist"),
            "check the number; `gh pr list` shows the open PRs",
        )
    })?;
    let mut jobs = Vec::new();
    for job in gh.failed_jobs(&pull.head_sha)? {
        jobs.push(json!({
            "name": job.name, "conclusion": job.conclusion, "url": job.html_url,
            "log": gh.job_log(job.id, args.tail)?,
        }));
    }
    Ok(Outcome::data(
        json!({"pr": n, "head_sha": pull.head_sha, "jobs": jobs}),
    ))
}
