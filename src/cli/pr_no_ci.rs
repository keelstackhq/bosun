//! `bosun pr <task> --ready` with `[ci] provider = "none"` (spec §4): bosun runs here what the
//! CI workflows would: gate stamps, risk-check, a clean verdict from another session, the owner's
//! chat approval when gated, and main-open. Then it posts one summary per head and queues the
//! task for `bosun merge`. No status, no auto-merge.

use serde_json::{Value, json};

use crate::app::{App, Result, main_red, set_state};
use crate::cli::risk_check::{autonomous_clean, evaluate_local, review_models, verdict_inputs};
use crate::config::{CONFIG_FILE, CiProvider, Config};
use crate::core::approve::{parse_ready_summary, ready_summary};
use crate::core::meta::linked_issues_in;
use crate::core::records::Record;
use crate::core::state::State;
use crate::core::verdict::{COMMENT_MARKER, comment_clean};
use crate::gates::main_open::judge_local;
use crate::github::{Github, Issue, Pull, is_ccr_refusal};
use crate::output::{BosunError, CmdResult, Exit, Outcome};

/// The CI provider in the config at `git_ref` (a PR never picks its own). No file, or one that
/// does not parse, is `github`: the CI path then refuses as it always did.
pub fn provider_at(gh: &dyn Github, git_ref: &str) -> Result<CiProvider> {
    Ok(gh
        .file_at(CONFIG_FILE, git_ref)?
        .and_then(|t| Config::parse(&t, CONFIG_FILE).ok())
        .map(|c| c.ci_provider())
        .unwrap_or_default())
}

/// The sessions with a clean verdict on the head, other than the author (a lease holder of a
/// linked task) and this session. Verdict records count; with no CI at the base, so do the
/// reviewers' own verdict comments by a writer (nothing applies them).
pub fn clean_by_other(app: &App, pull: &Pull) -> Result<Vec<String>> {
    let gh = app.gh.as_ref();
    let linked = linked_issues_in(&pull.body, &app.ctx.repo.to_string());
    let (comments, mut excluded) = verdict_inputs(gh, pull.number, &linked)?;
    excluded.push(app.ctx.session.clone());
    let models = review_models(gh, &pull.base_sha)?;
    let head = &pull.head_sha;
    let found = autonomous_clean(&comments, head, &excluded, models.as_deref());
    if !found.is_empty() || provider_at(gh, &pull.base_sha)? != CiProvider::None {
        return Ok(found);
    }
    let mut kept = Vec::new();
    for c in comments.iter().filter(|c| c.body.contains(COMMENT_MARKER)) {
        let p = gh.permission(&c.user)?.unwrap_or_default();
        if matches!(p.as_str(), "admin" | "write") {
            kept.push((c.body.as_str(), c.unedited()));
        }
    }
    // An approval must list every chunk of the review pack, as `verdict apply` requires.
    let pack = super::review_pack::build(gh, &app.ctx.repo.to_string(), pull.number, false)?;
    let chunks = pack["chunks"].as_array().map_or(0, Vec::len);
    let models = models.as_deref();
    Ok(comment_clean(
        kept,
        pull.number,
        head,
        &excluded,
        models,
        chunks,
    ))
}

/// `--ready` without CI for task `n` and its PR (the task is in review).
pub fn ready(app: &App, n: u64, issue: &Issue, mut pull: Pull) -> CmdResult {
    let gh = app.gh.as_ref();
    let (pr, head) = (pull.number, pull.head_sha.clone());
    let rerun = format!("bosun pr {n} --ready");
    let gates = super::pr::check_local_gates(app, n, &pull).map_err(|mut e| {
        // No CI runs them later: a missing stamp is for the session to fix before it hands over.
        if e.data
            .as_ref()
            .is_some_and(|d| d.get("missing_gates").is_some())
        {
            e.exit = Exit::NeedsHuman;
        }
        e
    })?;
    let mut data = json!({"pr": pr, "head_sha": head, "ci_provider": "none", "gates": gates});
    let with = |e: BosunError, data: &Value| {
        let mut d = data.clone();
        if let (Some(extra), Some(obj)) = (e.data.clone(), d.as_object_mut()) {
            obj.extend(extra.as_object().cloned().unwrap_or_default());
        }
        e.with_data(d)
    };
    // Risk first: its refusal for a missing owner approval waits until the rest is known.
    let risk = evaluate_local(app, &pull);
    let risk_data = match &risk {
        Ok(o) => o.data.clone(),
        Err(e) if e.exit == Exit::NeedsHuman => e.data.clone().unwrap_or_default(),
        Err(e) => return Err(with(e.clone(), &data)),
    };
    let ids: Vec<&str> = (risk_data["matches"].as_array().into_iter().flatten())
        .filter_map(|m| m["rule"].as_str())
        .collect();
    let risk_line = if risk_data["level"] == "high" {
        format!("high: {}", ids.join(", "))
    } else {
        "normal".to_string()
    };
    data["risk"] = json!(risk_line);
    let verdicts = clean_by_other(app, &pull)?;
    if verdicts.is_empty() {
        return Err(with(
            BosunError::needs_human(
                format!("PR #{pr} has no clean verdict on head {head} from another session"),
                format!(
                    "a reviewer session runs `bosun review-pack {pr}` and posts `bosun verdict comment --from -` (approve, no blocker or major), then rerun `{rerun}`"
                ),
            ),
            &data,
        ));
    }
    data["verdicts"] = json!(verdicts);
    let main = judge_local(gh, &pull, main_red(gh)?)?;
    if !main.pass {
        return Err(with(
            BosunError::halted(
                main.description,
                format!(
                    "fix main with a bosun:kind/mainfix PR, or wait for main-red to close, then rerun `{rerun}`"
                ),
            ),
            &data,
        ));
    }
    let approval = risk
        .as_ref()
        .ok()
        .and_then(|o| o.data["approved_by"].as_str());
    data["approval"] = json!(approval);
    let body = ready_summary(&head, &gates, &risk_line, &verdicts.join(", "), approval);
    let comments = gh.list_comments(pr)?;
    // starts_with: the cloud agent proxy appends a footer to every comment.
    let posted =
        (comments.iter()).any(|c| c.body.starts_with(&body) && parse_ready_summary(&c.body, &head));
    if !posted {
        gh.add_comment(pr, &body)?;
    }
    data["summary_posted"] = json!(!posted);
    if let Err(e) = risk {
        data["next_command"] = json!("bosun approve --list");
        return Err(with(
            BosunError::needs_human(
                e.message,
                format!(
                    "the orchestrator runs `bosun approve --list`, asks the owner and records the answer (`bosun approve {pr}@{head} --chat --question <id>`); then rerun `{rerun}`"
                ),
            ),
            &data,
        ));
    }
    if pull.draft {
        match gh.set_draft(&pull.node_id, false) {
            Ok(()) => pull.draft = false,
            Err(e) if is_ccr_refusal(&e) => {
                data["ready"] = json!("pending");
                data["note"] = json!(format!(
                    "this session cannot call GraphQL: mark PR #{pr} ready with the tool call in `next`, then rerun `{rerun}`"
                ));
                let mut out = Outcome::data(data);
                out.next.push(super::pr::update_pull_call(app, pr, false));
                return Ok(out);
            }
            Err(e) => return Err(with(e, &data)),
        }
    }
    let record = Record::TaskPr {
        session: app.ctx.session.clone(),
        pr,
    };
    let note = format!(
        "PR #{pr} ready for `bosun merge` (no CI)\n\n{}",
        record.to_line()
    );
    gh.add_comment(n, &note)?;
    let fresh = gh.get_issue(n)?.unwrap_or_else(|| issue.clone());
    set_state(gh, n, &fresh, State::Queued)?;
    data["ready"] = json!(true);
    data["auto_merge"] = json!(false);
    data["task_state"] = json!(State::Queued.name());
    data["merge_next"] = json!(format!("bosun merge {pr}"));
    Ok(Outcome::data(data))
}
