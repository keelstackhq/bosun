//! `bosun gate-missed`: every escaped defect teaches a check (spec §9). One follow-up issue per
//! incident asks "Which gate missed this?". main-health (a new main-red) and `revert` open it
//! on their own; this command opens it by hand. The issue is deduped by an incident key.

use std::path::Path;

use serde_json::json;

use super::GateMissedArgs;
use crate::app::{App, kill_switch};
use crate::github::{Github, IssueState};
use crate::output::{BosunError, CmdResult, Outcome};

pub const LABEL: &str = "bosun:process-fix";

/// What escaped: `key` dedupes (`run:URL`, `pr:N`, `main-red:#N`), `what` names it in the
/// title, `link` points at the evidence, `range` is the commit range if known.
pub struct Incident<'a> {
    pub key: &'a str,
    pub what: &'a str,
    pub link: &'a str,
    pub range: Option<&'a str>,
}

fn marker(key: &str) -> String {
    let key = key.replace("<!--", "").replace("-->", "");
    format!("<!-- bosun:gate-missed {key} -->")
}

/// Open the task for `inc`, or return the existing one (open or closed). `(issue, created)`.
pub fn open(gh: &dyn Github, inc: &Incident) -> Result<(u64, bool), BosunError> {
    let mark = marker(inc.key);
    if let Some(i) = gh
        .list_issues(&[LABEL], IssueState::All)?
        .iter()
        .filter(|i| !i.pull_request)
        .find(|i| i.body_text().contains(&mark))
    {
        return Ok((i.number, false));
    }
    let clean = |t: &str| t.replace("<!--", "").replace("-->", "");
    let (what, link) = (clean(inc.what), clean(inc.link));
    gh.ensure_label(LABEL, "d93f0b", "a gate missed an escaped defect")?;
    let range = inc.range.unwrap_or(
        "not known: from the last green main run to the failing commit (`bosun main-triage --good REV`)",
    );
    let body = format!(
        "An escaped defect: {what}.\n\n- Evidence: {link}\n- Commit range: {range}\n\n\
         Which gate missed this? Fill in each item, then close the issue with the PR that adds the check.\n\n\
         - [ ] Root cause\n\
         - [ ] The tightest loop that could catch it (local, CI, nightly)\n\
         - [ ] The proposed gate or test\n\n{mark}",
        what = what,
        link = link,
    );
    let n = gh
        .create_issue(&format!("Which gate missed this? {what}"), &body, &[LABEL])?
        .number;
    Ok((n, true))
}

pub fn run(dir: Option<&Path>, a: &GateMissedArgs) -> CmdResult {
    super::refuse_secret("--run", a.run.as_deref().unwrap_or(""))?;
    if a.pr.is_some() == a.run.is_some() {
        return Err(BosunError::usage(
            "name the incident with exactly one of --pr or --run",
            "pass --pr N (the reverted or escaped PR) or --run URL (the failing run)",
        ));
    }
    let app = App::load(dir)?;
    let gh = app.gh.as_ref();
    kill_switch(gh)?;
    let (key, what, link) = match (a.pr, a.run.as_deref()) {
        (Some(n), None) => {
            let pull = gh.get_pull(n)?.ok_or_else(|| {
                BosunError::error(format!("PR #{n} does not exist"), "check the PR number")
            })?;
            (
                format!("pr:{n}"),
                format!("#{n} {}", pull.title),
                format!("PR #{n}"),
            )
        }
        (None, Some(url)) => (
            format!("run:{url}"),
            "a failing run".into(),
            url.to_string(),
        ),
        _ => unreachable!("checked above"),
    };
    let inc = Incident {
        key: &key,
        what: &what,
        link: &link,
        range: a.range.as_deref(),
    };
    let (issue, created) = open(gh, &inc)?;
    Ok(Outcome::data(
        json!({"issue": issue, "created": created, "key": key}),
    ))
}
