//! Front-loaded human prerequisites (docs/prerequisites.md): the `## Prerequisites` section of a
//! plan and the tasks each prerequisite blocks. Pure.

use crate::core::plan::{Plan, PlanError};
use serde::Serialize;
use std::collections::HashMap;
use std::sync::LazyLock;

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Prereq {
    pub id: String,
    pub title: String,
    pub who: String,
    pub on: String,
    pub steps: String,
    pub verify: Verify,
    pub blocks: Vec<String>,
    pub line: usize,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum Verify {
    Owner,
    Secret { env: Option<String>, name: String },
    Variable { env: Option<String>, name: String },
    Environment { name: String },
    File { path: String },
}

fn re(p: &str) -> regex::Regex {
    regex::Regex::new(p).expect("static regex")
}
static ID: LazyLock<regex::Regex> = LazyLock::new(|| re(r"^H[1-9][0-9]{0,2}$"));
static LIST: LazyLock<regex::Regex> = LazyLock::new(|| re(r"^([-*+]|[0-9]+\.) "));
static BULLET: LazyLock<regex::Regex> = LazyLock::new(|| re(r"^- ([^:\s]+):\s*(\S.*)$"));
static KEY: LazyLock<regex::Regex> = LazyLock::new(|| re(r"^([a-z-]+):\s*(.*)$"));
static NAME: LazyLock<regex::Regex> = LazyLock::new(|| re(r"^[A-Za-z_][A-Za-z0-9_]*$"));
static ENV: LazyLock<regex::Regex> = LazyLock::new(|| re(r"^[A-Za-z0-9_.-]+$"));
static ENV_NAME: LazyLock<regex::Regex> =
    LazyLock::new(|| re(r"^[A-Za-z0-9_.-]+( [A-Za-z0-9_.-]+)*$"));

impl Verify {
    pub fn parse(v: &str) -> Result<Verify, String> {
        let v = v.trim();
        let (kind, arg) = v.split_once(char::is_whitespace).unwrap_or((v, ""));
        let arg = arg.trim();
        let scoped = |arg: &str| -> Result<(Option<String>, String), String> {
            let (env, name) = match arg.split_once('/') {
                Some((e, n)) => (Some(e), n),
                None => (None, arg),
            };
            if !NAME.is_match(name) || env.is_some_and(|e| !ENV.is_match(e)) {
                return Err(format!("`{arg}` is not NAME or ENV/NAME"));
            }
            if name.to_ascii_uppercase().starts_with("GITHUB_") {
                return Err(format!("`{name}`: GitHub names cannot start with GITHUB_"));
            }
            Ok((env.map(Into::into), name.into()))
        };
        match kind {
            "owner" if arg.is_empty() => Ok(Verify::Owner),
            "secret" => scoped(arg).map(|(env, name)| Verify::Secret { env, name }),
            "variable" => scoped(arg).map(|(env, name)| Verify::Variable { env, name }),
            "environment" if ENV_NAME.is_match(arg) => Ok(Verify::Environment { name: arg.into() }),
            "file"
                if arg.starts_with(['/', '~'])
                    || arg.contains('\\')
                    || arg.split('/').any(|c| c == "..") =>
            {
                Err(format!(
                    "file `{arg}` must be relative to the repo, without `..`"
                ))
            }
            "file" if !arg.is_empty() => Ok(Verify::File { path: arg.into() }),
            _ => Err(format!(
                "verify `{v}` is not owner, secret [ENV/]NAME, variable [ENV/]NAME, environment NAME or file PATH"
            )),
        }
    }

    pub fn describe(&self) -> String {
        let sc = |k: &str, env: &Option<String>, name: &String| match env {
            Some(e) => format!("{k} {e}/{name}"),
            None => format!("{k} {name}"),
        };
        match self {
            Verify::Owner => "owner".into(),
            Verify::Secret { env, name } => sc("secret", env, name),
            Verify::Variable { env, name } => sc("variable", env, name),
            Verify::Environment { name } => format!("environment {name}"),
            Verify::File { path } => format!("file {path}"),
        }
    }
}

macro_rules! bad {
    ($v:expr, $n:expr; $fix:expr => $($m:tt)*) => {
        $v.push(PlanError { line: $n, message: format!($($m)*), fix: $fix.into() })
    };
}

fn csv(v: &str) -> Vec<String> {
    let mut seen = std::collections::HashSet::new();
    let parts = v.split(',').map(str::trim);
    parts
        .filter(|s| !s.is_empty() && seen.insert(*s))
        .map(Into::into)
        .collect()
}

/// A bullet being read: its fields so far, each with its line.
struct Open {
    id: String,
    title: String,
    line: usize,
    kv: HashMap<String, (String, usize)>,
}

fn finish(o: Open, plan: &Plan, out: &mut Vec<Prereq>, errs: &mut Vec<PlanError>) {
    let id = o.id.clone();
    let get = |k: &str| o.kv.get(k).filter(|(v, _)| !v.is_empty());
    let at = |k: &str| get(k).map_or(o.line, |(_, l)| *l);
    let val = |k: &str| get(k).map_or("", |(v, _)| v.as_str());
    let on = val("on");
    if on.is_empty() {
        bad!(errs, o.line; "write `on: phone` or `on: computer`" => "{id} has no `on:`");
    } else if !matches!(on, "phone" | "computer") {
        bad!(errs, at("on"); "write `on: phone` or `on: computer`" => "{id}: `on:` is `{on}`, not phone or computer");
    }
    if val("steps").is_empty() {
        bad!(errs, o.line; "write the steps on one line, `steps: ...`" => "{id} has no `steps:`");
    }
    let blocks = csv(val("blocks"));
    if blocks.is_empty() {
        bad!(errs, o.line; "list what waits for it, e.g. `blocks: M1/T2, M2`" => "{id} has no `blocks:`");
    }
    for b in &blocks {
        let (item, task) = b
            .split_once('/')
            .map_or((b.as_str(), None), |(i, t)| (i, Some(t)));
        let found = plan.items.iter().find(|i| i.id == item);
        if found.is_none_or(|i| task.is_some_and(|t| !i.tasks.iter().any(|x| x.id == t))) {
            bad!(errs, at("blocks"); "name an item like M1 or a task like M1/T2" => "{id} blocks `{b}`, which is not an item or task of the plan");
        }
    }
    let verify = Verify::parse(get("verify").map_or("owner", |(v, _)| v)).unwrap_or_else(|e| {
        bad!(errs, at("verify"); "use owner, secret NAME, variable NAME, environment NAME or file PATH" => "{id}: {e}");
        Verify::Owner
    });
    out.push(Prereq {
        title: o.title.clone(),
        who: get("who").map_or("owner", |(v, _)| v).into(),
        on: on.into(),
        steps: val("steps").into(),
        verify,
        blocks,
        line: o.line,
        id,
    });
}

/// A `## ` heading that looks like a misspelt `## Prerequisites` (`## Pre-requisites`).
fn near_miss(line: &str) -> bool {
    let h = line[3..].trim().to_lowercase();
    h.starts_with("pre") && h.contains("requisite") && line != "## Prerequisites"
}

pub fn parse(text: &str, plan: &Plan) -> Result<Vec<Prereq>, Vec<PlanError>> {
    let (mut out, mut errs) = (Vec::new(), Vec::new());
    let (mut in_sec, mut seen, mut fence): (bool, bool, Option<&str>) = (false, false, None);
    let mut cur: Option<Open> = None;
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    for (i, raw) in text.lines().enumerate() {
        let (n, line) = (i + 1, raw.trim_end());
        let marker = ["```", "~~~"]
            .into_iter()
            .find(|m| line.trim_start().starts_with(m));
        if let Some(m) = marker.filter(|m| fence.is_none_or(|f| f == *m)) {
            fence = if fence.is_some() { None } else { Some(m) };
            continue;
        }
        if fence.is_some() || line.trim().is_empty() {
            continue;
        }
        let indented = line.starts_with([' ', '\t']);
        if !indented && let Some(o) = cur.take() {
            finish(o, plan, &mut out, &mut errs);
        }
        if line.starts_with("## ") {
            in_sec = line == "## Prerequisites";
            if near_miss(line) {
                bad!(errs, n; "write the heading as `## Prerequisites`" => "`{line}` is not `## Prerequisites`");
            } else if in_sec && std::mem::replace(&mut seen, true) {
                bad!(errs, n; "keep one `## Prerequisites` section" => "a second `## Prerequisites` section");
            }
            continue;
        }
        if !in_sec {
            continue;
        }
        if indented {
            let Some(o) = cur.as_mut() else {
                bad!(errs, n; "put the key line under a `- H1: title` bullet" => "indented line before any prerequisite");
                continue;
            };
            let Some(c) = KEY.captures(line.trim()) else {
                bad!(errs, n; "write fields as `  key: value`, each on one line" => "expected an indented `key: value` line");
                continue;
            };
            let k = c[1].to_string();
            if !["who", "on", "steps", "verify", "blocks"].contains(&k.as_str()) {
                bad!(errs, n; "use who, on, steps, verify or blocks" => "unknown prerequisite key `{k}`");
            } else if o.kv.insert(k.clone(), (c[2].trim().into(), n)).is_some() {
                bad!(errs, n; "keep one line per key" => "`{k}:` is given twice for {}", o.id);
            }
        } else if LIST.is_match(line) {
            match BULLET.captures(line) {
                Some(c) if ID.is_match(&c[1]) => {
                    if out.iter().any(|p| p.id == c[1]) {
                        bad!(errs, n; "give every prerequisite its own id" => "prerequisite id `{}` is used twice", &c[1]);
                    }
                    cur = Some(Open {
                        id: c[1].into(),
                        title: c[2].trim().into(),
                        line: n,
                        kv: HashMap::new(),
                    });
                }
                _ => {
                    bad!(errs, n; "write the bullet as `- H1: title` (id H1 to H999)" => "`{line}` is not `- H<n>: <title>`")
                }
            }
        }
    }
    if let Some(o) = cur.take() {
        finish(o, plan, &mut out, &mut errs);
    }
    if errs.is_empty() { Ok(out) } else { Err(errs) }
}

/// Ids of the prerequisites that block this task (listed as `<item>/<task>` or as `<item>`).
pub fn blocked_by<'a>(prereqs: &'a [Prereq], item: &str, task: &str) -> Vec<&'a str> {
    let full = format!("{item}/{task}");
    let hit = |b: &String| *b == full || b == item;
    prereqs
        .iter()
        .filter(|p| p.blocks.iter().any(hit))
        .map(|p| p.id.as_str())
        .collect()
}
