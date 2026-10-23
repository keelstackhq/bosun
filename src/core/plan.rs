//! Plan-format parser (docs/plan-format.md, rules 1-8). Pure: text in, items out, every broken
//! rule reported with its line and a fix.

use serde::Serialize;
use std::collections::HashSet;
use std::sync::LazyLock;

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Plan {
    pub title: String,
    pub project: Option<String>,
    pub items: Vec<Item>,
    pub warnings: Vec<PlanError>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Default)]
pub struct Item {
    pub id: String,
    pub title: String,
    pub area: String,
    /// Item ids or "seed".
    pub depends: Vec<String>,
    pub description: String,
    pub tasks: Vec<Task>,
    pub ucs: Vec<Uc>,
    pub line: usize,
}

#[derive(Debug, Clone, PartialEq, Serialize, Default)]
pub struct Task {
    pub id: String,
    pub title: String,
    pub touch: Vec<String>,
    /// Normalised to `<item>/<task>`.
    pub depends: Vec<String>,
    pub size: Option<u32>,
    pub kind: String,
    pub risk: String,
    pub line: usize,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Uc {
    pub id: String,
    pub text: String,
    pub line: usize,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct PlanError {
    pub line: usize,
    pub message: String,
    pub fix: String,
}

impl Task {
    pub fn plan_id(&self, item: &str) -> String {
        format!("{item}/{}", self.id)
    }
}

impl PlanError {
    pub fn describe(&self, file: &str) -> String {
        format!("{file}:{}: {}", self.line, self.message)
    }
}

fn re(p: &str) -> regex::Regex {
    regex::Regex::new(p).expect("static regex")
}
static ITEM_ID: LazyLock<regex::Regex> = LazyLock::new(|| re(r"^[A-Z][A-Z0-9]{0,7}$"));
static TASK_ID: LazyLock<regex::Regex> = LazyLock::new(|| re(r"^T[0-9]{1,3}$"));
static AREA: LazyLock<regex::Regex> = LazyLock::new(|| re(r"^[a-z0-9-]+$"));
static HEADING: LazyLock<regex::Regex> = LazyLock::new(|| re(r"^## ([^:\s]*):\s*(\S.*)$"));
static BULLET: LazyLock<regex::Regex> = LazyLock::new(|| re(r"^- ([^:\s]+):\s*(\S.*)$"));
static UC: LazyLock<regex::Regex> = LazyLock::new(|| re(r"^UC-([A-Z0-9]+)-([0-9]+)$"));
static KEY: LazyLock<regex::Regex> = LazyLock::new(|| re(r"^([a-z-]+):\s*(.*)$"));

const KINDS: [&str; 5] = ["feature", "bookkeeping", "mainfix", "process-fix", "flake"];

#[derive(PartialEq, Clone, Copy)]
enum Sec {
    Fields,
    Desc,
    Tasks,
    Done,
    Other,
}

fn csv(v: &str) -> Vec<String> {
    v.split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect()
}

fn flush(items: &mut [Item], desc: &mut Vec<&str>, open: bool) {
    if let (true, Some(it)) = (open, items.last_mut())
        && it.description.is_empty()
    {
        it.description = desc.join("\n").trim().to_string();
    }
    desc.clear();
}

/// Push a `PlanError`: `bad!(list, line; "fix" => "message {x}", x)`.
macro_rules! bad {
    ($v:expr, $n:expr; $fix:expr => $($m:tt)*) => {
        $v.push(PlanError { line: $n, message: format!($($m)*), fix: $fix.into() })
    };
}

pub fn parse(text: &str) -> Result<Plan, Vec<PlanError>> {
    let (mut errs, mut warns): (Vec<PlanError>, Vec<PlanError>) = (vec![], vec![]);
    let (mut title, mut project) = (None, None);
    let mut items: Vec<Item> = Vec::new();
    let mut has_tasks: Vec<bool> = Vec::new();
    let (mut sec, mut open, mut fenced, mut seen_item) = (Sec::Other, false, false, false);
    let mut desc: Vec<&str> = Vec::new();

    for (i, raw) in text.lines().enumerate() {
        let n = i + 1;
        let line = raw.trim_end();
        if line.trim_start().starts_with("```") {
            fenced = !fenced;
            continue;
        }
        if fenced {
            continue;
        }
        if let Some(t) = line.strip_prefix("# ") {
            title = title.or_else(|| Some(t.trim().to_string()));
            continue;
        }
        if line.starts_with("## ") {
            flush(&mut items, &mut desc, open);
            (seen_item, open, sec) = (true, false, Sec::Other);
            let Some(c) = HEADING.captures(line) else {
                bad!(errs, n; "write the heading as `## M1: Title`" => "item heading `{line}` is not `## <ID>: <title>`");
                continue;
            };
            let id = c[1].to_string();
            if !ITEM_ID.is_match(&id) {
                bad!(errs, n; "use an id like M1 or S10" => "item id `{id}` does not match ^[A-Z][A-Z0-9]{{0,7}}$");
            } else if items.iter().any(|x| x.id == id) {
                bad!(errs, n; "give every item its own id" => "item id `{id}` is used twice");
            } else {
                let title = c[2].trim().into();
                items.push(Item {
                    id,
                    title,
                    line: n,
                    ..Item::default()
                });
                has_tasks.push(false);
                (open, sec) = (true, Sec::Fields);
            }
            continue;
        }
        if let (true, Some(h)) = (open, line.strip_prefix("### ")) {
            flush(&mut items, &mut desc, true);
            sec = match h.trim() {
                "Tasks" => Sec::Tasks,
                "Done when" => Sec::Done,
                _ => Sec::Other,
            };
            *has_tasks.last_mut().unwrap() |= sec == Sec::Tasks;
            continue;
        }
        if !seen_item {
            if let Some(p) = line.strip_prefix("project:") {
                project = Some(p.trim().to_string());
            }
            continue;
        }
        if !open {
            continue;
        }
        if line.trim().is_empty() {
            if sec == Sec::Desc {
                desc.push("");
            }
            continue;
        }
        let it = items.last_mut().unwrap();
        match sec {
            Sec::Fields | Sec::Desc => {
                let key = KEY.captures(line).filter(|_| sec == Sec::Fields);
                match key.as_ref().map(|c| (&c[1], c[2].trim())) {
                    Some(("area", v)) if AREA.is_match(v) => it.area = v.into(),
                    Some(("area", v)) => {
                        bad!(errs, n; "use lowercase letters, digits and `-`" => "area `{v}` does not match ^[a-z0-9-]+$")
                    }
                    Some(("depends", v)) => it.depends = csv(v),
                    _ => {
                        sec = Sec::Desc;
                        desc.push(line);
                    }
                }
            }
            Sec::Tasks if line.starts_with([' ', '\t']) => {
                let (Some(t), Some(c)) = (it.tasks.last_mut(), KEY.captures(line.trim_start()))
                else {
                    continue;
                };
                let v = c[2].trim();
                match &c[1] {
                    "touch" => t.touch = csv(v),
                    "depends" => t.depends = csv(v),
                    "size" => match v.parse::<u32>() {
                        Ok(s) if s > 400 => {
                            bad!(errs, n; "split the task so each PR is at most 400 lines" => "size {s} is over 400 changed lines")
                        }
                        Ok(s) => t.size = Some(s),
                        Err(_) => {
                            bad!(errs, n; "write the planned changed lines, e.g. `size: 250`" => "size `{v}` is not an integer")
                        }
                    },
                    "kind" if KINDS.contains(&v) => t.kind = v.into(),
                    "kind" => {
                        bad!(errs, n; "use one of feature, bookkeeping, mainfix, process-fix, flake" => "kind `{v}` is unknown")
                    }
                    "risk" if v == "high" => t.risk = v.into(),
                    "risk" => {
                        bad!(errs, n; "use `risk: high` or leave the line out" => "risk `{v}` is not valid")
                    }
                    k => {
                        bad!(errs, n; "use touch, depends, size, kind or risk" => "unknown task key `{k}`")
                    }
                }
            }
            Sec::Tasks if line.starts_with("- ") => match BULLET.captures(line) {
                Some(c) if TASK_ID.is_match(&c[1]) => {
                    if it.tasks.iter().any(|t| t.id == c[1]) {
                        bad!(errs, n; "give every task in an item its own id" => "task id `{}` is used twice in {}", &c[1], it.id);
                    }
                    let (id, title) = (c[1].into(), c[2].trim().into());
                    it.tasks.push(Task {
                        id,
                        title,
                        kind: "feature".into(),
                        risk: "normal".into(),
                        line: n,
                        ..Task::default()
                    });
                }
                _ => {
                    bad!(errs, n; "write the task as `- T1: Title` (id T1 to T999)" => "task bullet `{line}` is not `- T<n>: <title>`")
                }
            },
            Sec::Done if line.starts_with("- ") => {
                let c = BULLET.captures(line);
                let Some((c, u)) = c.as_ref().and_then(|c| UC.captures(&c[1]).map(|u| (c, u)))
                else {
                    bad!(errs, n; "write it as `- UC-M1-1: a test can check this`" => "done-when bullet `{line}` is not `- UC-<item>-<n>: <text>`");
                    continue;
                };
                if u[1] != it.id {
                    bad!(errs, n; "use the id of the enclosing item" => "`{}` belongs to item {}, not {}", &c[1], &u[1], it.id);
                } else if u[2].parse::<usize>().ok() != Some(it.ucs.len() + 1) {
                    bad!(errs, n; "number the UCs from 1 with no gaps" => "`{}` is out of order; expected number {}", &c[1], it.ucs.len() + 1);
                } else {
                    it.ucs.push(Uc {
                        id: c[1].into(),
                        text: c[2].trim().into(),
                        line: n,
                    });
                }
            }
            _ => {}
        }
    }
    flush(&mut items, &mut desc, open);
    if title.is_none() {
        bad!(errs, 1; "start the file with `# Plan: <name>`" => "the plan has no `# ` title");
    }

    let ids: HashSet<&str> = items.iter().map(|x| x.id.as_str()).collect();
    let all: HashSet<String> = items
        .iter()
        .flat_map(|i| i.tasks.iter().map(|t| t.plan_id(&i.id)))
        .collect();
    for (k, it) in items.iter().enumerate() {
        let (l, id) = (it.line, &it.id);
        if it.area.is_empty() {
            bad!(errs, l; "add `area: <label>` right after the heading" => "{id} has no `area:` field");
        }
        for d in it
            .depends
            .iter()
            .filter(|d| *d != "seed" && !ids.contains(d.as_str()))
        {
            bad!(errs, l; "depend on an item id of this plan or `seed`" => "{id} depends on `{d}`, which is not an item");
        }
        if it.tasks.is_empty() {
            let m = if has_tasks[k] {
                "has no tasks under `### Tasks`"
            } else {
                "has no `### Tasks` section"
            };
            bad!(errs, l; "add `### Tasks` with at least one `- T1: title` bullet" => "{id} {m}");
        }
        if it.ucs.is_empty() {
            bad!(errs, l; "add `### Done when` with `- UC-<item>-1: ...`" => "{id} has no UC under `### Done when`");
        }
    }

    for it in &mut items {
        for t in &mut it.tasks {
            let (me, l) = (t.plan_id(&it.id), t.line);
            if t.touch.is_empty() {
                bad!(errs, l; "add an indented `touch: path, path` line" => "{me} has no `touch:` paths");
            }
            if let (true, Some(s @ 101..)) = (t.risk == "high", t.size) {
                bad!(warns, l; "split the task or expect an owner review" => "{me} is risk high with size {s}, over 100");
            }
            for d in std::mem::take(&mut t.depends) {
                let full = if d.contains('/') {
                    d.clone()
                } else {
                    format!("{}/{d}", it.id)
                };
                if !all.contains(&full) {
                    bad!(errs, l; "use a task id of this item or `<item>/<task>` of an existing task" => "{me} depends on `{d}`, which is not a task");
                }
                t.depends.push(full);
            }
        }
    }
    errs.sort_by_key(|e| e.line);
    match title {
        Some(title) if errs.is_empty() => Ok(Plan {
            title,
            project,
            items,
            warnings: warns,
        }),
        _ => Err(errs),
    }
}

#[cfg(test)]
#[rustfmt::skip]
mod tests {
    use super::*;

    const OK: &str = "# Plan: x\n\nproject: x\n\n## M1: One\n\narea: core\ndepends: seed\n\nWords here.\n\n### Tasks\n\n- T1: First\n  touch: src/a.rs, tests/a.rs\n  size: 100\n- T2: Second\n  touch: src/b.rs\n  depends: T1\n  kind: flake\n\n### Done when\n\n- UC-M1-1: it works.\n";

    fn has(t: &str, line: usize, msg: &str) {
        let e = parse(t).unwrap_err();
        assert!(e.iter().any(|e| e.line == line && e.message.contains(msg)), "{msg} @{line}: {e:?}");
        assert!(e.iter().all(|e| !e.fix.is_empty()));
    }
    fn with(from: &str, to: &str) -> String {
        assert!(OK.contains(from), "{from}");
        OK.replacen(from, to, 1)
    }

    #[test]
    fn valid_plan() {
        let p = parse(OK).unwrap();
        assert_eq!((p.title.as_str(), p.project.as_deref()), ("Plan: x", Some("x")));
        let (i, t) = (&p.items[0], &p.items[0].tasks);
        assert_eq!((i.area.as_str(), i.description.as_str(), i.line), ("core", "Words here.", 5));
        assert_eq!(i.depends, ["seed"]);
        assert_eq!(t[0].touch, ["src/a.rs", "tests/a.rs"]);
        assert_eq!((t[0].kind.as_str(), t[0].risk.as_str(), t[0].size), ("feature", "normal", Some(100)));
        assert_eq!((t[1].depends.clone(), t[1].plan_id("M1")), (vec!["M1/T1".to_string()], "M1/T2".to_string()));
        assert_eq!(i.ucs[0].id, "UC-M1-1");
        assert_eq!(parse(&format!("```\n## bad\n```\n{OK}")).unwrap().items.len(), 1);
    }

    #[test]
    fn rules_1_to_3() {
        has(&OK.replace("# Plan: x\n", ""), 1, "no `# ` title");
        has(&with("## M1: One", "## m1: One"), 5, "item id");
        has(&with("## M1: One", "## Milestone one"), 5, "not `## <ID>: <title>`");
        has(&format!("{OK}\n## M1: Again\n"), 26, "used twice");
        has(&with("area: core\n", ""), 5, "no `area:`");
        has(&with("area: core", "area: Core!"), 7, "area");
        has(&with("depends: seed", "depends: M9"), 5, "not an item");
    }

    #[test]
    fn rule5_tasks() {
        has(&with("- T2:", "- T1:"), 17, "used twice");
        has(&with("- T2:", "- X2:"), 17, "task bullet");
        has(&with("touch: src/b.rs", "touch: "), 17, "no `touch:`");
        has(&with("size: 100", "size: 401"), 16, "over 400");
        has(&with("size: 100", "size: ten"), 16, "not an integer");
        has(&with("kind: flake", "kind: odd"), 20, "kind");
        has(&with("kind: flake", "risk: low"), 20, "risk");
        has(&with("kind: flake", "colour: red"), 20, "unknown task key");
        has(&with("depends: T1", "depends: T9"), 17, "not a task");
        has(&with("depends: T1", "depends: M3/T1"), 17, "not a task");
        let hi = parse(&with("size: 100", "size: 101\n  risk: high")).unwrap();
        assert!(hi.warnings[0].message.contains("risk high") && !hi.warnings[0].fix.is_empty());
        assert_eq!(hi.items[0].tasks[0].risk, "high");
    }

    #[test]
    fn rule7_ucs_and_sections() {
        has(&with("UC-M1-1", "UC-M2-1"), 24, "belongs to item M2");
        has(&with("UC-M1-1", "UC-M1-2"), 24, "expected number 1");
        has(&with("### Done when\n\n- UC-M1-1: it works.\n", ""), 5, "no UC");
        has(&with("### Tasks", "### Other"), 5, "no `### Tasks`");
    }

    #[test]
    fn all_errors_are_collected_with_describe() {
        let bad = with("area: core\n", "").replace("UC-M1-1", "UC-M1-3").replace("touch: src/b.rs", "touch:");
        let e = parse(&bad).unwrap_err();
        assert!(e.len() >= 3 && e.windows(2).all(|w| w[0].line <= w[1].line));
        assert!(e[0].describe("plans/x.md").starts_with("plans/x.md:5: "));
    }
}
