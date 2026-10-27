//! `bosun prereqs <plan>... [--no-setup]`: the human steps a plan needs up front, checked through
//! the API and the ticks of the plan's checklist issue (design F2-F4).

use std::path::Path;

use serde_json::{Value, json};

use super::PrereqsArgs;
use super::plan::{plan_errors, plan_key};
use crate::app::{App, Result, default_branch};
use crate::core::approve::{APPROVERS_PATH, Role, parse_approvers};
use crate::core::plan;
use crate::core::prereqs::{self as pq, CheckItem, Verify};
use crate::core::risk::Rules;
use crate::github::http::enc;
use crate::github::{Github, IssueState};
use crate::output::{BosunError, CmdResult, Outcome};

const LABEL: &str = "bosun:prereqs";
const RULES_PATH: &str = ".bosun/risk.toml";
const BOSUN_CODE_MARKER: &str = "src/cli/risk_check.rs";

pub fn run(dir: Option<&Path>, args: &PrereqsArgs) -> CmdResult {
    let app = App::load(dir)?;
    let gh = app.gh.as_ref();
    let root = app.ctx.dir.as_path();
    let read = |rel: &str| std::fs::read_to_string(root.join(rel)).ok();
    let rules = match read(RULES_PATH) {
        Some(t) => Rules::parse(&t).map_err(|e| {
            BosunError::error(
                format!("{RULES_PATH}: {e}"),
                format!("fix {RULES_PATH}, or remove it to use the built-in rules"),
            )
        })?,
        None => Rules::defaults(),
    };
    let bosun_code = root.join(BOSUN_CODE_MARKER).is_file();
    let mut warnings = Vec::new();
    let keys = read(APPROVERS_PATH).map(|t| parse_approvers(&t));
    if let Some(Err(e)) = &keys {
        warnings.push(format!("{APPROVERS_PATH}: {e}"));
    }
    let has_delegate = keys
        .and_then(|k| k.ok())
        .is_some_and(|k| k.iter().any(|k| k.role == Role::Delegate));
    let setup = if args.no_setup {
        vec![]
    } else {
        doctor_items(dir, &mut warnings)
    };
    let base = std::fs::canonicalize(dir.unwrap_or_else(|| Path::new(".")))
        .unwrap_or_else(|_| dir.unwrap_or_else(|| Path::new(".")).to_path_buf());
    let mut roots = vec![root.to_path_buf()];
    roots.extend(std::fs::canonicalize(root).ok());
    let existing = gh.list_issues(&[LABEL], IssueState::All)?;
    let mut branch = None;
    let (mut plans, mut open) = (Vec::new(), 0);
    for (k, file) in args.files.iter().enumerate() {
        let path = dir.map_or_else(|| file.into(), |d| d.join(file));
        let text = std::fs::read_to_string(&path).map_err(|e| {
            BosunError::error(
                format!("cannot read {file}: {e}"),
                "pass the path of a plan file, for example plans/bosun.md",
            )
        })?;
        let plan = plan::parse(&text).map_err(|e| plan_errors(file, &e))?;
        let prereqs = pq::parse(&text, &plan).map_err(|e| plan_errors(file, &e))?;
        let plan_path = plan_key(file, &base, &roots);
        let marker = pq::issue_marker(&plan_path);
        let issue = existing.iter().find(|i| i.body_text().starts_with(&marker));
        let ticks = issue.map_or_else(Vec::new, |i| pq::read_issue(i.body_text()));
        let mut items = Vec::new();
        for p in &prereqs {
            let mut it = CheckItem::from(p);
            let tick = ticks.iter().find(|t| t.id == it.id);
            it.done = match verify(gh, &p.verify, &mut branch)? {
                Some(found) => found,
                None => {
                    it.api = false;
                    tick.is_some_and(|t| t.ticked)
                }
            };
            items.push(it);
        }
        if k == 0 {
            items.extend(setup.iter().cloned()); // setup steps belong to the repo: listed once
        }
        let approvals = pq::approvals(&plan, &rules, bosun_code);
        if !has_delegate && approvals.iter().any(|a| a.needs == "delegate") {
            items.push(delegate_key());
        }
        pq::sort(&mut items);
        let n_open = items.iter().filter(|i| !i.done).count();
        open += n_open;
        let posted = issue.map(|i| (i.number, false));
        let url = posted.map(|(n, _)| format!("https://github.com/{}/issues/{n}", app.ctx.repo));
        let message = pq::message(&plan.title, &items, &approvals, url.as_deref());
        plans.push(json!({"file": file, "plan_path": plan_path, "title": plan.title,
            "items": items, "approvals": approvals, "open": n_open, "message": message,
            "issue": posted.map(|(number, created)| json!({"number": number, "url": url, "created": created}))}));
    }
    let message: Vec<&str> = plans.iter().filter_map(|p| p["message"].as_str()).collect();
    Ok(Outcome::data(json!({"message": message.join("\n\n"),
        "plans": plans, "open": open, "warnings": warnings})))
}

/// `Some(found)` when the API answered, `None` when it did not (owner, or a 403).
fn verify(gh: &dyn Github, v: &Verify, branch: &mut Option<String>) -> Result<Option<bool>> {
    let scoped = |kind: &str, env: &Option<String>, name: &str| match env {
        Some(e) => format!("/environments/{}/{kind}/{}", enc(e), enc(name)),
        None => format!("/actions/{kind}/{}", enc(name)),
    };
    match v {
        Verify::Owner => Ok(None),
        Verify::Secret { env, name } => gh.exists(&scoped("secrets", env, name)),
        Verify::Variable { env, name } => gh.exists(&scoped("variables", env, name)),
        Verify::Environment { name } => gh.exists(&format!("/environments/{}", enc(name))),
        Verify::File { path } => {
            if branch.is_none() {
                *branch = Some(default_branch(gh)?);
            }
            let segments: Vec<String> = path.split('/').map(enc).collect();
            let r = enc(branch.as_deref().unwrap_or("main"));
            gh.exists(&format!("/contents/{}?ref={r}", segments.join("/")))
        }
    }
}

/// Each missing step of `bosun doctor` as an item.
fn doctor_items(dir: Option<&Path>, warnings: &mut Vec<String>) -> Vec<CheckItem> {
    let data = match super::doctor::run(dir, false) {
        Ok(o) => o.data,
        Err(e) => match e.data {
            Some(d) => d,
            None => {
                warnings.push(format!("bosun doctor failed: {}", e.message));
                return vec![];
            }
        },
    };
    let text = |i: &Value, k: &str| i[k].as_str().unwrap_or("").to_string();
    let items = data["items"].as_array().cloned().unwrap_or_default();
    let missing = items.iter().filter(|i| i["status"] == "missing");
    missing
        .map(|i| {
            auto(
                format!("D{}", i["step"]),
                &text(i, "name"),
                &text(i, "fix"),
                "doctor",
            )
        })
        .collect()
}

/// An item bosun adds itself: the owner does it on a computer.
fn auto(id: String, title: &str, steps: &str, verify: &str) -> CheckItem {
    let (who, on) = ("owner".to_string(), "computer".to_string());
    let (title, steps, verify) = (title.into(), steps.into(), verify.into());
    CheckItem {
        id,
        title,
        who,
        on,
        steps,
        verify,
        api: true,
        done: false,
        blocks: vec![],
    }
}

fn delegate_key() -> CheckItem {
    let steps = "an orchestrator session makes an ed25519 key and opens a PR adding it to .bosun/approvers.toml with role = \"delegate\"; approve that PR on GitHub";
    auto(
        "K1".into(),
        "Enrol a delegate key",
        steps,
        &format!("file {APPROVERS_PATH}"),
    )
}
