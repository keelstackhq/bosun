//! `bosun pr <task> --ready` with `[ci] provider = "none"` (spec §4): runs here what CI would, posts
//! one summary per head and queues the task for `bosun merge`. No status, no auto-merge.

use serde_json::{Value, json};

use crate::app::{App, Result, is_writer, main_red, set_state};
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

/// The CI provider of the repo: its config on the default branch, never the PR's base or head
/// (a PR cannot pick its own provider); `github` if unreadable.
pub fn provider_default(gh: &dyn Github) -> Result<CiProvider> {
    let text = gh.file_at(CONFIG_FILE, &crate::app::default_ref(gh)?)?;
    Ok(text
        .and_then(|t| Config::parse(&t, CONFIG_FILE).ok())
        .map(|c| c.ci_provider())
        .unwrap_or_default())
}

/// Sessions with a clean verdict on the head, not the author or this one; no CI: writers' comments count.
pub fn clean_by_other(app: &App, pull: &Pull) -> Result<Vec<String>> {
    let gh = app.gh.as_ref();
    // The author is the lease holder of a linked task (as in D-017); with none, nothing counts.
    let linked = linked_issues_in(&pull.body, &app.ctx.repo.to_string());
    let (comments, mut excluded) = verdict_inputs(gh, pull.number, &linked)?;
    if excluded.is_empty() {
        return Ok(Vec::new());
    }
    excluded.push(app.ctx.session.clone());
    let models = review_models(gh, &pull.base_sha)?;
    let head = &pull.head_sha;
    let found = autonomous_clean(&comments, head, &excluded, models.as_deref());
    if !found.is_empty() || provider_default(gh)? != CiProvider::None {
        return Ok(found);
    }
    let mut kept = Vec::new();
    for c in comments.iter().filter(|c| c.body.contains(COMMENT_MARKER)) {
        if is_writer(gh, &c.user)? {
            kept.push((c.body.as_str(), c.unedited()));
        }
    }
    // An approval must list every chunk of the review pack, as `verdict apply` requires.
    let pack = super::review_pack::build(gh, &app.ctx.repo.to_string(), pull.number, false)?;
    let chunks = pack["chunks"].as_array().map_or(0, Vec::len);
    Ok(comment_clean(
        kept,
        pull.number,
        head,
        &excluded,
        models.as_deref(),
        chunks,
    ))
}

/// `--ready` without CI for task `n` and its PR (the task is in review).
pub fn ready(app: &App, n: u64, issue: &Issue, plain: bool, mut pull: Pull) -> CmdResult {
    let gh = app.gh.as_ref();
    let (pr, head) = (pull.number, pull.head_sha.clone());
    let rerun = format!("bosun pr {n} --ready");
    let gates = super::pr::check_local_gates(app, &rerun, &pull).map_err(|mut e| {
        if e.data
            .as_ref()
            .is_some_and(|d| d.get("missing_gates").is_some())
        {
            e.exit = Exit::NeedsHuman;
        }
        e
    })?;
    let mut data = json!({"pr": pr, "head_sha": head, "created": false, "auto_merge": false, "draft": pull.draft, "stamps": gates});
    let with = |e: BosunError, data: &Value| {
        let mut d = data.clone();
        let extra = e.data.iter().filter_map(|x| x.as_object()).flatten();
        d.as_object_mut()
            .unwrap()
            .extend(extra.map(|(k, v)| (k.clone(), v.clone())));
        e.with_data(d)
    };
    let risk = evaluate_local(app, &pull);
    let risk_data = match &risk {
        Ok(o) => o.data.clone(),
        Err(e) if e.exit == Exit::NeedsHuman => e.data.clone().unwrap_or_default(),
        Err(e) => return Err(with(e.clone(), &data)),
    };
    let ids =
        (risk_data["matches"].as_array().into_iter().flatten()).filter_map(|m| m["rule"].as_str());
    let risk_line = match risk_data["level"] == "high" {
        true => format!("high: {}", ids.collect::<Vec<_>>().join(", ")),
        false => "normal".into(),
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
    let mut posted = false;
    for c in comments.iter().filter(|c| c.body.starts_with(&body)) {
        posted = posted || (parse_ready_summary(&c.body, &head) && is_writer(gh, &c.user)?);
    }
    if !posted {
        gh.add_comment(pr, &body)?;
    }
    if let Err(e) = risk {
        data["next_command"] = json!("bosun approve --list");
        return Err(with(
            BosunError::needs_human(
                e.message,
                format!(
                    "the orchestrator runs `bosun approve --list`, asks the owner and records the answer (`bosun approve {pr}@{head} --chat --question <id>`), or: {}; then rerun `{rerun}`",
                    e.fix
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
    gh.add_comment(
        n,
        &format!(
            "PR #{pr} ready for `bosun merge` (no CI)\n\n{}",
            record.to_line()
        ),
    )?;
    if !plain {
        let fresh = gh.get_issue(n)?.unwrap_or_else(|| issue.clone());
        set_state(gh, n, &fresh, State::Queued)?;
    }
    data["ready"] = json!(true);
    data["draft"] = json!(false);
    data["task_state"] = json!((!plain).then_some(State::Queued.name()));
    data["merge_next"] = json!(format!("bosun merge {pr}"));
    Ok(Outcome::data(data))
}
