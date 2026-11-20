//! `bosun review-loop <pr> [--max N]` / `--report`: the PR author owns the review loop (spec §7).
//! Rounds come from the verdict comments of the PR; the command returns the one next step.

use std::collections::HashMap;
use std::path::Path;

use serde_json::{Value, json};

use super::ReviewLoopArgs;
use super::verdict::{require_all_chunks, trusted_config};
use crate::app::{App, kill_switch};
use crate::core::review_loop::{Loop, Seen, Step, plan};
use crate::core::verdict::{Verdict, closes_issue, comment_chunks, comment_json, comment_session};
use crate::github::{Github, IssueState, Pull};
use crate::output::{BosunError, CmdResult, Outcome};

pub fn run(dir: Option<&Path>, args: &ReviewLoopArgs) -> CmdResult {
    let app = App::load(dir)?;
    let gh = app.gh.as_ref();
    kill_switch(gh)?;
    // The limit and the bot list come from the default branch, never the checkout; `--max`
    // can only lower the limit. Past it the owner raises `[mode] review_rounds` (an R-gov PR).
    let (cfg, _) = trusted_config(gh, &format!("issues/{}", args.pr.unwrap_or_default()))?;
    let limit = cfg.mode_effective().knobs.review_rounds.value;
    let max = args.max.map_or(limit, |m| m.min(limit));
    if max == 0 {
        return Err(BosunError::usage(
            "--max must be at least 1",
            "pass a round limit of 1 or more",
        ));
    }
    let bots = cfg.approvals.verdict_bots.clone().unwrap_or_default();
    let repo = app.ctx.repo.to_string();
    if args.report {
        let mut rows = Vec::new();
        for pull in gh.list_pulls(None, IssueState::Open)? {
            let seen = seen(gh, &repo, &pull, &bots)?;
            let l = plan(&pull.head_sha, &seen, max);
            if l.step != Step::Ready {
                rows.push(row(&pull, &l));
            }
        }
        return Ok(Outcome::data(json!({"max": max, "prs": rows})));
    }
    let n = args.pr.unwrap_or_default();
    let pull = gh.get_pull(n)?.ok_or_else(|| {
        BosunError::error(format!("PR #{n} does not exist"), "check the PR number")
    })?;
    let l = plan(&pull.head_sha, &seen(gh, &repo, &pull, &bots)?, max);
    let mut data = row(&pull, &l);
    data["max"] = json!(max);
    match &l.step {
        Step::AskOwner(why) => Err(BosunError::needs_human(
            format!("PR #{n}: {why}"),
            match closes_issue(&pull.body) {
                Some(t) => format!("ask the owner with `bosun ask {t} \"<why the review loop stops>\"`; after the answer, run again with a higher --max"),
                None => "tell the orchestrator; after the owner answers, run again with a higher --max".into(),
            },
        )
        .with_data(data)),
        Step::SpawnReviewer => Ok(Outcome::data(data).with_next(
            "Agent",
            json!({"subagent_type": "general-purpose", "description": format!("review PR #{n}"),
                "prompt": reviewer_prompt(n, &pull.head_sha)}),
        )),
        Step::Fix => Ok(Outcome::data(data).with_next(
            "Agent",
            json!({"subagent_type": "general-purpose", "description": format!("fix review of PR #{n}"),
                "prompt": format!("Fix these review findings on the branch of PR #{n} (head {}), locally; push once when all gates pass. Findings: {}. Why the verdict does not approve: {}",
                    pull.head_sha, serde_json::to_string(&l.findings).unwrap_or_default(), l.reasons.join("; "))}),
        )),
        Step::Ready => {
            let cmd = match closes_issue(&pull.body) {
                Some(t) => format!("bosun pr {t} --ready"),
                None => format!("bosun pr --pr {n} --ready"),
            };
            Ok(Outcome::data(data).with_next("Bash", json!({"command": cmd})))
        }
    }
}

fn row(pull: &Pull, l: &Loop) -> Value {
    json!({"pr": pull.number, "head_sha": pull.head_sha, "step": l.step.name(),
        "rounds": l.rounds, "findings": l.findings, "reasons": l.reasons,
        "reason": match &l.step { Step::AskOwner(w) => json!(w), _ => Value::Null }})
}

fn reviewer_prompt(n: u64, head: &str) -> String {
    format!(
        "Review PR #{n} at head {head}. Read `bosun review-pack {n}` and follow its instructions. A read-only \
reviewer cannot write statuses: save the verdict JSON to a file in your scratchpad first (it survives a \
restart), then post it with `bosun verdict comment --from -` (JSON on stdin) or `--from <file>` (without CI add `--sign-key <reviewer.pem>`)."
    )
}

/// The verdict comments of the PR that `verdict apply` would accept: unedited, on this PR, by a
/// writer (or a bot). A newer comment of the same reviewer session (a bot: the same login) for
/// the same head replaces an older one. A bot verdict is never independent.
fn seen(
    gh: &dyn Github,
    repo: &str,
    pull: &Pull,
    bots: &[String],
) -> Result<Vec<Seen>, BosunError> {
    let n = pull.number;
    let mut writers: HashMap<String, bool> = HashMap::new();
    let mut out: Vec<(String, Seen)> = Vec::new();
    for c in gh.list_comments(n)? {
        let Some(text) = comment_json(&c.body) else {
            continue;
        };
        // What `verdict apply` refuses never counts, a counted approval too.
        let Some(mut verdict) = Verdict::parse(text)
            .ok()
            .filter(|v| !super::verdict::counted_approval(v, text))
        else {
            continue;
        };
        if verdict.chunks_reviewed.is_empty() {
            verdict.chunks_reviewed = comment_chunks(&c.body);
        }
        if !c.unedited() || verdict.pr != n || !c.issue_url.ends_with(&format!("/issues/{n}")) {
            continue;
        }
        let bot = c.user_type == "Bot" || bots.iter().any(|b| b.eq_ignore_ascii_case(&c.user));
        if !bot && !writers.contains_key(&c.user) {
            let ok = matches!(gh.permission(&c.user)?.as_deref(), Some("admin" | "write"));
            writers.insert(c.user.clone(), ok);
        }
        if !bot && !writers[&c.user] {
            continue;
        }
        // As `verdict apply`: an approval of the head must list every chunk of the pack.
        if !bot
            && verdict.head_sha == pull.head_sha
            && require_all_chunks(gh, repo, &verdict).is_err()
        {
            continue;
        }
        let who = if bot {
            format!("bot:{}", c.user.to_lowercase())
        } else {
            comment_session(&c.body).unwrap_or("unknown").to_string()
        };
        let key = format!("{who}@{}", verdict.head_sha);
        out.retain(|(k, _)| *k != key);
        out.push((key, Seen { bot, verdict }));
    }
    Ok(out.into_iter().map(|(_, s)| s).collect())
}
