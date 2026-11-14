//! `bosun plan check|import`: read a plan file (docs/plan-format.md) and, for import, make the
//! epics, plan tasks and tasks as GitHub issues. Issues are found by their hidden marker
//! `<!-- bosun:plan <id> -->` (or adopted by their metadata) and never changed.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use super::prereqs::Env;
use super::prereqs_gate::{Held, blockers};
use super::{PlanAmendArgs, PlanCheckArgs, PlanCmd, PlanImportArgs};
use crate::app::{App, Result, default_branch, kill_switch, set_state, task_meta, trusted_notes};
use crate::config::{Config, Limits};
use crate::core::budget::feature_estimate;
use crate::core::glob::Glob;
use crate::core::lease::leases_of;
use crate::core::meta::{TaskMeta, closing_issues_in, linked_issues, parse_task_meta};
use crate::core::mode::BudgetFeature;
use crate::core::plan::{self, Item, Plan, PlanError, Task};
use crate::core::prereqs as pq;
use crate::core::risk::Rules;
use crate::core::shape;
use crate::core::state::{State, labels_to_remove, state_of};
use crate::github::{Github, Issue, IssuePatch, IssueState, PatchState, Probe};
use crate::output::{BosunError, CmdResult, Outcome};
use crate::time::Timestamp;

pub fn run(dir: Option<&Path>, cmd: &PlanCmd) -> CmdResult {
    match cmd {
        PlanCmd::Check(a) => check(dir, a),
        PlanCmd::Import(a) => run_import(dir, a, false),
        PlanCmd::Amend(a) => amend(dir, a),
        PlanCmd::Pr(a) => super::plan_pr::run(dir, a),
    }
}

/// The risk-rule path globs of `.bosun/risk.toml` under `root` (else the built-in rules): a task
/// touching one is gated. Advisory: `plan check` works without GitHub, so it reads the working
/// tree; import and amend use the default branch (`app::gated_from_default`).
fn gated_rules(root: &Path) -> Result<Vec<Glob>> {
    let rules = match std::fs::read_to_string(root.join(".bosun/risk.toml")) {
        Ok(t) => Rules::parse(&t).map_err(|e| {
            BosunError::error(
                format!(".bosun/risk.toml: {e}"),
                "fix .bosun/risk.toml, or remove it to use the built-in rules",
            )
        })?,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Rules::defaults(),
        Err(e) => {
            return Err(BosunError::error(
                format!("cannot read .bosun/risk.toml: {e}"),
                "make .bosun/risk.toml readable, or remove it to use the built-in rules",
            ));
        }
    };
    Ok(shape::gated_globs(&rules))
}

fn read_plan(dir: Option<&Path>, file: &str) -> Result<Plan> {
    read_plan_text(dir, file).map(|(_, p)| p)
}

/// The plan file's text and what `plan::parse` made of it.
fn read_plan_text(dir: Option<&Path>, file: &str) -> Result<(String, Plan)> {
    let path = dir.map_or_else(|| file.into(), |d| d.join(file));
    let text = std::fs::read_to_string(&path).map_err(|e| {
        BosunError::error(
            format!("cannot read {file}: {e}"),
            "pass the path of a plan file, for example plans/bosun.md",
        )
    })?;
    let plan = match plan::parse(&text) {
        Ok(p) => p,
        // Rule 6 counts only open tasks: look up the merged ones only when it is all that failed,
        // so a good plan is still checked without GitHub.
        Err(errs) if plan::only_critical_path(&errs) => {
            let done = App::load(dir)
                .and_then(|app| merged_plan_ids(app.gh.as_ref()))
                .map_err(|e| {
                    eprintln!(
                        "plan: cannot read the merged tasks ({}); rule 6 counts every task",
                        e.message
                    );
                    plan_errors(file, &errs)
                })?;
            plan::parse_with_done(&text, &done).map_err(|e| plan_errors(file, &e))?
        }
        Err(errs) => return Err(plan_errors(file, &errs)),
    };
    Ok((text, plan))
}

/// Plan ids (from the `bosun:plan` marker) of issues that are merged or closed as completed.
fn merged_plan_ids(gh: &dyn Github) -> Result<HashSet<String>> {
    Ok(all_issues(gh)?
        .iter()
        .filter(|i| {
            state_of(&i.labels) == Some(State::Merged)
                || (i.state == "closed" && i.state_reason.as_deref() == Some("completed"))
        })
        .filter_map(|i| {
            let rest = &i.body_text()[i.body_text().find("<!-- bosun:plan ")? + 16..];
            Some(rest[..rest.find(" -->")?].trim().to_string())
        })
        .collect())
}

pub(super) fn plan_errors(file: &str, errs: &[PlanError]) -> BosunError {
    let first = &errs[0];
    BosunError::error(
        format!("{} errors in {file}", errs.len()),
        format!("{}; {}", first.describe(file), first.fix),
    )
    .with_data(json!({ "errors": errs }))
}

/// `per_task_usd` when the `budget_feature` knob is not off.
fn per_task_if_on(config: &crate::config::Config, limits: &crate::config::Limits) -> Option<f64> {
    let on = config.mode_effective().knobs.budget_feature.value != BudgetFeature::Off;
    on.then_some(limits.budget.per_task_usd)
}

fn check(dir: Option<&Path>, a: &PlanCheckArgs) -> CmdResult {
    let p = read_plan(dir, &a.file)?;
    let root = dir.unwrap_or_else(|| Path::new("."));
    // No GitHub here: the working tree's files; a missing or bad one means the knob is off.
    let per_task = Config::load(root)
        .and_then(|c| Limits::load(root).map(|l| per_task_if_on(&c, &l)))
        .unwrap_or(None);
    let gated = gated_rules(dir.unwrap_or_else(|| Path::new(".")))?;
    let errs = shape::gated_size_errors(&p.items, &gated);
    if !errs.is_empty() {
        return Err(plan_errors(&a.file, &errs));
    }
    let items: Vec<Value> = p
        .items
        .iter()
        .map(|i| {
            let d = shape::effective(i, &gated);
            let mut row = json!({"id": i.id, "title": i.title, "area": i.area,
                "tasks": i.tasks.len(), "ucs": i.ucs.len(), "shape": d.shape,
                "shape_reasons": d.reasons, "gated": d.gated, "size": d.total});
            if let Some((usd, _)) = item_budget(i, per_task) {
                row["budget_usd"] = json!(usd);
            }
            row
        })
        .collect();
    Ok(Outcome::data(json!({"file": a.file, "title": p.title,
        "project": p.project, "items": items, "warnings": p.warnings})))
}

/// What the import knows about one issue of the plan.
#[derive(Clone)]
struct Slot {
    id: String,
    issue: Option<u64>,
    action: &'static str,
    state: Option<String>,
    closed: bool,
    /// The issue is merged: labelled `merged`, or closed as completed.
    merged: bool,
}

impl Slot {
    fn json(&self, with_state: bool) -> Value {
        let mut v = json!({"id": self.id, "issue": self.issue, "action": self.action});
        if with_state {
            v["state"] = json!(self.state);
        }
        if self.closed {
            v["closed"] = json!(true);
        }
        v
    }
}

fn marker(id: &str) -> String {
    format!("<!-- bosun:plan {id} -->")
}

pub(crate) fn epic_item(body: &str) -> Option<String> {
    let rest = &body[body.find("<!-- bosun:epic")? + "<!-- bosun:epic".len()..];
    let v: Value = serde_json::from_str(rest[..rest.find("-->")?].trim()).ok()?;
    v["item"].as_str().map(str::to_string)
}

/// The existing issue for a plan id: by marker, else adopted by metadata (`task` is `None` for
/// an epic). With several matches the lowest number wins, and a warning names them all.
fn lookup(
    issues: &[Issue],
    id: &str,
    item: &str,
    task: Option<&str>,
    warnings: &mut Vec<String>,
) -> Option<(Issue, bool)> {
    let m = marker(id);
    // Issues closed as duplicates keep the marker but never win and never warn.
    let issues: Vec<&Issue> = issues
        .iter()
        .filter(|i| i.state_reason.as_deref() != Some("duplicate"))
        .collect();
    let mut found: Vec<&Issue> = issues
        .iter()
        .copied()
        .filter(|i| i.body_text().contains(&m))
        .collect();
    let adopted = found.is_empty();
    if adopted {
        found = issues
            .iter()
            .copied()
            .filter(|i| match task {
                Some(t) => task_meta(i).is_some_and(|m| m.item == item && m.task == t),
                None => epic_item(i.body_text()).as_deref() == Some(item),
            })
            .collect();
    }
    found.sort_by_key(|i| i.number);
    let first = *found.first()?;
    if found.len() > 1 {
        let nums: Vec<String> = found.iter().map(|i| format!("#{}", i.number)).collect();
        warnings.push(format!(
            "{id} matches {}; using #{}",
            nums.join(" and "),
            first.number
        ));
    }
    Some((first.clone(), adopted))
}

struct Importer<'a> {
    gh: &'a dyn Github,
    issues: Vec<Issue>,
    dry: bool,
    ensured: HashSet<String>,
    epics: Vec<Slot>,
    tasks: Vec<Slot>,
    warnings: Vec<String>,
    /// `done-by` fields that were refused: the import still runs, then it exits 1.
    refused: Vec<String>,
    dir: PathBuf,
    repo: String,
    /// `per_task_usd` when the `budget_feature` knob is on: the epic then carries an estimate.
    per_task: Option<f64>,
    /// The session that writes the bind records (modes 7b): `Some` when the knob is not off.
    bind: Option<String>,
    promoted: Vec<String>,
    /// Open owner-checklist items of the plan being imported, and what they hold back.
    open_items: Vec<Held>,
    held: Vec<Value>,
    /// Plan tasks closed because their item is on the default branch already (F5).
    on_default: Vec<String>,
    /// Plan tasks left open by `--keep-plan-tasks-open` although the file is on the default branch.
    on_default_open: Vec<String>,
    /// The local plan file's text, compared byte for byte with the default branch's (F5).
    local_text: String,
    /// `allow:` per task id of the plan file on the default branch, read once when a task has one.
    main_tasks: Option<HashMap<String, Task>>,
    /// Path globs of the risk rules: a task touching one is gated (D-016).
    gated: shape::Gated,
    /// `plan amend`: a new item counts as merged only through an intake (F5 is import only).
    amend: bool,
    /// The plan path as stored in epics: relative to the repo root.
    plan_path: String,
    roots: Vec<PathBuf>,
    now: Timestamp,
}

impl Importer<'_> {
    /// The `allow:` of task `t` (plan id `id`) that may go into its metadata: test-integrity
    /// trusts it for the files of the task's `touch`, so the whole grant must be owner-approved,
    /// which means merged. It is `t.allow` only when the same task in the plan file on the
    /// default branch has the same `allow`, `touch`, `depends`, `kind` and `risk` (a local
    /// `touch: **` must not borrow main's `allow`); otherwise none, with a warning (a missing or
    /// unreadable file there allows nothing). Plans set no `capabilities`.
    fn allowed(&mut self, id: &str, t: &Task) -> Vec<String> {
        if t.allow.is_empty() {
            return vec![];
        }
        if self.main_tasks.is_none() {
            let read =
                default_branch(self.gh).and_then(|head| self.gh.file_at(&self.plan_path, &head));
            let map = match read {
                Ok(text) => text
                    .and_then(|t| plan::parse_to_compare(&t).ok())
                    .map(|p| {
                        p.items
                            .iter()
                            .flat_map(|i| i.tasks.iter().map(|t| (t.plan_id(&i.id), t.clone())))
                            .collect()
                    })
                    .unwrap_or_default(),
                Err(e) => {
                    self.warnings.push(format!(
                        "cannot read {} on the default branch ({}); no task gets `allow:`",
                        self.plan_path, e.message
                    ));
                    HashMap::new()
                }
            };
            self.main_tasks = Some(map);
        }
        fn same_grant(a: &Task, b: &Task) -> bool {
            (&a.allow, &a.touch, &a.depends, &a.kind, &a.risk)
                == (&b.allow, &b.touch, &b.depends, &b.kind, &b.risk)
        }
        let main = self.main_tasks.as_ref().and_then(|m| m.get(id));
        if main.is_some_and(|m| same_grant(m, t)) {
            return t.allow.clone();
        }
        self.warnings.push(format!(
            "{id}: `allow: {}` with this touch, depends, kind and risk is not in {} on the default branch; imported without it (merge the plan, then run `bosun plan amend`)",
            t.allow.join(", "),
            self.plan_path
        ));
        vec![]
    }

    /// Find or create one issue.
    fn slot(
        &mut self,
        id: &str,
        key: (&str, Option<&str>),
        make: impl FnOnce() -> (String, String, Vec<String>),
    ) -> Result<Slot> {
        let found = lookup(&self.issues, id, key.0, key.1, &mut self.warnings);
        if let Some((i, adopted)) = found {
            let slot = Slot {
                id: id.into(),
                issue: Some(i.number),
                action: if adopted { "adopted" } else { "existing" },
                state: state_of(&i.labels).map(|s| s.name().to_string()),
                closed: i.state == "closed",
                merged: state_of(&i.labels) == Some(State::Merged)
                    || (i.state == "closed" && i.state_reason.as_deref() == Some("completed")),
            };
            self.keep(&slot, key.1.is_none());
            return Ok(slot);
        }
        let (title, body, labels) = make();
        let state = labels
            .iter()
            .find_map(|l| l.strip_prefix("bosun:state/"))
            .map(str::to_string);
        if self.dry {
            let slot = Slot {
                id: id.into(),
                issue: None,
                action: "would-create",
                state,
                closed: false,
                merged: false,
            };
            self.keep(&slot, key.1.is_none());
            return Ok(slot);
        }
        for l in &labels {
            if self.ensured.insert(l.clone()) {
                let color = if l.starts_with("bosun:state/") {
                    "ededed"
                } else {
                    "5319e7"
                };
                self.gh.ensure_label(l, color, "bosun plan label")?;
            }
        }
        let refs: Vec<&str> = labels.iter().map(String::as_str).collect();
        let created = self.gh.create_issue(&title, &body, &refs)?;
        let slot = Slot {
            id: id.into(),
            issue: Some(created.number),
            action: "created",
            state,
            closed: false,
            merged: false,
        };
        self.keep(&slot, key.1.is_none());
        Ok(slot)
    }

    fn keep(&mut self, slot: &Slot, epic: bool) {
        let list = if epic {
            &mut self.epics
        } else {
            &mut self.tasks
        };
        list.push(slot.clone());
    }

    /// What was found or made so far: the output, and the data of an error part-way through.
    fn data(&self, file: &str) -> Value {
        let promote = if self.dry {
            "would_promote"
        } else {
            "promoted"
        };
        json!({
            "file": file,
            "dry_run": self.dry,
            "epics": self.epics.iter().map(|s| s.json(false)).collect::<Vec<_>>(),
            "tasks": self.tasks.iter().map(|s| s.json(true)).collect::<Vec<_>>(),
            promote: self.promoted,
            "held": self.held,
            "on_default": self.on_default,
            "on_default_open": self.on_default_open,
            "warnings": self.warnings,
        })
    }

    /// Epics that `reap` closed as complete (closed and done) that have an open task again open
    /// as `ready`. Decided from this run's slots, with no list call (GitHub's list lags): a task
    /// slot not merged and not closed, or one `reconcile` reopened (`reopened`, plan ids).
    /// Returns the plan ids of the epics (would-be ones in a dry run).
    fn reopen_done_epics(&mut self, reopened: &[String]) -> Result<Vec<String>> {
        let mut out = Vec::new();
        for k in 0..self.epics.len() {
            let e = self.epics[k].clone();
            let Some(number) = e.issue.filter(|_| e.closed && e.merged) else {
                continue;
            };
            let live = self.tasks.iter().any(|t| {
                t.id.split('/').next() == Some(e.id.as_str())
                    && !t.merged
                    && (!t.closed || reopened.contains(&t.id))
            });
            if !live {
                continue;
            }
            let needs = blockers(&self.open_items, &e.id, None);
            if !needs.is_empty() {
                self.held.push(json!({"task": e.id, "needs": needs}));
                continue;
            }
            if !self.dry {
                let labels = self.issues.iter().find(|i| i.number == number);
                let labels = labels.map(|i| i.labels.clone()).unwrap_or_default();
                let ready = State::Ready.label();
                self.gh.ensure_label(&ready, "ededed", "bosun task state")?;
                let open = IssuePatch {
                    state: Some(PatchState::Open),
                    ..IssuePatch::default()
                };
                self.gh.update_issue(number, open)?;
                for l in labels_to_remove(&labels, State::Ready) {
                    self.gh.remove_label(number, &l)?;
                }
                self.gh.add_labels(number, &[&ready])?;
                self.epics[k].closed = false;
                self.epics[k].state = Some("ready".into());
            }
            out.push(e.id);
        }
        Ok(out)
    }

    /// Once the plan task of an item merged, the epic and the tasks this plan made move from
    /// `planned` to `ready`. Adopted hand-made issues are never touched.
    fn promote(&mut self, epic: &Slot, pslot: &Slot, tasks: &[Slot]) -> Result<()> {
        if !pslot.merged {
            return Ok(());
        }
        for s in std::iter::once(epic).chain(tasks) {
            if s.action == "adopted" || s.closed || s.state.as_deref() != Some("planned") {
                continue;
            }
            let (item, task) = match s.id.split_once('/') {
                Some((i, t)) => (i, Some(t)),
                None => (s.id.as_str(), None),
            };
            let needs = blockers(&self.open_items, item, task);
            if !needs.is_empty() {
                self.held.push(json!({"task": s.id, "needs": needs}));
                continue;
            }
            if !self.dry {
                let number = s.issue.expect("a made or found issue has a number");
                let issue = match self.issues.iter().find(|i| i.number == number) {
                    Some(i) => i.clone(),
                    None => self.gh.get_issue(number)?.ok_or_else(|| {
                        BosunError::error(
                            format!("#{number} ({}) vanished during the import", s.id),
                            "run `bosun plan import` again",
                        )
                    })?,
                };
                set_state(self.gh, number, &issue, State::Ready)?;
            }
            self.promoted.push(s.id.clone());
        }
        Ok(())
    }
}

fn labels(kind: &str, area: &str, state: &str) -> Vec<String> {
    vec![
        format!("bosun:kind/{kind}"),
        format!("bosun:area/{area}"),
        "bosun:source/plan".into(),
        format!("bosun:state/{state}"),
    ]
}

/// The feature budget of an item and where it comes from: its `budget_usd:`, else (knob not off)
/// tasks times `per_task_usd`. `None` is no budget: the output stays as before the knob.
fn item_budget(item: &Item, per_task: Option<f64>) -> Option<(f64, &'static str)> {
    match (item.budget_usd, per_task) {
        (Some(b), _) => Some((b, "plan")),
        (None, Some(p)) => Some((feature_estimate(item.tasks.len(), p), "per_task")),
        _ => None,
    }
}

fn epic_body(item: &Item, file: &str, gated: &shape::Gated, per_task: Option<f64>) -> String {
    let tasks: String = item
        .tasks
        .iter()
        .map(|t| format!("- [ ] {}: {}\n", t.plan_id(&item.id), t.title))
        .collect();
    let ucs: String = item
        .ucs
        .iter()
        .map(|u| format!("- {}: {}\n", u.id, u.text))
        .collect();
    let ids: Vec<&str> = item.ucs.iter().map(|u| u.id.as_str()).collect();
    let mut meta = json!({"item": item.id, "area": item.area, "plan": file,
        "depends": item.depends, "ucs": ids,
        "shape": shape::effective_by(item, &|i, t| gated.is(i, t)).shape});
    if let Some((usd, source)) = item_budget(item, per_task) {
        meta["budget_usd"] = json!(usd);
        meta["budget_source"] = json!(source);
    }
    format!(
        "{}\n{}\n\n{tasks}\n{ucs}\n<!-- bosun:epic {meta} -->",
        marker(&item.id),
        item.description
    )
}

fn task_body(id: &str, intro: &str, meta: &TaskMeta) -> String {
    let json = serde_json::to_string(meta).expect("meta serializes");
    format!("{}\n\n{intro}<!-- bosun:task {json} -->", marker(id))
}

fn plan_task_meta(item: &Item, plan_path: &str, epic: Option<u64>) -> TaskMeta {
    TaskMeta {
        item: item.id.clone(),
        task: "P".into(),
        epic,
        touch: vec![plan_path.to_string()],
        depends: vec![],
        kind: "plan".into(),
        risk: "normal".into(),
        allow: vec![],
        capabilities: vec![],
        gated: false,
        size: None,
    }
}

fn task_meta_of(
    item: &Item,
    t: &Task,
    epic: Option<u64>,
    allow: Vec<String>,
    gated: &shape::Gated,
) -> TaskMeta {
    TaskMeta {
        item: item.id.clone(),
        task: t.id.clone(),
        epic,
        touch: t.touch.clone(),
        depends: t.depends.clone(),
        kind: t.kind.clone(),
        risk: t.risk.clone(),
        allow,
        capabilities: vec![],
        gated: gated.is(&item.id, t),
        size: t.size,
    }
}

/// `plan import` of every item, then the reconcile (UC-M2-4).
fn amend(dir: Option<&Path>, a: &PlanAmendArgs) -> CmdResult {
    let args = PlanImportArgs {
        file: a.file.clone(),
        dry_run: a.dry_run,
        item: vec![],
        keep_plan_tasks_open: false,
        full: a.full,
    };
    run_import(dir, &args, true)
}

fn run_import(dir: Option<&Path>, a: &PlanImportArgs, amend: bool) -> CmdResult {
    let (text, p) = read_plan_text(dir, &a.file)?;
    // Always parsed: a misspelt heading is an error here, never "no section". Nothing is written.
    let prereqs = pq::parse(&text, &p).map_err(|e| plan_errors(&a.file, &e))?;
    let app = App::load(dir)?;
    if let Some(project) = &p.project
        && *project != app.ctx.config.project
    {
        return Err(BosunError::error(
            format!(
                "{} is for project `{project}` but this repo's project is `{}`",
                a.file, app.ctx.config.project
            ),
            "import the plan of this project, or fix `project:` in the plan or in .bosun/bosun.toml",
        ));
    }
    if let Some(bad) = a.item.iter().find(|i| !p.items.iter().any(|x| &x.id == *i)) {
        return Err(BosunError::usage(
            format!("--item {bad} is not an item of {}", a.file),
            format!(
                "use one of: {}",
                p.items
                    .iter()
                    .map(|i| i.id.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        ));
    }
    if !a.dry_run {
        kill_switch(app.gh.as_ref())?;
    }
    let base = std::fs::canonicalize(dir.unwrap_or_else(|| Path::new(".")))
        .unwrap_or_else(|_| dir.unwrap_or_else(|| Path::new(".")).to_path_buf());
    let globs = crate::app::gated_from_default(app.gh.as_ref())?;
    // Wildcard touches are resolved against the default branch's tree (one read).
    let files = crate::app::base_files(app.gh.as_ref())?;
    let (resolved, mut errs) = shape::resolve_gated(&p.items, &globs, files.as_deref());
    let gated = shape::Gated { globs, resolved };
    errs.extend(shape::gated_size_errors_by(&p.items, &|i, t| {
        gated.is(i, t)
    }));
    if !errs.is_empty() {
        return Err(plan_errors(&a.file, &errs));
    }
    let mut roots = vec![app.ctx.dir.clone()];
    roots.extend(std::fs::canonicalize(&app.ctx.dir).ok());
    let knob = super::budget::knob_from_default(app.gh.as_ref())?;
    let mut imp = Importer {
        gh: app.gh.as_ref(),
        issues: all_issues(app.gh.as_ref())?,
        dry: a.dry_run,
        ensured: HashSet::new(),
        epics: vec![],
        tasks: vec![],
        warnings: vec![],
        refused: vec![],
        dir: app.ctx.dir.clone(),
        repo: app.ctx.repo.to_string(),
        per_task: match knob {
            BudgetFeature::Off => None,
            _ => Some(
                crate::app::limits_from_default(app.gh.as_ref(), &app.ctx)?
                    .budget
                    .per_task_usd,
            ),
        },
        bind: (knob != BudgetFeature::Off).then(|| app.ctx.session.clone()),
        promoted: vec![],
        open_items: vec![],
        held: vec![],
        on_default: vec![],
        on_default_open: vec![],
        local_text: text.clone(),
        main_tasks: None,
        gated,
        amend,
        plan_path: plan_key(&a.file, &base, &roots),
        roots,
        now: app.ctx.now,
    };
    if !prereqs.is_empty()
        && let Err(e) = refresh_checklist(&mut imp, &app, dir, a, &text)
    {
        return Err(e.with_data(imp.data(&a.file)));
    }
    let mut data = match import_items(&mut imp, &p, a) {
        Ok(()) => imp.data(&a.file),
        Err(e) => return Err(e.with_data(imp.data(&a.file))),
    };
    if amend {
        match reconcile(&mut imp, &p) {
            Ok(ch) => {
                let dry = a.dry_run;
                let key =
                    |plain: &'static str, would: &'static str| if dry { would } else { plain };
                data[key("closed", "would_close")] = json!(ch.closed);
                data[key("updated", "would_update")] = json!(ch.updated);
                data[key("reopened", "would_reopen")] = json!(ch.reopened);
                data["skipped"] = json!(ch.skipped);
                data["warnings"] = json!(imp.warnings);
            }
            Err(e) => return Err(e.with_data(data)),
        }
    }
    let again: Vec<String> = data["reopened"]
        .as_array()
        .into_iter()
        .flatten()
        .chain(data["would_reopen"].as_array().into_iter().flatten())
        .filter_map(|v| v.as_str().map(String::from))
        .collect();
    let epics = imp
        .reopen_done_epics(&again)
        .map_err(|e| e.with_data(data.clone()))?;
    data["held"] = json!(imp.held);
    if !epics.is_empty() {
        let key = if a.dry_run {
            "would_reopen"
        } else {
            "reopened"
        };
        data["epics"] = imp
            .epics
            .iter()
            .map(|e| e.json(epics.contains(&e.id)))
            .collect();
        let mut all = data[key].as_array().cloned().unwrap_or_default();
        all.extend(epics.iter().map(|id| json!(id)));
        data[key] = Value::Array(all);
    }
    data["warnings"] = json!(imp.warnings);
    if !imp.refused.is_empty() {
        return Err(BosunError::error(
            imp.refused.join("; "),
            "merge the PR and name the task in it (`Closes #<issue>` or its plan id), or remove `done-by:` from the plan; the rest of the import was done",
        )
        .with_data(data));
    }
    if !a.full {
        summarize_import(&mut data);
    }
    Ok(Outcome::data(data))
}

/// The default view: epics and tasks the import left as they were are counted, not listed.
fn summarize_import(data: &mut Value) {
    let mut unchanged = serde_json::Map::new();
    for key in ["epics", "tasks"] {
        if let Some(rows) = data[key].as_array() {
            let (same, changed): (Vec<_>, Vec<_>) =
                rows.iter().cloned().partition(|r| r["action"] == "existing");
            unchanged.insert(key.into(), json!(same.len()));
            data[key] = Value::Array(changed);
        }
    }
    data["unchanged"] = Value::Object(unchanged);
    data["summary"] = json!(true);
}

/// A plan with prerequisites gets its checklist issue refreshed (as `bosun
/// prereqs --post --no-setup`; read-only with `--dry-run`); its open items hold tasks back.
fn refresh_checklist(
    imp: &mut Importer,
    app: &App,
    dir: Option<&Path>,
    a: &PlanImportArgs,
    text: &str,
) -> Result<()> {
    let mut env = Env::load(app, dir, &mut imp.warnings)?;
    env.one_issue(&imp.plan_path)?;
    let c = env.checklist(app, &a.file, text, !a.dry_run, &[], &mut imp.warnings)?;
    let open = c.items.into_iter().filter(|i| !i.done);
    imp.open_items = open
        .map(|i| Held {
            id: i.id,
            blocks: i.blocks,
        })
        .collect();
    Ok(())
}

/// The `plan` file named in the metadata of an epic body.
pub(crate) fn epic_plan(body: &str) -> Option<String> {
    let rest = &body[body.find("<!-- bosun:epic")? + "<!-- bosun:epic".len()..];
    let v: Value = serde_json::from_str(rest[..rest.find("-->")?].trim()).ok()?;
    v["plan"].as_str().map(str::to_string)
}

/// The `shape` (`single` or `split`) named in the metadata of an epic body.
pub(crate) fn epic_shape(body: &str) -> Option<String> {
    let rest = &body[body.find("<!-- bosun:epic")? + "<!-- bosun:epic".len()..];
    let v: Value = serde_json::from_str(rest[..rest.find("-->")?].trim()).ok()?;
    v["shape"].as_str().map(str::to_string)
}

/// Replace the task metadata comment of `body`, keeping every other byte.
fn replace_task_meta(body: &str, meta: &TaskMeta) -> Option<String> {
    let start = body.rfind(crate::core::meta::OPEN)?;
    let end = start + body[start..].find("-->")? + "-->".len();
    let json = serde_json::to_string(meta).expect("meta serializes");
    Some(format!(
        "{}<!-- bosun:task {json} -->{}",
        &body[..start],
        &body[end..]
    ))
}

/// What a reconcile did, by plan id.
#[derive(Default)]
struct Changes {
    closed: Vec<String>,
    updated: Vec<String>,
    skipped: Vec<String>,
    reopened: Vec<String>,
}

/// `p` without `.` and resolved `..` components.
fn clean(p: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for c in p.components() {
        match c {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir if out.file_name().is_some() => {
                out.pop();
            }
            c => out.push(c),
        }
    }
    out
}

fn slashed(p: &Path) -> String {
    p.components()
        .map(|c| c.as_os_str().to_string_lossy().into_owned())
        .collect::<Vec<_>>()
        .join("/")
}

/// A plan path as the epics store it: relative to the repo root, with `/`, without `./`.
/// A relative `file` is resolved against `base`; a path outside every root stays as given.
pub(super) fn plan_key(file: &str, base: &Path, roots: &[PathBuf]) -> String {
    let joined = clean(&base.join(file));
    let canon = std::fs::canonicalize(&joined).unwrap_or_else(|_| joined.clone());
    for cand in [&canon, &joined] {
        for r in roots {
            if let Ok(rel) = cand.strip_prefix(r)
                && !rel.as_os_str().is_empty()
            {
                return slashed(rel);
            }
        }
    }
    slashed(&clean(Path::new(file)))
}

/// Why `i` must not be closed now: it is in an active state or holds a live lease.
fn blocked(imp: &Importer, i: &Issue) -> Result<Option<String>> {
    if let Some(s) = state_of(&i.labels)
        && s.is_active()
    {
        return Ok(Some(s.name().to_string()));
    }
    let n = trusted_notes(imp.gh, i.number)?;
    let live = leases_of(i.number, &n).iter().any(|l| l.is_live(imp.now));
    Ok(live.then(|| "leased".to_string()))
}

/// Close what the plan file dropped, reopen what it brought back, and update tasks whose
/// definition changed. Only issues with the plan marker whose epic names this file are touched.
fn reconcile(imp: &mut Importer, p: &Plan) -> Result<Changes> {
    let file = imp.plan_path.clone();
    let issues: Vec<Issue> = imp.issues.clone();
    let ours = |i: &Issue| i.state_reason.as_deref() != Some("duplicate");
    let marked = |i: &Issue| i.body_text().starts_with("<!-- bosun:plan ");
    let same = |s: &str| plan_key(s, &imp.dir, &imp.roots) == file;
    let epics: HashSet<u64> = issues
        .iter()
        .filter(|i| ours(i) && marked(i) && epic_plan(i.body_text()).is_some_and(|s| same(&s)))
        .map(|i| i.number)
        .collect();
    if epics.is_empty()
        && issues.iter().any(|i| {
            ours(i)
                && marked(i)
                && epic_item(i.body_text()).is_some_and(|it| p.items.iter().any(|x| x.id == it))
        })
    {
        imp.warnings.push(format!(
            "no epic names {file}; nothing reconciled — check the path"
        ));
    }
    // A task of this plan: its metadata and plan id.
    let task_of = |i: &Issue| -> Option<(TaskMeta, String)> {
        let meta = task_meta(i)?;
        let id = format!("{}/{}", meta.item, meta.task);
        (marked(i)
            && i.body_text().contains(&marker(&id))
            && meta.epic.is_some_and(|e| epics.contains(&e)))
        .then_some((meta, id))
    };
    let in_plan = |meta: &TaskMeta| {
        p.items
            .iter()
            .find(|x| x.id == meta.item)
            .is_some_and(|x| meta.task == "P" || x.tasks.iter().any(|t| t.id == meta.task))
    };
    let mut ch = Changes::default();
    let comment = |imp: &Importer, i: &Issue, text: &str, patch: IssuePatch| -> Result<()> {
        if !imp.dry {
            imp.gh.add_comment(i.number, text)?;
            imp.gh.update_issue(i.number, patch)?;
        }
        Ok(())
    };
    // Items that came back: reopen what an earlier amendment closed as not planned.
    let mut reopened: HashSet<u64> = HashSet::new();
    for i in issues
        .iter()
        .filter(|i| i.state == "closed" && i.state_reason.as_deref() == Some("not_planned"))
    {
        let id = if epics.contains(&i.number) {
            epic_item(i.body_text()).filter(|it| p.items.iter().any(|x| &x.id == it))
        } else {
            task_of(i).filter(|(m, _)| in_plan(m)).map(|(_, id)| id)
        };
        if let Some(id) = id {
            let patch = IssuePatch {
                state: Some(PatchState::Open),
                ..IssuePatch::default()
            };
            comment(imp, i, "Re-added by an amendment.", patch)?;
            reopened.insert(i.number);
            ch.reopened.push(id);
        }
    }
    let live: Vec<&Issue> = issues
        .iter()
        .filter(|i| (i.state == "open" || reopened.contains(&i.number)) && ours(i))
        .collect();
    let dropped = |meta: &TaskMeta| !in_plan(meta);
    // Dropped tasks that must stay open, and the epics holding them.
    let mut busy: HashMap<u64, Option<String>> = HashMap::new();
    let mut busy_epics: HashSet<u64> = HashSet::new();
    for i in &live {
        if let Some((meta, _)) = task_of(i)
            && dropped(&meta)
            && !reopened.contains(&i.number)
        {
            let b = blocked(imp, i)?;
            if b.is_some() {
                busy_epics.extend(meta.epic);
            }
            busy.insert(i.number, b);
        }
    }
    let reason = format!("Removed from {file} by an amendment.");
    let closing = IssuePatch {
        state: Some(PatchState::Closed),
        state_reason: Some("not_planned".into()),
        ..IssuePatch::default()
    };
    for i in live {
        let body = i.body_text();
        if epics.contains(&i.number) {
            let item = epic_item(body).unwrap_or_default();
            if p.items.iter().any(|x| x.id == item) || reopened.contains(&i.number) {
                continue;
            }
            let why = if busy_epics.contains(&i.number) {
                Some("has active tasks".to_string())
            } else {
                blocked(imp, i)?
            };
            if let Some(why) = why {
                imp.warnings.push(if why == "has active tasks" {
                    format!(
                        "{item} #{} has active tasks; left open — finish or release them, then amend again",
                        i.number
                    )
                } else {
                    format!(
                        "{item} #{} is {why}; not closed — finish or release it, then amend again",
                        i.number
                    )
                });
                ch.skipped.push(item);
                continue;
            }
            comment(imp, i, &reason, closing.clone())?;
            ch.closed.push(item);
            continue;
        }
        let Some((meta, id)) = task_of(i) else {
            continue;
        };
        if reopened.contains(&i.number) {
            continue;
        }
        if dropped(&meta) {
            if let Some(Some(why)) = busy.get(&i.number) {
                imp.warnings.push(format!(
                    "{id} #{} is {why}; not closed — finish or release it, then amend again",
                    i.number
                ));
                ch.skipped.push(id);
                continue;
            }
            comment(imp, i, &reason, closing.clone())?;
            ch.closed.push(id);
            continue;
        }
        if meta.task == "P" {
            continue;
        }
        let t = p
            .items
            .iter()
            .find(|x| x.id == meta.item)
            .and_then(|x| x.tasks.iter().find(|t| t.id == meta.task))
            .expect("in_plan checked");
        let title = format!("{id}: {}", t.title);
        let mut new = meta.clone();
        (new.touch, new.depends) = (t.touch.clone(), t.depends.clone());
        (new.kind, new.risk) = (t.kind.clone(), t.risk.clone());
        new.allow = imp.allowed(&id, t);
        (new.gated, new.size) = (imp.gated.is(&meta.item, t), t.size);
        if new == meta && title == i.title {
            continue;
        }
        if !imp.dry {
            let body = replace_task_meta(body, &new).expect("meta was parsed from this body");
            imp.gh.update_issue(
                i.number,
                IssuePatch {
                    title: Some(title),
                    body: Some(body),
                    ..IssuePatch::default()
                },
            )?;
            if new.kind != meta.kind {
                let label = format!("bosun:kind/{}", new.kind);
                imp.gh.ensure_label(&label, "5319e7", "bosun plan label")?;
                for old in i.labels.iter().filter(|l| l.starts_with("bosun:kind/")) {
                    imp.gh.remove_label(i.number, old)?;
                }
                imp.gh.add_labels(i.number, &[&label])?;
            }
        }
        ch.updated.push(id);
    }
    Ok(ch)
}

/// Every issue, including ones the list does not show yet: GitHub's list lags seconds behind
/// creation, while GET by number is consistent. Probe the numbers after the highest listed one.
fn all_issues(gh: &dyn Github) -> Result<Vec<Issue>> {
    let mut issues = gh.list_issues(&[], IssueState::All)?;
    let first = issues.iter().map(|i| i.number).max().unwrap_or(0) + 1;
    let (mut found, mut probes) = (0, 0);
    // Stop at the first number that does not exist. Pull requests and deleted or transferred
    // numbers are skipped; only issues the list lacked count toward the cap of 50.
    for next in first.. {
        probes += 1;
        if found >= 50 || probes > 1000 {
            break;
        }
        match gh.probe_issue(next)? {
            Probe::Issue(i) => {
                if !i.pull_request && !issues.iter().any(|x| x.number == i.number) {
                    issues.push(*i);
                    found += 1;
                }
            }
            Probe::Gone => {}
            Probe::Missing => break,
        }
    }
    Ok(issues)
}

/// Items among `wanted` that a merged intake PR added to the plan file `plan_path`, as item id ->
/// (intake, PR). An intake counts when it touches this plan file and is merged, or closed as
/// completed. Its PR is a closed, merged pull request into the default branch that links it; the
/// plan file at its merge commit's first parent and at the merge commit are parsed, and the ids only the later one
/// has were added by that PR. A file missing at the parent has no ids; an unparsable one counts the
/// same. Newest intakes first, and the scan stops once every wanted id has an owner.
fn intake_added(imp: &Importer, wanted: &[&str]) -> Result<HashMap<String, (u64, u64)>> {
    let want = imp.plan_path.trim_start_matches("./");
    let mut intakes: Vec<&Issue> = imp
        .issues
        .iter()
        .filter(|i| !i.pull_request && i.labels.iter().any(|l| l == "bosun:kind/plan"))
        .filter(|i| {
            state_of(&i.labels) == Some(State::Merged)
                || (i.state == "closed" && i.state_reason.as_deref() == Some("completed"))
        })
        .filter(|i| {
            parse_task_meta(i.body_text()).is_some_and(|m| {
                m.task == "intake" && m.touch.iter().any(|t| t.trim_start_matches("./") == want)
            })
        })
        .collect();
    intakes.sort_by_key(|i| std::cmp::Reverse(i.number));
    let mut added = HashMap::new();
    if intakes.is_empty() {
        return Ok(added);
    }
    // A missing file has no ids; one that exists but does not parse says nothing (`None`).
    let ids = |text: Option<String>| -> Option<HashSet<String>> {
        let Some(t) = text else {
            return Some(HashSet::new());
        };
        plan::parse_to_compare(&t)
            .ok()
            .map(|p| p.items.into_iter().map(|i| i.id).collect())
    };
    let base = default_branch(imp.gh)?;
    let pulls = imp.gh.list_pulls(None, IssueState::Closed)?;
    for intake in intakes {
        if wanted.iter().all(|w| added.contains_key(*w)) {
            break;
        }
        for p in pulls
            .iter()
            .filter(|p| p.base_ref == base && linked_issues(&p.body).contains(&intake.number))
        {
            let Some(full) = imp.gh.get_pull(p.number)?.filter(|f| f.merged) else {
                continue;
            };
            let after = full.merge_commit_sha.as_deref().unwrap_or(&full.head_sha);
            // What this PR added is the change from the merge commit's first parent, not from the
            // PR's base: other PRs may have reached the branch in between. An unknown merge commit
            // tells nothing, so the intake is not trusted.
            let Some(parent) = imp.gh.first_parent(after)? else {
                eprintln!(
                    "plan: GitHub does not know PR #{}'s merge commit {after}; intake #{} is not trusted",
                    p.number, intake.number
                );
                continue;
            };
            let before = ids(imp.gh.file_at(&imp.plan_path, &parent)?);
            let now = ids(imp.gh.file_at(&imp.plan_path, after)?);
            let (Some(before), Some(now)) = (before, now) else {
                eprintln!(
                    "plan: {} does not parse at PR #{}'s merge commit or its parent; intake #{} is not trusted",
                    imp.plan_path, p.number, intake.number
                );
                continue;
            };
            for id in now.difference(&before) {
                added.entry(id.clone()).or_insert((intake.number, p.number));
            }
        }
    }
    Ok(added)
}

/// Whether two parses of an item agree on everything that decides what gets built (line numbers
/// and the free-text description aside).
fn same_item(a: &plan::Item, b: &plan::Item) -> bool {
    let task = |t: &plan::Task| {
        (
            t.id.clone(),
            t.title.clone(),
            t.touch.clone(),
            t.depends.clone(),
            t.size,
            t.kind.clone(),
            t.risk.clone(),
        )
    };
    let uc = |u: &plan::Uc| (u.id.clone(), u.text.clone());
    (&a.id, &a.title, &a.area, &a.depends) == (&b.id, &b.title, &b.area, &b.depends)
        && a.tasks.iter().map(task).eq(b.tasks.iter().map(task))
        && a.ucs.iter().map(uc).eq(b.ucs.iter().map(uc))
}

/// The plan file on the default branch; `None` when it is missing or does not parse.
fn plan_on_default(imp: &Importer) -> Result<Option<Plan>> {
    let head = default_branch(imp.gh)?;
    Ok(imp
        .gh
        .file_at(&imp.plan_path, &head)?
        .and_then(|t| plan::parse_to_compare(&t).ok()))
}

/// Whether the plan file on the default branch is byte-identical to the local one.
/// A failed read counts as "not the same" (nothing is closed) and adds a warning.
fn same_file_on_default(imp: &mut Importer) -> bool {
    let read = default_branch(imp.gh).and_then(|head| imp.gh.file_at(&imp.plan_path, &head));
    match read {
        Ok(text) => text.as_deref() == Some(imp.local_text.as_str()),
        Err(e) => {
            imp.warnings.push(format!(
                "cannot read {} on the default branch ({}); no plan task is closed as already merged",
                imp.plan_path, e.message
            ));
            false
        }
    }
}

fn has_plan_task(issues: &[Issue], pid: &str) -> bool {
    let m = marker(pid);
    issues.iter().any(|i| i.body_text().contains(&m))
}

/// Why PR `pr` cannot vouch for task `tid` (`number`: its issue when it exists), or `None` when
/// it can: it must be merged into the default branch and link the task, by `Closes #<issue>` or by
/// naming the plan id (`M1/T2`) in its title or body. A PR the author merged for other work does
/// not close a task.
fn done_by_refusal(
    imp: &Importer,
    tid: &str,
    number: Option<u64>,
    pr: u64,
) -> Result<Option<String>> {
    let base = default_branch(imp.gh)?;
    let Some(p) = imp.gh.get_pull(pr)? else {
        return Ok(Some(format!("#{pr} is not a pull request")));
    };
    if !p.merged || p.base_ref != base {
        return Ok(Some(format!("PR #{pr} is not merged into {base}")));
    }
    let text = format!("{}\n{}", p.title, p.body);
    let named = regex::Regex::new(&format!(
        r"(?:^|[^A-Za-z0-9/]){}(?:[^A-Za-z0-9/]|$)",
        regex::escape(tid)
    ))
    .is_ok_and(|re| re.is_match(&text));
    let closes = number.is_some_and(|n| closing_issues_in(&p.body, Some(&imp.repo)).contains(&n));
    Ok((!named && !closes).then(|| format!("PR #{pr} does not close {tid} or name it")))
}

/// A task whose `done-by` PR merged: one just made (already labelled merged) or an open
/// planned/ready one is closed as completed with a note; anything else is left alone.
fn finish_done_by(imp: &mut Importer, slot: &mut Slot, pr: u64) -> Result<()> {
    let made = matches!(slot.action, "created" | "would-create");
    let open_early = !made
        && slot.issue.is_some_and(|n| {
            imp.issues.iter().any(|i| {
                i.number == n
                    && i.state == "open"
                    && matches!(state_of(&i.labels), Some(State::Planned | State::Ready))
            })
        });
    if !made && !open_early {
        return Ok(());
    }
    if let (false, Some(n)) = (imp.dry, slot.issue) {
        imp.gh.add_comment(
            n,
            &format!(
                "Done by PR #{pr}, merged into the default branch. Closed by `bosun plan import`."
            ),
        )?;
        if open_early {
            // By hand, not `set_state`: planned or ready -> merged is no transition of the state
            // machine; the work landed outside bosun.
            let labels = imp
                .issues
                .iter()
                .find(|i| i.number == n)
                .map(|i| i.labels.clone())
                .unwrap_or_default();
            imp.gh
                .ensure_label(&State::Merged.label(), "ededed", "bosun task state")?;
            for l in labels_to_remove(&labels, State::Merged) {
                imp.gh.remove_label(n, &l)?;
            }
            imp.gh.add_labels(n, &[&State::Merged.label()])?;
        }
        imp.gh.update_issue(
            n,
            IssuePatch {
                state: Some(PatchState::Closed),
                state_reason: Some("completed".into()),
                ..IssuePatch::default()
            },
        )?;
    }
    slot.merged = true;
    slot.closed = !imp.dry;
    slot.state = Some("merged".into());
    if let Some(kept) = imp.tasks.iter_mut().rev().find(|t| t.id == slot.id) {
        kept.merged = true;
        kept.closed = slot.closed;
        kept.state = slot.state.clone();
    }
    Ok(())
}

fn import_items(imp: &mut Importer, p: &Plan, a: &PlanImportArgs) -> Result<()> {
    let plan_path = imp.plan_path.clone();
    let mut added: Option<HashMap<String, (u64, u64)>> = None;
    // The plan file on the default branch, read once: an intake vouches only for what merged.
    let mut on_default: Option<Option<Plan>> = None;
    // F5: whether the plan file on the default branch is this very file, read once when needed.
    let mut same_file: Option<bool> = None;
    for item in p
        .items
        .iter()
        .filter(|i| a.item.is_empty() || a.item.contains(&i.id))
    {
        let gated = imp.gated.clone();
        let per_task = imp.per_task;
        let epic = imp.slot(&item.id, (&item.id, None), || {
            (
                format!("{}: {}", item.id, item.title),
                epic_body(item, &plan_path, &gated, per_task),
                labels("feature", &item.area, "planned"),
            )
        })?;
        let pid = format!("{}/P", item.id);
        // Only a new item can have been born of an intake; read the PRs once, when one shows up.
        if added.is_none() && !has_plan_task(&imp.issues, &pid) {
            let wanted: Vec<&str> = p
                .items
                .iter()
                .filter(|i| a.item.is_empty() || a.item.contains(&i.id))
                .filter(|i| !has_plan_task(&imp.issues, &format!("{}/P", i.id)))
                .map(|i| i.id.as_str())
                .collect();
            added = Some(intake_added(imp, &wanted)?);
        }
        let mut done_by = added.as_ref().and_then(|m| m.get(&item.id)).copied();
        if done_by.is_some() {
            if on_default.is_none() {
                on_default = Some(plan_on_default(imp)?);
            }
            let merged = on_default.as_ref().and_then(Option::as_ref);
            if !merged.is_some_and(|m| m.items.iter().any(|i| same_item(i, item))) {
                eprintln!(
                    "plan: {} differs from {} on the default branch; {} is not trusted as merged",
                    item.id, imp.plan_path, item.id
                );
                done_by = None;
            }
        }
        // F5: when the plan file on the default branch is byte-identical to this one, a plan task
        // has nothing left to commit: a new one is made merged and closed (like one an intake
        // vouches for) and an open `ready` one is closed. `--keep-plan-tasks-open` only lists
        // them in `on_default_open`. Only `plan import`: an amendment's new items need their
        // intake.
        let prior = lookup(&imp.issues, &pid, &item.id, Some("P"), &mut vec![]);
        let ready = prior.as_ref().is_some_and(|(i, adopted)| {
            !adopted && i.state == "open" && state_of(&i.labels) == Some(State::Ready)
        });
        let mut on_main = false;
        if done_by.is_none() && (prior.is_none() || ready) && !imp.amend {
            if same_file.is_none() {
                same_file = Some(same_file_on_default(imp));
            }
            on_main = same_file == Some(true);
            if on_main && a.keep_plan_tasks_open {
                imp.on_default_open.push(pid.clone());
                on_main = false;
            }
        }
        let pslot = imp.slot(&pid, (&item.id, Some("P")), || {
            let by = done_by
                .map(|(n, pr)| {
                    format!("Done by intake #{n}: this item is on the default branch since its PR #{pr} merged.\n\n")
                })
                .or_else(|| on_main.then(|| {
                    "Done: this item is on the default branch as planned; there is nothing to commit.\n\n".to_string()
                }))
                .unwrap_or_default();
            let intro = format!(
                "{by}Commit the plan file `{file}`. The UCs of {id} below land as passing tests together \
                 with the code of the tasks that implement them; there are no pending tests. \
                 The item's tasks become ready when this merges (run `bosun plan import` again); \
                 if the plan is already on the default branch, there is nothing to commit: \
                 close this issue as completed instead.\n\n{}\n",
                item.ucs
                    .iter()
                    .map(|u| format!("- {}: {}", u.id, u.text))
                    .collect::<Vec<_>>()
                    .join("\n"),
                id = item.id,
                file = plan_path
            );
            (
                format!("{pid}: plan"),
                task_body(&pid, &intro, &plan_task_meta(item, &plan_path, epic.issue)),
                labels(
                    "plan",
                    &item.area,
                    if done_by.is_some() || on_main { "merged" } else { "ready" },
                ),
            )
        })?;
        let mut pslot = pslot;
        let made = matches!(pslot.action, "created" | "would-create");
        let close_ready = on_main && ready && !pslot.merged;
        if close_ready && !imp.dry {
            let n = pslot.issue.expect("an existing issue has a number");
            let labels = prior
                .as_ref()
                .map(|(i, _)| i.labels.clone())
                .unwrap_or_default();
            imp.gh.add_comment(
                n,
                &format!(
                    "Done: {} on the default branch has {} as planned, so there is nothing to \
                     commit. Closed by `bosun plan import`.",
                    imp.plan_path, item.id
                ),
            )?;
            // By hand, not `set_state`: ready -> merged is no transition of the state machine;
            // the plan task is done without a PR, as when an intake vouches for it.
            imp.gh
                .ensure_label(&State::Merged.label(), "ededed", "bosun task state")?;
            for l in labels_to_remove(&labels, State::Merged) {
                imp.gh.remove_label(n, &l)?;
            }
            imp.gh.add_labels(n, &[&State::Merged.label()])?;
        }
        if on_main && (made || close_ready) {
            imp.on_default.push(pid.clone());
        }
        if (done_by.is_some() || on_main) && made || close_ready {
            // Made already merged: close it as completed so the tasks promote on this run.
            if let (false, Some(n)) = (imp.dry, pslot.issue) {
                imp.gh.update_issue(
                    n,
                    IssuePatch {
                        state: Some(PatchState::Closed),
                        state_reason: Some("completed".into()),
                        ..IssuePatch::default()
                    },
                )?;
            }
            pslot.merged = true;
            pslot.closed = !imp.dry;
            if close_ready && !imp.dry {
                pslot.state = Some("merged".into());
            }
            if let Some(kept) = imp.tasks.iter_mut().rev().find(|t| t.id == pslot.id) {
                kept.merged = true;
                kept.closed = pslot.closed;
                kept.state = pslot.state.clone();
            }
        }
        let mut slots = vec![];
        for t in &item.tasks {
            let tid = t.plan_id(&item.id);
            // Vetted only for a task that does not exist yet (the slot makes it).
            let existing = lookup(&imp.issues, &tid, &item.id, Some(&t.id), &mut vec![]);
            let known = existing.is_some();
            let allow = if known { vec![] } else { imp.allowed(&tid, t) };
            // `done-by: #N`: PR N, merged and linking the task, already did it. Only a new task
            // or an open planned or ready one can be finished this way.
            let open_early = existing.as_ref().is_some_and(|(i, _)| {
                i.state == "open"
                    && matches!(state_of(&i.labels), Some(State::Planned | State::Ready))
            });
            let done = match t.done_by {
                Some(pr) if !known || open_early => {
                    let number = existing.as_ref().map(|(i, _)| i.number);
                    match done_by_refusal(imp, &tid, number, pr)? {
                        None => Some(pr),
                        Some(why) => {
                            let msg = format!(
                                "{tid}: done-by #{pr} refused: {why}; merge the PR and link {tid} in it, or remove done-by"
                            );
                            imp.warnings.push(msg.clone());
                            imp.refused.push(msg);
                            None
                        }
                    }
                }
                _ => None,
            };
            let gated = imp.gated.clone();
            let state = if done.is_some() { "merged" } else { "planned" };
            let mut slot = imp.slot(&tid, (&item.id, Some(&t.id)), || {
                (
                    format!("{tid}: {}", t.title),
                    task_body(&tid, "", &task_meta_of(item, t, epic.issue, allow, &gated)),
                    labels(&t.kind, &item.area, state),
                )
            })?;
            if let Some(pr) = done {
                finish_done_by(imp, &mut slot, pr)?;
            }
            slots.push(slot);
        }
        if let (Some(session), false, Some(e)) = (imp.bind.clone(), imp.dry, epic.issue) {
            let plan_task = pslot.issue.map(|n| ("P".to_string(), n));
            let made: Vec<(String, u64)> = (plan_task.into_iter())
                .chain(
                    (item.tasks.iter().zip(&slots))
                        .filter_map(|(t, s)| s.issue.map(|n| (t.id.clone(), n))),
                )
                .collect();
            let plan = imp.plan_path.clone();
            super::budget::write_index(imp.gh, &session, &plan, &item.id, e)?;
            super::budget::write_binds(imp.gh, &session, &plan, item, e, &made)?;
        }
        imp.promote(&epic, &pslot, &slots)?;
    }
    Ok(())
}
