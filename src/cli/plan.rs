//! `bosun plan check|import`: read a plan file (docs/plan-format.md) and, for import, make the
//! epics, plan tasks and tasks as GitHub issues. Issues are found by their hidden marker
//! `<!-- bosun:plan <id> -->` (or adopted by their metadata) and never changed.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use super::{PlanAmendArgs, PlanCheckArgs, PlanCmd, PlanImportArgs};
use crate::app::{App, Result, kill_switch, notes, set_state, task_meta};
use crate::core::lease::leases_of;
use crate::core::meta::TaskMeta;
use crate::core::plan::{self, Item, Plan, PlanError, Task, Uc};
use crate::core::state::{State, state_of};
use crate::github::{Github, Issue, IssuePatch, IssueState, PatchState, Probe};
use crate::output::{BosunError, CmdResult, Outcome};
use crate::time::Timestamp;

pub fn run(dir: Option<&Path>, cmd: &PlanCmd) -> CmdResult {
    match cmd {
        PlanCmd::Check(a) => check(dir, a),
        PlanCmd::Import(a) => run_import(dir, a, false),
        PlanCmd::Amend(a) => amend(dir, a),
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
    dir: PathBuf,
    rust: bool,
    promoted: Vec<String>,
    stubs: Vec<Value>,
    /// The plan path as stored in epics: relative to the repo root.
    plan_path: String,
    roots: Vec<PathBuf>,
    now: Timestamp,
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
        let (promote, write) = if self.dry {
            ("would_promote", "would_write")
        } else {
            ("promoted", "stubs")
        };
        json!({
            "file": file,
            "dry_run": self.dry,
            "epics": self.epics.iter().map(|s| s.json(false)).collect::<Vec<_>>(),
            "tasks": self.tasks.iter().map(|s| s.json(true)).collect::<Vec<_>>(),
            promote: self.promoted,
            write: self.stubs,
            "warnings": self.warnings,
        })
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

    /// Write the UCs of `item` that no file under `tests/` mentions yet (rust profile only).
    fn stub(&mut self, item: &Item) -> Result<()> {
        let all = tests_text(&self.dir.join("tests"));
        let missing: Vec<&Uc> = item
            .ucs
            .iter()
            .filter(|u| !plan::mentions_uc(&all, &u.id) && !plan::defines_uc_fn(&all, &u.id))
            .collect();
        if missing.is_empty() {
            return Ok(());
        }
        let rel = format!("tests/uc_{}.rs", item.id.to_lowercase());
        if !self.dry {
            let path = self.dir.join(&rel);
            let mut text = match std::fs::read_to_string(&path) {
                Ok(t) if t.ends_with('\n') || t.is_empty() => t,
                Ok(t) => t + "\n",
                Err(_) => plan::uc_file_header(item),
            };
            for u in &missing {
                text.push_str(&plan::uc_stub(&u.id, &u.text));
            }
            let io = |e: std::io::Error| {
                BosunError::error(
                    format!("cannot write {rel}: {e}"),
                    "make `tests/` writable, then run `bosun plan import` again",
                )
            };
            std::fs::create_dir_all(path.parent().expect("has a parent")).map_err(io)?;
            std::fs::write(&path, text).map_err(io)?;
        }
        let ucs: Vec<&str> = missing.iter().map(|u| u.id.as_str()).collect();
        self.stubs.push(json!({"file": rel, "ucs": ucs}));
        Ok(())
    }
}

/// Every file under `dir`, concatenated; unreadable files are skipped.
fn tests_text(dir: &Path) -> String {
    let mut out = String::new();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return out;
    };
    for e in entries.flatten() {
        let path = e.path();
        if path.is_dir() {
            out.push_str(&tests_text(&path));
        } else if let Ok(t) = std::fs::read_to_string(&path) {
            out.push_str(&t);
            out.push('\n');
        }
    }
    out
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

/// `plan import` of every item, then the reconcile (UC-M2-4).
fn amend(dir: Option<&Path>, a: &PlanAmendArgs) -> CmdResult {
    let args = PlanImportArgs {
        file: a.file.clone(),
        dry_run: a.dry_run,
        item: vec![],
    };
    run_import(dir, &args, true)
}

fn run_import(dir: Option<&Path>, a: &PlanImportArgs, amend: bool) -> CmdResult {
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
    let base = std::fs::canonicalize(dir.unwrap_or_else(|| Path::new(".")))
        .unwrap_or_else(|_| dir.unwrap_or_else(|| Path::new(".")).to_path_buf());
    let mut roots = vec![app.ctx.dir.clone()];
    roots.extend(std::fs::canonicalize(&app.ctx.dir).ok());
    let mut imp = Importer {
        gh: app.gh.as_ref(),
        issues: all_issues(app.gh.as_ref())?,
        dry: a.dry_run,
        ensured: HashSet::new(),
        epics: vec![],
        tasks: vec![],
        warnings: vec![],
        rust: app.ctx.config.stack.iter().any(|s| s.profile == "rust"),
        dir: app.ctx.dir.clone(),
        promoted: vec![],
        stubs: vec![],
        plan_path: plan_key(&a.file, &base, &roots),
        roots,
        now: app.ctx.now,
    };
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
    Ok(Outcome::data(data))
}

/// The `plan` file named in the metadata of an epic body.
fn epic_plan(body: &str) -> Option<String> {
    let rest = &body[body.find("<!-- bosun:epic")? + "<!-- bosun:epic".len()..];
    let v: Value = serde_json::from_str(rest[..rest.find("-->")?].trim()).ok()?;
    v["plan"].as_str().map(str::to_string)
}

/// Replace the task metadata comment of `body`, keeping every other byte.
fn replace_task_meta(body: &str, meta: &TaskMeta) -> Option<String> {
    let start = body.find("<!-- bosun:task")?;
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
fn plan_key(file: &str, base: &Path, roots: &[PathBuf]) -> String {
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
    let n = notes(imp.gh, i.number)?;
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
                    issues.push(i);
                    found += 1;
                }
            }
            Probe::Gone => {}
            Probe::Missing => break,
        }
    }
    Ok(issues)
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
                    issues.push(i);
                    found += 1;
                }
            }
            Probe::Gone => {}
            Probe::Missing => break,
        }
    }
    Ok(issues)
}

fn import_items(imp: &mut Importer, p: &Plan, a: &PlanImportArgs) -> Result<()> {
    let mut imported = false;
    let plan_path = imp.plan_path.clone();
    for item in p
        .items
        .iter()
        .filter(|i| a.item.is_empty() || a.item.contains(&i.id))
    {
        imported = true;
        let epic = imp.slot(&item.id, (&item.id, None), || {
            (
                format!("{}: {}", item.id, item.title),
                epic_body(item, &plan_path),
                labels("feature", &item.area, "planned"),
            )
        })?;
        let pid = format!("{}/P", item.id);
        let pslot = imp.slot(&pid, (&item.id, Some("P")), || {
            let intro = format!(
                "Run `bosun plan import {file} --item {id}` to write each UC of {id} as a pending test \
                 (`#[ignore = \"UC-..\"]`) into tests/uc_{low}.rs, then commit that file; \
                 the item's tasks become ready when this merges (run `bosun plan import` again).\n\n{}\n",
                item.ucs
                    .iter()
                    .map(|u| format!("- {}: {}", u.id, u.text))
                    .collect::<Vec<_>>()
                    .join("\n"),
                id = item.id,
                file = plan_path,
                low = item.id.to_lowercase()
            );
            (
                format!("{pid}: pending UC tests"),
                task_body(&pid, &intro, &plan_task_meta(item, epic.issue)),
                labels("plan", &item.area, "ready"),
            )
        })?;
        let mut slots = vec![];
        for t in &item.tasks {
            let tid = t.plan_id(&item.id);
            slots.push(imp.slot(&tid, (&item.id, Some(&t.id)), || {
                (
                    format!("{tid}: {}", t.title),
                    task_body(&tid, "", &task_meta_of(item, t, epic.issue)),
                    labels(&t.kind, &item.area, "planned"),
                )
            })?);
        }
        imp.promote(&epic, &pslot, &slots)?;
        if imp.rust {
            imp.stub(item)?;
        }
    }
    if imported && !imp.rust {
        imp.warnings
            .push("UC stubs are only written for the rust profile".into());
    }
    Ok(())
}
