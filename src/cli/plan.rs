//! `bosun plan check|import`: read a plan file (docs/plan-format.md) and, for import, make the
//! epics, plan tasks and tasks as GitHub issues. Issues are found by their hidden marker
//! `<!-- bosun:plan <id> -->` (or adopted by their metadata) and never changed.

use std::collections::HashSet;
use std::path::Path;

use serde_json::{Value, json};

use super::{PlanCheckArgs, PlanCmd, PlanImportArgs};
use crate::app::{App, Result, kill_switch, task_meta};
use crate::core::meta::TaskMeta;
use crate::core::plan::{self, Item, Plan, PlanError, Task};
use crate::core::state::state_of;
use crate::github::{Github, Issue, IssueState};
use crate::output::{BosunError, CmdResult, Outcome};

pub fn run(dir: Option<&Path>, cmd: &PlanCmd) -> CmdResult {
    match cmd {
        PlanCmd::Check(a) => check(dir, a),
        PlanCmd::Import(a) => import(dir, a),
    }
}

fn read_plan(dir: Option<&Path>, file: &str) -> Result<Plan> {
    let path = dir.map_or_else(|| file.into(), |d| d.join(file));
    let text = std::fs::read_to_string(&path).map_err(|e| {
        BosunError::error(
            format!("cannot read {file}: {e}"),
            "pass the path of a plan file, for example plans/bosun.md",
        )
    })?;
    plan::parse(&text).map_err(|errs| plan_errors(file, &errs))
}

fn plan_errors(file: &str, errs: &[PlanError]) -> BosunError {
    let first = &errs[0];
    BosunError::error(
        format!("{} errors in {file}", errs.len()),
        format!("{}; {}", first.describe(file), first.fix),
    )
    .with_data(json!({ "errors": errs }))
}

fn check(dir: Option<&Path>, a: &PlanCheckArgs) -> CmdResult {
    let p = read_plan(dir, &a.file)?;
    let items: Vec<Value> = p
        .items
        .iter()
        .map(|i| {
            json!({"id": i.id, "title": i.title, "area": i.area,
                "tasks": i.tasks.len(), "ucs": i.ucs.len()})
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

fn epic_item(body: &str) -> Option<String> {
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
    let mut found: Vec<&Issue> = issues
        .iter()
        .filter(|i| i.body_text().contains(&m))
        .collect();
    let adopted = found.is_empty();
    if adopted {
        found = issues
            .iter()
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
}

impl Importer<'_> {
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
        json!({
            "file": file,
            "dry_run": self.dry,
            "epics": self.epics.iter().map(|s| s.json(false)).collect::<Vec<_>>(),
            "tasks": self.tasks.iter().map(|s| s.json(true)).collect::<Vec<_>>(),
            "warnings": self.warnings,
        })
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

fn epic_body(item: &Item, file: &str) -> String {
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
    let meta = json!({"item": item.id, "area": item.area, "plan": file,
        "depends": item.depends, "ucs": ids});
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

fn plan_task_meta(item: &Item, epic: Option<u64>) -> TaskMeta {
    TaskMeta {
        item: item.id.clone(),
        task: "P".into(),
        epic,
        touch: vec![format!("tests/uc_{}.rs", item.id.to_lowercase())],
        depends: vec![],
        kind: "plan".into(),
        risk: "normal".into(),
        allow: vec!["skip".into()],
        capabilities: vec![],
    }
}

fn task_meta_of(item: &Item, t: &Task, epic: Option<u64>) -> TaskMeta {
    TaskMeta {
        item: item.id.clone(),
        task: t.id.clone(),
        epic,
        touch: t.touch.clone(),
        depends: t.depends.clone(),
        kind: t.kind.clone(),
        risk: t.risk.clone(),
        allow: vec![],
        capabilities: vec![],
    }
}

fn import(dir: Option<&Path>, a: &PlanImportArgs) -> CmdResult {
    let p = read_plan(dir, &a.file)?;
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
    let mut imp = Importer {
        gh: app.gh.as_ref(),
        issues: app.gh.list_issues(&[], IssueState::All)?,
        dry: a.dry_run,
        ensured: HashSet::new(),
        epics: vec![],
        tasks: vec![],
        warnings: vec![],
    };
    match import_items(&mut imp, &p, a) {
        Ok(()) => Ok(Outcome::data(imp.data(&a.file))),
        Err(e) => Err(e.with_data(imp.data(&a.file))),
    }
}

fn import_items(imp: &mut Importer, p: &Plan, a: &PlanImportArgs) -> Result<()> {
    for item in p
        .items
        .iter()
        .filter(|i| a.item.is_empty() || a.item.contains(&i.id))
    {
        let epic = imp.slot(&item.id, (&item.id, None), || {
            (
                format!("{}: {}", item.id, item.title),
                epic_body(item, &a.file),
                labels("feature", &item.area, "planned"),
            )
        })?;
        let pid = format!("{}/P", item.id);
        imp.slot(&pid, (&item.id, Some("P")), || {
            let intro = format!(
                "Write each UC of {id} as a pending test (`#[ignore = \"UC-..\"]`) in tests/uc_{low}.rs; \
                 the item's tasks become ready when this merges (run `bosun plan import` again).\n\n{}\n",
                item.ucs
                    .iter()
                    .map(|u| format!("- {}: {}", u.id, u.text))
                    .collect::<Vec<_>>()
                    .join("\n"),
                id = item.id,
                low = item.id.to_lowercase()
            );
            (
                format!("{pid}: pending UC tests"),
                task_body(&pid, &intro, &plan_task_meta(item, epic.issue)),
                labels("plan", &item.area, "ready"),
            )
        })?;
        for t in &item.tasks {
            let tid = t.plan_id(&item.id);
            imp.slot(&tid, (&item.id, Some(&t.id)), || {
                (
                    format!("{tid}: {}", t.title),
                    task_body(&tid, "", &task_meta_of(item, t, epic.issue)),
                    labels(&t.kind, &item.area, "planned"),
                )
            })?;
        }
    }
    Ok(())
}
