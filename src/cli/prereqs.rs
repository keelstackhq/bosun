//! `bosun prereqs <plan>... [--post] [--no-setup]`: the human steps a plan needs up front, checked
//! through the API, and with `--post` kept in one checklist issue per plan (design F2-F4).

use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use super::PrereqsArgs;
use super::plan::{plan_errors, plan_key};
use crate::app::{App, Result, default_branch, kill_switch};
use crate::core::approve::{APPROVERS_PATH, Role, parse_approvers};
use crate::core::plan;
use crate::core::prereqs::{self as pq, CheckItem, HoldBox, Verify};
use crate::core::risk::Rules;
use crate::github::http::enc;
use crate::github::{Github, Issue, IssuePatch, IssueState, PatchState};
use crate::output::{BosunError, CmdResult, Outcome};

const LABEL: &str = "bosun:prereqs";
const RULES_PATH: &str = ".bosun/risk.toml";
const BOSUN_CODE_MARKER: &str = "src/cli/risk_check.rs";

/// What `Env::checklist` found for one plan.
pub(crate) struct PlanChecklist {
    pub plan_path: String,
    pub title: String,
    pub items: Vec<CheckItem>,
    pub approvals: Vec<pq::ApprovalNeed>,
    pub open: usize,
    /// The checklist issue: its number and whether this call created it.
    pub issue: Option<(u64, bool)>,
    pub url: Option<String>,
    pub message: String,
}

/// A plan checked but maybe not yet written.
pub(crate) struct Prepared {
    pub list: PlanChecklist,
    existing: Option<Issue>,
    body: String,
    any: bool,
}

/// What every plan of one run shares: rules, approvers, the existing checklist issues.
pub(crate) struct Env {
    rules: Rules,
    bosun_code: bool,
    has_delegate: bool,
    base: PathBuf,
    roots: Vec<PathBuf>,
    existing: Vec<Issue>,
    branch: Option<String>,
}

impl Env {
    pub(crate) fn load(app: &App, dir: Option<&Path>, warnings: &mut Vec<String>) -> Result<Env> {
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
        let keys = read(APPROVERS_PATH).map(|t| parse_approvers(&t));
        if let Some(Err(e)) = &keys {
            warnings.push(format!("{APPROVERS_PATH}: {e}"));
        }
        let has_delegate = keys
            .and_then(|k| k.ok())
            .is_some_and(|k| k.iter().any(|k| k.role == Role::Delegate));
        let base = std::fs::canonicalize(dir.unwrap_or_else(|| Path::new(".")))
            .unwrap_or_else(|_| dir.unwrap_or_else(|| Path::new(".")).to_path_buf());
        let mut roots = vec![root.to_path_buf()];
        roots.extend(std::fs::canonicalize(root).ok());
        let existing = app.gh.list_issues(&[LABEL], IssueState::All)?;
        Ok(Env {
            rules,
            bosun_code,
            has_delegate,
            base,
            roots,
            existing,
            branch: None,
        })
    }

    /// Exit 5 when more than one checklist issue (any state) belongs to the plan.
    pub(crate) fn one_issue(&self, plan_path: &str) -> Result<()> {
        let marker = pq::issue_marker(plan_path);
        let n = (self.existing.iter()).filter(|i| i.body_text().starts_with(&marker));
        if n.count() > 1 {
            return Err(BosunError::needs_human(
                format!("two checklist issues for {plan_path}"),
                "close one of them; the owner checklist is one issue per plan",
            ));
        }
        Ok(())
    }

    /// The checklist issue of the plan, if one exists (any state).
    pub(crate) fn issue_of(&self, plan_path: &str) -> Option<&Issue> {
        let marker = pq::issue_marker(plan_path);
        self.existing
            .iter()
            .find(|i| i.body_text().starts_with(&marker))
    }

    /// The items, approvals and (with `post_it`) the issue of one plan. `setup` items are added.
    pub(crate) fn checklist(
        &mut self,
        app: &App,
        file: &str,
        text: &str,
        post_it: bool,
        setup: &[CheckItem],
        warnings: &mut Vec<String>,
    ) -> Result<PlanChecklist> {
        let mut p = self.prepare(app, file, text, post_it, setup, warnings)?;
        if post_it {
            self.write(app, &mut p)?;
        }
        Ok(p.list)
    }

    /// Everything `checklist` does but the write. With `post_it` it also runs the refusal checks,
    /// so a caller with several plans can check them all before writing any.
    pub(crate) fn prepare(
        &mut self,
        app: &App,
        file: &str,
        text: &str,
        post_it: bool,
        setup: &[CheckItem],
        warnings: &mut Vec<String>,
    ) -> Result<Prepared> {
        let gh = app.gh.as_ref();
        let plan = plan::parse(text).map_err(|e| plan_errors(file, &e))?;
        let prereqs = pq::parse(text, &plan).map_err(|e| plan_errors(file, &e))?;
        let plan_path = plan_key(file, &self.base, &self.roots);
        check_plan_path(&plan_path)?;
        let marker = pq::issue_marker(&plan_path);
        let mut mine = self
            .existing
            .iter()
            .filter(|i| i.body_text().starts_with(&marker));
        let issue = mine.next();
        if post_it && mine.next().is_some() {
            return Err(BosunError::needs_human(
                format!("two checklist issues for {plan_path}"),
                "close one of them; the owner checklist is one issue per plan",
            ));
        }
        if self.branch.is_none() {
            self.branch = Some(default_branch(gh)?);
        }
        let main = self.branch.as_deref().unwrap_or("main");
        if gh.file_at(&plan_path, main)?.as_deref() != Some(text) {
            let msg = format!("{plan_path} on {main} differs from the local copy");
            if post_it {
                return Err(BosunError::conflict(
                    msg,
                    "merge the plan change first; --post writes the checklist of the plan on the default branch",
                ));
            }
            warnings.push(format!(
                "{msg}, or is not there: the checklist is built from the local file"
            ));
        }
        let ticks = issue.map_or_else(Vec::new, |i| pq::read_issue(i.body_text()));
        let mut items = Vec::new();
        for p in &prereqs {
            let mut it = CheckItem::from(p);
            // An API item is done only when the API finds it; an owner item, or an API item the
            // token cannot read (403), when its one box with a matching sum is ticked.
            (it.state, it.done) = match verify(gh, &p.verify, &mut self.branch)? {
                Some(true) => ("api ok".into(), true),
                Some(false) => ("api".into(), false),
                None => {
                    let state = if p.verify == Verify::Owner {
                        "owner"
                    } else {
                        it.verify += " (bosun cannot read it: your tick decides)";
                        "api 403"
                    };
                    (state.into(), pq::ticked(&ticks, &it.id, &it.sum))
                }
            };
            items.push(it);
        }
        items.extend(setup.iter().cloned());
        let approvals = pq::approvals(&plan, &self.rules, self.bosun_code);
        if !self.has_delegate && approvals.iter().any(|a| a.needs == "delegate") {
            items.push(delegate_key());
        }
        pq::sort(&mut items);
        // The boxes of `bosun hold` ride along on every rewrite (each re-rendered from its
        // marker, with its tick), unless a plan item `replaces` the hold.
        let replaced = |id: &str| prereqs.iter().any(|p| p.replaces.iter().any(|r| r == id));
        let (_, found) = issue.map_or_else(Default::default, |i| pq::split_found(i.body_text()));
        let found: Vec<HoldBox> = found.into_iter().filter(|b| !replaced(&b.id)).collect();
        let open =
            items.iter().filter(|i| !i.done).count() + found.iter().filter(|b| !b.ticked).count();
        let body = pq::render_issue(&plan.title, &plan_path, &items, &approvals);
        let body = pq::with_found(&body, &found);
        let any = !items.is_empty() || !approvals.is_empty() || !found.is_empty();
        items.extend(found.iter().map(HoldBox::item));
        let issue = issue.cloned();
        let existing = issue.as_ref().map(|i| (i.number, false));
        let url = existing.map(|(n, _)| format!("https://github.com/{}/issues/{n}", app.ctx.repo));
        let message = pq::message(&plan.title, &items, &approvals, url.as_deref());
        let list = PlanChecklist {
            plan_path,
            title: plan.title,
            items,
            approvals,
            open,
            issue: existing,
            url,
            message,
        };
        Ok(Prepared {
            list,
            existing: issue,
            body,
            any,
        })
    }

    /// Write the checklist issue of a prepared plan and fill in its number, url and message.
    pub(crate) fn write(&self, app: &App, p: &mut Prepared) -> Result<()> {
        let l = &mut p.list;
        l.issue = post(
            app.gh.as_ref(),
            p.existing.as_ref(),
            &l.title,
            &p.body,
            l.open,
            p.any,
        )?;
        l.url = l
            .issue
            .map(|(n, _)| format!("https://github.com/{}/issues/{n}", app.ctx.repo));
        l.message = pq::message(&l.title, &l.items, &l.approvals, l.url.as_deref());
        Ok(())
    }
}

pub fn run(dir: Option<&Path>, args: &PrereqsArgs) -> CmdResult {
    let app = App::load(dir)?;
    if args.post {
        kill_switch(app.gh.as_ref())?;
    }
    let mut warnings = Vec::new();
    let mut env = Env::load(&app, dir, &mut warnings)?;
    let setup = if args.no_setup {
        vec![]
    } else {
        doctor_items(dir, &mut warnings)
    };
    let mut prepared = Vec::new();
    for (k, file) in args.files.iter().enumerate() {
        let path = dir.map_or_else(|| file.into(), |d| d.join(file));
        let text = std::fs::read_to_string(&path).map_err(|e| {
            BosunError::error(
                format!("cannot read {file}: {e}"),
                "pass the path of a plan file, for example plans/bosun.md",
            )
        })?;
        // Setup steps belong to the repo: listed once.
        let first = if k == 0 { &setup[..] } else { &[] };
        let p = env.prepare(&app, file, &text, args.post, first, &mut warnings)?;
        prepared.push((file, p));
    }
    // Every plan passed its refusal checks: only now write.
    let (mut plans, mut open) = (Vec::new(), 0);
    for (file, mut p) in prepared {
        if args.post {
            env.write(&app, &mut p)?;
        }
        let c = p.list;
        open += c.open;
        let issue = c
            .issue
            .map(|(number, created)| json!({"number": number, "url": c.url, "created": created}));
        plans.push(
            json!({"file": file, "plan_path": c.plan_path, "title": c.title,
            "items": c.items, "approvals": c.approvals, "open": c.open, "message": c.message,
            "issue": issue}),
        );
    }
    let message: Vec<&str> = plans.iter().filter_map(|p| p["message"].as_str()).collect();
    Ok(Outcome::data(json!({"message": message.join("\n\n"),
        "plans": plans, "open": open, "warnings": warnings})))
}

/// Create the issue or update it (body, and open or closed to match the open count).
/// Returns its number and whether it was created.
pub(crate) fn post(
    gh: &dyn Github,
    issue: Option<&Issue>,
    title: &str,
    body: &str,
    open: usize,
    any: bool,
) -> Result<Option<(u64, bool)>> {
    let Some(i) = issue else {
        if !any {
            return Ok(None);
        }
        gh.ensure_label(LABEL, "0e8a16", "the owner checklist of a plan")?;
        let made = gh.create_issue(&format!("Owner checklist: {title}"), body, &[LABEL])?;
        return Ok(Some((made.number, true)));
    };
    let is_open = i.state == "open";
    let (state, state_reason) = match (open, is_open) {
        (0, true) => (Some(PatchState::Closed), Some("completed".to_string())),
        (1.., false) => (Some(PatchState::Open), None),
        _ => (None, None),
    };
    let patch = IssuePatch {
        body: Some(body.into()),
        state,
        state_reason,
        ..IssuePatch::default()
    };
    gh.update_issue(i.number, patch)?;
    Ok(Some((i.number, false)))
}

/// A plan path goes into the checklist issue's marker: plain repo-relative text only.
pub(crate) fn check_plan_path(p: &str) -> Result<()> {
    let bad = p.contains("--") || p.contains('<') || p.contains(char::is_whitespace);
    if bad || crate::core::text::has_hidden(p) || p.is_empty() {
        return Err(BosunError::usage(
            format!("plan path `{}` is not a plain file path", p.escape_debug()),
            "name the plan file with a plain repo-relative path, such as plans/bosun.md",
        ));
    }
    Ok(())
}

/// `Some(found)` when the API answered, `None` when it did not (owner, or a 403).
pub(crate) fn verify(
    gh: &dyn Github,
    v: &Verify,
    branch: &mut Option<String>,
) -> Result<Option<bool>> {
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
    let d = |i: &Value| {
        auto(
            format!("D{}", i["step"]),
            &text(i, "name"),
            &text(i, "fix"),
            "doctor",
        )
    };
    missing.map(d).collect()
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
        sum: String::new(),
        state: "api".into(),
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
