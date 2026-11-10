//! `bosun verdict post|comment|apply`: record the reviewer's verdict as the `review-verdict`
//! status, directly or through a verdict comment that the verdict workflow applies (spec §7).

use crate::github::gate::GateWriter;
use std::io::Read;
use std::path::Path;

use serde_json::json;

use super::{ApplyArg, FromArg, VerdictCmd};
use crate::app::{App, kill_switch, set_state};
use crate::core::records::Record;
use crate::core::state::{State, state_of, transition_allowed};
use crate::core::verdict::{
    COMMENT_MARKER, Verdict, closes_issue, comment_body, comment_chunks, comment_json,
    comment_model, comment_session, effective, valid_model,
};
use crate::github::{Github, Pull};
use crate::output::{BosunError, CmdResult, Outcome};

pub const CONTEXT: &str = "review-verdict";

fn read_input(from: &str) -> Result<String, BosunError> {
    let mut text = String::new();
    let res = if from == "-" {
        std::io::stdin().read_to_string(&mut text).map(|_| ())
    } else {
        std::fs::read_to_string(from).map(|t| text = t)
    };
    res.map_err(|e| {
        BosunError::usage(
            format!("cannot read the verdict from {from}: {e}"),
            "pass the reviewer's JSON on stdin or with --from FILE",
        )
    })?;
    Ok(text)
}

pub fn run(dir: Option<&Path>, cmd: &VerdictCmd) -> CmdResult {
    let app = App::load(dir)?;
    let gh = app.gh.as_ref();
    kill_switch(gh)?;
    match cmd {
        VerdictCmd::Post(FromArg { from }) => {
            if crate::github::gate::checks_mode() {
                // A session's token cannot write the gate App's check run (spec §15).
                return Err(BosunError::error(
                    "`bosun verdict post` cannot post review-verdict in [checks] mode \"checks\"",
                    "post the verdict with `bosun verdict comment --from <file>`; the verdict workflow applies it",
                ));
            }
            let verdict = parse(&read_input(from)?)?;
            let pull = current_pull(gh, &verdict)?;
            require_all_chunks(gh, &app.ctx.repo.to_string(), &verdict)?;
            record(
                &app,
                &pull,
                &verdict,
                &app.ctx.session,
                env_model().as_deref(),
            )
        }
        VerdictCmd::Comment(FromArg { from }) => {
            let verdict = parse(&read_input(from)?)?;
            current_pull(gh, &verdict)?;
            require_all_chunks(gh, &app.ctx.repo.to_string(), &verdict)?;
            let c = gh.add_comment(
                verdict.pr,
                &comment_body(&verdict, &app.ctx.session, env_model().as_deref()),
            )?;
            Ok(Outcome::data(
                json!({"pr": verdict.pr, "head_sha": verdict.head_sha,
                "comment_id": c.id, "applied_by": "the verdict workflow"}),
            ))
        }
        VerdictCmd::Apply(ApplyArg { comment_id }) => apply(&app, *comment_id),
    }
}

/// The reviewer model this session declares: `BOSUN_MODEL`, else `ANTHROPIC_MODEL` (as the
/// commit trailers do).
fn env_model() -> Option<String> {
    ["BOSUN_MODEL", "ANTHROPIC_MODEL"]
        .iter()
        .find_map(|k| std::env::var(k).ok().filter(|v| valid_model(v).is_some()))
}

fn parse(text: &str) -> Result<Verdict, BosunError> {
    Verdict::parse(text).map_err(|m| {
        BosunError::usage(
            format!("invalid verdict: {m}"),
            "return the JSON shape in spec §7 (see `bosun review-pack` instructions)",
        )
    })
}

/// The verdict's PR; exit 3 when the verdict is for another head than the current one.
pub(crate) fn current_pull(gh: &dyn Github, verdict: &Verdict) -> Result<Pull, BosunError> {
    let n = verdict.pr;
    let pull = gh.get_pull(n)?.ok_or_else(|| {
        BosunError::error(
            format!("PR #{n} does not exist"),
            "check the number in the verdict",
        )
    })?;
    if pull.head_sha != verdict.head_sha {
        return Err(BosunError::conflict(
            format!(
                "stale verdict for {}; PR #{n} is now at {}",
                verdict.head_sha, pull.head_sha
            ),
            format!("review the new head: run `bosun review-pack {n}` again"),
        ));
    }
    Ok(pull)
}

/// Check a verdict comment (unedited, by a member, on its own PR, for the current head) and
/// record its verdict.
fn apply(app: &App, id: u64) -> CmdResult {
    let gh = app.gh.as_ref();
    let refuse = |why: String| {
        BosunError::error(
            format!("verdict comment {id} is refused: {why}"),
            "post the verdict again with `bosun verdict comment --from -`, unedited, on the PR",
        )
    };
    let c = gh
        .get_comment(id)?
        .ok_or_else(|| refuse("it does not exist".into()))?;
    let text = comment_json(&c.body).ok_or_else(|| refuse(format!("no {COMMENT_MARKER}")))?;
    if !c.unedited() {
        return Err(refuse("it was edited".into()));
    }
    // author_association is no proof: a workflow token sees an org member as CONTRIBUTOR.
    let permission = gh.permission(&c.user)?.unwrap_or_else(|| "none".into());
    if !matches!(permission.as_str(), "admin" | "write") {
        return Err(refuse(format!(
            "{} has {permission} access, not write",
            c.user
        )));
    }
    let mut verdict = Verdict::parse(text).map_err(|m| refuse(format!("invalid verdict: {m}")))?;
    if verdict.chunks_reviewed.is_empty() {
        verdict.chunks_reviewed = comment_chunks(&c.body);
    }
    if !c.issue_url.ends_with(&format!("/issues/{}", verdict.pr)) {
        return Err(refuse(format!("it is not on PR #{}", verdict.pr)));
    }
    let pull = current_pull(gh, &verdict)?;
    // Comments apply in order: a newer verdict comment of the same reviewer session for the
    // same head replaces this one. Another session's is a second verdict (two-verdict
    // approval), not a replacement; a comment without a session counts as "unknown".
    let session = comment_session(&c.body).unwrap_or("unknown");
    let newer = gh.list_comments(verdict.pr)?.into_iter().any(|o| {
        o.id > c.id
            && comment_session(&o.body).unwrap_or("unknown") == session
            && comment_json(&o.body)
                .and_then(|t| Verdict::parse(t).ok())
                .is_some_and(|v| v.head_sha == verdict.head_sha)
    });
    if newer {
        return Err(refuse(format!(
            "a newer verdict comment for {} replaces it",
            verdict.head_sha
        )));
    }
    require_all_chunks(gh, &app.ctx.repo.to_string(), &verdict)?;
    record(app, &pull, &verdict, session, comment_model(&c.body))
}

/// A review of a PR with several chunks must list every chunk (1-based, as in the pack).
/// Only an approval needs it: a request-changes verdict never turns green, and may stop at the
/// first blocker.
fn require_all_chunks(gh: &dyn Github, repo: &str, verdict: &Verdict) -> Result<(), BosunError> {
    if verdict.verdict != "approve" {
        return Ok(());
    }
    let pack = super::review_pack::build(gh, repo, verdict.pr, false)?;
    let count = pack["chunks"].as_array().map_or(0, Vec::len);
    if count <= 1 {
        return Ok(());
    }
    let missing: Vec<usize> = (1..=count)
        .filter(|k| !verdict.chunks_reviewed.iter().any(|c| *c as usize == *k))
        .collect();
    if missing.is_empty() {
        return Ok(());
    }
    let mut listed = verdict.chunks_reviewed.clone();
    listed.sort_unstable();
    Err(BosunError::error(
        format!("verdict lists chunks {listed:?} of {count}; missing {missing:?}"),
        "review every chunk of `bosun review-pack <pr>` and list each number in `chunks_reviewed`",
    ))
}

/// Post the review-verdict status and the verdict record, and move the linked task.
pub(crate) fn record(
    app: &App,
    pull: &Pull,
    verdict: &Verdict,
    session: &str,
    model: Option<&str>,
) -> CmdResult {
    let gh = app.gh.as_ref();
    let n = verdict.pr;
    let mut eff = effective(verdict);
    if verdict.escalate_risk {
        // An escalation is for the owner: never a green verdict.
        eff.approve = false;
        eff.reasons.push("escalated to the owner".into());
    }
    let state = if eff.approve { "success" } else { "failure" };
    let description = if verdict.escalate_risk {
        format!("escalated to the owner; {}", verdict.summary())
    } else {
        verdict.summary()
    };
    let w = GateWriter::for_ctx(gh, &app.ctx);
    w.post(&verdict.head_sha, CONTEXT, state, &description)?;
    let record = Record::Verdict {
        session: session.to_string(),
        head_sha: verdict.head_sha.clone(),
        approved: eff.approve,
        confidence: Some(verdict.confidence),
        major: Some(verdict.count("major") as u32),
        model: model.map(str::to_string),
    };
    let human = format!("Review verdict: {}.", verdict.summary());
    w.add_record(n, &format!("{human}\n\n{}", record.to_line()))?;

    let target = if verdict.escalate_risk {
        Some(State::NeedsHuman)
    } else if !eff.approve {
        Some(State::Fixing)
    } else {
        None
    };
    let mut task_state = None;
    if let (Some(to), Some(issue_no)) = (target, closes_issue(&pull.body))
        && let Some(issue) = gh.get_issue(issue_no)?
        && state_of(&issue.labels).is_none_or(|from| transition_allowed(from, to))
    {
        set_state(gh, issue_no, &issue, to)?;
        task_state = Some(to.name());
    }
    let mut data = json!({
        "pr": n, "head_sha": verdict.head_sha, "approved": eff.approve,
        "reasons": eff.reasons, "status": state, "task_state": task_state,
    });
    if w.checks_skipped() {
        data["checks_skipped"] = json!(true);
    }
    Ok(Outcome::data(data))
}
