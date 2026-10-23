//! `bosun gate main-open --pr N` (spec section 9): while main is red, only a mainfix PR passes.
//! It posts the `main-open` commit status on the PR head. It does not check the kill switch: it
//! is a check, not work.

use std::path::Path;

use serde_json::json;

use crate::app::{App, Result, main_red};
use crate::core::meta::linked_issues;
use crate::github::{Github, Pull};
use crate::output::{BosunError, CmdResult, Outcome};

const CONTEXT: &str = "main-open";
pub const MAINFIX_LABEL: &str = "bosun:kind/mainfix";

#[derive(Debug, PartialEq, Eq)]
pub struct Verdict {
    pub pass: bool,
    pub description: String,
}

/// Pass when main is open, or when the PR is a mainfix.
pub fn judge(main_red: Option<u64>, mainfix: bool) -> Verdict {
    let (pass, description) = match (main_red, mainfix) {
        (None, _) => (true, "main is open".to_string()),
        (Some(n), true) => (true, format!("main is red (#{n}); mainfix passes")),
        (Some(n), false) => (
            false,
            format!("main is red (#{n}): only mainfix PRs merge until it closes"),
        ),
    };
    Verdict { pass, description }
}

/// A mainfix PR: the label on the PR, or the label on a linked issue that is open and is not a
/// pull request. Only labels count (they need triage rights); the task marker in a body is
/// written by its author, so it is never trusted here.
pub fn is_mainfix(gh: &dyn Github, pull: &Pull) -> Result<bool> {
    if pull.labels.iter().any(|l| l == MAINFIX_LABEL) {
        return Ok(true);
    }
    for n in linked_issues(&pull.body) {
        if let Some(issue) = gh.get_issue(n)?
            && issue.state == "open"
            && !issue.pull_request
            && issue.labels.iter().any(|l| l == MAINFIX_LABEL)
        {
            return Ok(true);
        }
    }
    Ok(false)
}

/// Judge one PR against `red` and post the `main-open` status on its head. Returns whether it
/// passes. `bosun main-health` re-judges every open PR with this when main turns red or green.
pub fn judge_pr(gh: &dyn Github, pull: &Pull, red: Option<u64>) -> Result<bool> {
    let mainfix = red.is_some() && is_mainfix(gh, pull)?;
    Ok(post(gh, pull, red, mainfix)?.pass)
}

fn post(gh: &dyn Github, pull: &Pull, red: Option<u64>, mainfix: bool) -> Result<Verdict> {
    let v = judge(red, mainfix);
    let state = if v.pass { "success" } else { "failure" };
    gh.create_status(&pull.head_sha, CONTEXT, state, &v.description, None)?;
    Ok(v)
}

pub fn run(dir: Option<&Path>, pr: Option<u64>) -> CmdResult {
    let n = match pr {
        Some(n) => n,
        None => std::env::var("BOSUN_PR")
            .ok()
            .and_then(|v| v.trim().parse().ok())
            .ok_or_else(|| {
                BosunError::usage(
                    "main-open needs a PR number",
                    "pass --pr N (or set BOSUN_PR)",
                )
            })?,
    };
    let app = App::load(dir)?;
    let gh = app.gh.as_ref();
    let pull = gh.get_pull(n)?.ok_or_else(|| {
        BosunError::error(
            format!("PR #{n} does not exist"),
            "check the number; `gh pr list` lists the open PRs",
        )
    })?;
    let red = main_red(gh)?;
    let mainfix = red.is_some() && is_mainfix(gh, &pull)?;
    let v = post(gh, &pull, red, mainfix)?;
    let data = json!({"pr": n, "head_sha": pull.head_sha, "main_red": red,
        "mainfix": mainfix, "pass": v.pass});
    if v.pass {
        return Ok(Outcome::data(data));
    }
    let red = red.unwrap_or_default();
    Err(BosunError::halted(
        v.description,
        format!("fix main with a bosun:kind/mainfix PR, or wait for #{red} to close"),
    )
    .with_data(data))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn open_main_passes_everyone() {
        let v = judge(None, false);
        assert!(v.pass);
        assert_eq!(v.description, "main is open");
    }

    #[test]
    fn red_main_passes_only_mainfix() {
        let v = judge(Some(5), true);
        assert!(v.pass);
        assert_eq!(v.description, "main is red (#5); mainfix passes");
        let v = judge(Some(5), false);
        assert!(!v.pass);
        assert!(v.description.contains("#5"));
        assert!(v.description.len() < 140);
    }
}
