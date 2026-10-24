//! Plan-format parser (docs/plan-format.md, rules 1-8). Pure: text in, items out, every broken
//! rule reported with its line and a fix.

use serde::Serialize;
use std::collections::{HashMap, HashSet};
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
static HEADING: LazyLock<regex::Regex> = LazyLock::new(|| re(r"^## ([A-Z][A-Z0-9]*):\s*(\S.*)$"));
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

/// A `## ` heading is prose (ignored, and it ends the current item) only when its first word has
/// no colon, no digit and a lowercase letter after the first character (`## Notes`). Anything
/// else (`## m1: x`, `## M2 Two`, `## FAQ`) must be a valid `## <ID>: <title>` item heading.
fn is_prose(line: &str) -> bool {
    let w = line[3..].split_whitespace().next().unwrap_or("");
    !w.contains(':')
        && !w.contains(|c: char| c.is_ascii_digit())
        && w.chars().skip(1).any(char::is_lowercase)
}

fn csv(v: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for s in v.split(',').map(str::trim).filter(|s| !s.is_empty()) {
        if !out.iter().any(|o| o == s) {
            out.push(s.into());
        }
    }
    out
}

/// A cycle in a graph given as adjacency lists, as node indexes in order.
fn find_cycle(adj: &[Vec<usize>]) -> Option<Vec<usize>> {
    fn visit(
        n: usize,
        adj: &[Vec<usize>],
        st: &mut [u8],
        path: &mut Vec<usize>,
    ) -> Option<Vec<usize>> {
        st[n] = 1;
        path.push(n);
        for &m in &adj[n] {
            if st[m] == 1 {
                let at = path.iter().position(|&p| p == m).unwrap_or(0);
                return Some(path[at..].to_vec());
            }
            if st[m] == 0
                && let Some(c) = visit(m, adj, st, path)
            {
                return Some(c);
            }
        }
        path.pop();
        st[n] = 2;
        None
    }
    let mut st = vec![0u8; adj.len()];
    (0..adj.len()).find_map(|n| {
        if st[n] == 0 {
            visit(n, adj, &mut st, &mut Vec::new())
        } else {
            None
        }
    })
}

/// Longest chain (in nodes) starting at `n` of an acyclic graph.
fn longest(n: usize, adj: &[Vec<usize>], memo: &mut [usize]) -> usize {
    if memo[n] == 0 {
        memo[n] = 1 + adj[n]
            .iter()
            .map(|&m| longest(m, adj, memo))
            .max()
            .unwrap_or(0);
    }
    memo[n]
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
    let (mut sec, mut open, mut seen_item) = (Sec::Other, false, false);
    let mut fence: Option<(&str, usize)> = None;
    let mut desc: Vec<&str> = Vec::new();
    let (mut ifields, mut tkeys) = (Vec::<String>::new(), Vec::<String>::new());
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);

    for (i, raw) in text.lines().enumerate() {
        let n = i + 1;
        let line = raw.trim_end();
        let marker = ["```", "~~~"]
            .into_iter()
            .find(|m| line.trim_start().starts_with(m));
        if let Some(m) = marker.filter(|m| fence.is_none_or(|f| f.0 == *m)) {
            if fence.take().is_none() {
                fence = Some((m, n));
                if open && matches!(sec, Sec::Tasks | Sec::Done) {
                    bad!(errs, n; "move the code block into the description" => "code fence inside `### Tasks` or `### Done when`");
                }
            }
            if open && matches!(sec, Sec::Fields | Sec::Desc) {
                sec = Sec::Desc;
                desc.push(line);
            }
            continue;
        }
        if fence.is_some() {
            if open && sec == Sec::Desc {
                desc.push(line);
            }
            continue;
        }
        if let Some(t) = line.strip_prefix("# ") {
            title = title.or_else(|| Some(t.trim().to_string()));
            continue;
        }
        if line.starts_with("## ") {
            flush(&mut items, &mut desc, open);
            (seen_item, open, sec) = (true, false, Sec::Other);
            if is_prose(line) {
                continue;
            }
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
                ifields.clear();
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
                let known = key.as_ref().map(|c| c[1].to_string());
                if let Some(k) = known.filter(|k| k == "area" || k == "depends") {
                    if ifields.contains(&k) {
                        bad!(errs, n; "keep one line per field" => "`{k}:` is given twice");
                        continue;
                    }
                    ifields.push(k);
                }
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
                let Some(t) = it.tasks.last_mut() else {
                    bad!(errs, n; "put the key line under a `- T1: title` bullet" => "indented line before any task");
                    continue;
                };
                let Some(c) = KEY.captures(line.trim_start()) else {
                    bad!(errs, n; "write task fields as `  key: value`" => "expected an indented `key: value` line");
                    continue;
                };
                if tkeys.contains(&c[1].to_string()) {
                    bad!(errs, n; "keep one line per key" => "`{}:` is given twice for {}", &c[1], t.id);
                }
                tkeys.push(c[1].into());
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
                    tkeys.clear();
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
            Sec::Tasks => {
                bad!(errs, n; "write the task as `- T1: Title`" => "expected a `- T<n>: title` bullet, found `{line}`")
            }
            Sec::Done if line.starts_with("- ") => {
                let c = BULLET.captures(line);
                let Some((c, u)) = c.as_ref().and_then(|c| UC.captures(&c[1]).map(|u| (c, u)))
                else {
                    bad!(errs, n; "write it as `- UC-M1-1: a test can check this`" => "done-when bullet `{line}` is not `- UC-<item>-<n>: <text>`");
                    continue;
                };
                let want = format!("UC-{}-{}", it.id, it.ucs.len() + 1);
                if it.ucs.iter().any(|x| x.id == c[1]) {
                    bad!(errs, n; "give every UC its own number" => "`{}` is used twice", &c[1]);
                } else if u[1] != it.id {
                    bad!(errs, n; "use the id of the enclosing item" => "`{}` belongs to item {}, not {}", &c[1], &u[1], it.id);
                } else if c[1] != want {
                    bad!(errs, n; "number the UCs from 1 with no gaps or leading zeros" => "`{}` is out of order; expected `{want}`", &c[1]);
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
    if let Some((m, l)) = fence {
        bad!(errs, l; "close the code fence" => "unclosed code fence `{m}`");
    }
    if items.is_empty() && errs.is_empty() {
        bad!(errs, 1; "add `## M1: Title` items" => "the plan has no items");
    }
    if title.is_none() {
        bad!(errs, 1; "start the file with `# Plan: <name>`" => "the plan has no `# ` title");
    }

    let ids: HashMap<&str, usize> = items
        .iter()
        .enumerate()
        .map(|(i, x)| (x.id.as_str(), i))
        .collect();
    let all: HashSet<String> = items
        .iter()
        .flat_map(|i| i.tasks.iter().map(|t| t.plan_id(&i.id)))
        .collect();
    let mut item_adj = vec![Vec::new(); items.len()];
    for (k, it) in items.iter().enumerate() {
        let (l, id) = (it.line, &it.id);
        if it.area.is_empty() {
            bad!(errs, l; "add `area: <label>` right after the heading" => "{id} has no `area:` field");
        }
        for d in it.depends.iter().filter(|d| *d != "seed") {
            match ids.get(d.as_str()) {
                Some(&j) => item_adj[k].push(j),
                None => {
                    bad!(errs, l; "depend on an item id of this plan or `seed`" => "{id} depends on `{d}`, which is not an item")
                }
            }
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
    if let Some(c) = find_cycle(&item_adj) {
        let names: Vec<_> = c
            .iter()
            .chain(&c[..1])
            .map(|&i| items[i].id.as_str())
            .collect();
        bad!(errs, items[c[0]].line; "remove one of these `depends:` links" => "item dependency cycle: {}", names.join(" -> "));
    }

    let mut adjs: Vec<Vec<Vec<usize>>> = Vec::new();
    for it in &mut items {
        let own: HashMap<String, usize> = it
            .tasks
            .iter()
            .enumerate()
            .map(|(i, t)| (t.id.clone(), i))
            .collect();
        let mut adj = vec![Vec::new(); it.tasks.len()];
        for (k, t) in it.tasks.iter_mut().enumerate() {
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
                match (
                    all.contains(&full),
                    full.strip_prefix(&format!("{}/", it.id))
                        .and_then(|s| own.get(s)),
                ) {
                    (false, _) => {
                        bad!(errs, l; "use a task id of this item or `<item>/<task>` of an existing task" => "{me} depends on `{d}`, which is not a task")
                    }
                    (true, Some(&j)) => adj[k].push(j),
                    _ => {}
                }
                t.depends.push(full);
            }
        }
        adjs.push(adj);
    }
    // One cycle check over every `<item>/<task>` node, same-item and cross-item edges alike.
    let flat: Vec<(String, usize)> = items
        .iter()
        .flat_map(|i| i.tasks.iter().map(|t| (t.plan_id(&i.id), t.line)))
        .collect();
    let gi: HashMap<&str, usize> = flat
        .iter()
        .enumerate()
        .map(|(k, (id, _))| (id.as_str(), k))
        .collect();
    let gadj: Vec<Vec<usize>> = items
        .iter()
        .flat_map(|i| i.tasks.iter())
        .map(|t| {
            t.depends
                .iter()
                .filter_map(|d| gi.get(d.as_str()).copied())
                .collect()
        })
        .collect();
    if let Some(c) = find_cycle(&gadj) {
        let names: Vec<_> = c
            .iter()
            .chain(&c[..1])
            .map(|&i| flat[i].0.as_str())
            .collect();
        bad!(errs, flat[c[0]].1; "remove one of these `depends:` links" => "task dependency cycle: {}", names.join(" -> "));
    } else {
        // The critical path counts same-item chains only.
        for (it, adj) in items.iter().zip(&adjs) {
            let mut memo = vec![0; adj.len()];
            let deepest = (0..adj.len()).max_by_key(|&k| longest(k, adj, &mut memo));
            if let Some(d) = deepest.filter(|&d| memo[d] > 4) {
                bad!(errs, it.tasks[d].line; "cut the dependency chain to at most 4 tasks" => "critical path of {} is {} tasks, over 4", it.id, memo[d]);
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

/// The head of a `tests/uc_<item>.rs` of pending UC tests. Legacy: `bosun plan import` no longer
/// writes these files (#179, option b); kept for repos that still have them.
pub fn uc_file_header(item: &Item) -> String {
    format!(
        "//! Use cases of {} (spec \u{a7}12): {}.\n\
         //! Written by `bosun plan import`; the build task removes the ignore and makes each test pass.\n",
        item.id, item.title
    )
}

/// One pending test for a use case, preceded by a blank line.
pub fn uc_stub(id: &str, text: &str) -> String {
    let name = id.to_lowercase().replace('-', "_");
    format!(
        "\n/// {id}: {text}\n#[test]\n#[ignore = \"{id}\"]\nfn {name}() {{\n    todo!(\"{id}\")\n}}\n"
    )
}

/// Whether `id` (`UC-M2-3`) occurs in `text` as a whole id: `UC-M2-1` is not in `UC-M2-12`.
pub fn mentions_uc(text: &str, id: &str) -> bool {
    text.match_indices(id).any(|(i, _)| {
        !text[i + id.len()..]
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_alphanumeric() || c == '-')
    })
}

/// Whether `text` defines `fn uc_<item>_<n>` for the use case `id` (`UC-M2-3` -> `fn uc_m2_3`),
/// the name `uc_stub` gives (legacy, see `uc_file_header`).
pub fn defines_uc_fn(text: &str, id: &str) -> bool {
    let name = id.to_lowercase().replace('-', "_");
    let pat = format!("fn {name}");
    text.match_indices(&pat).any(|(i, _)| {
        text[i + pat.len()..]
            .chars()
            .next()
            .is_some_and(|c| c == '(' || c.is_whitespace() || c == '<')
    })
}

#[cfg(test)]
#[rustfmt::skip]
mod tests {
    use super::*;

    const OK: &str = concat!(
        "# Plan: x\n\nproject: x\n\n## M1: One\n\narea: core\ndepends: seed\n\nWords here.\n\n",
        "### Tasks\n\n- T1: First\n  touch: src/a.rs, tests/a.rs\n  size: 100\n",
        "- T2: Second\n  touch: src/b.rs\n  depends: T1\n  kind: flake\n\n",
        "### Done when\n\n- UC-M1-1: it works.\n"
    );
    #[test]
    fn uc_stub_text_and_whole_id_matching() {
        assert_eq!(uc_stub("UC-M2-3", "t."), "\n/// UC-M2-3: t.\n#[test]\n#[ignore = \"UC-M2-3\"]\nfn uc_m2_3() {\n    todo!(\"UC-M2-3\")\n}\n");
        assert!(mentions_uc("x UC-M2-1 y", "UC-M2-1"));
        assert!(mentions_uc("UC-M2-1", "UC-M2-1"));
        assert!(!mentions_uc("UC-M2-12", "UC-M2-1"));
        assert!(!mentions_uc("UC-M2-1-x", "UC-M2-1"));
        assert!(defines_uc_fn("fn uc_m2_1() {}", "UC-M2-1"));
        assert!(!defines_uc_fn("fn uc_m2_12() {}", "UC-M2-1"));
        assert!(mentions_uc("UC-M2-12 UC-M2-1.", "UC-M2-1"));
    }
    const H: &str = "not `## <ID>: <title>`";

    /// OK with the first `from` replaced by `to`.
    fn w(from: &str, to: &str) -> String {
        assert!(OK.contains(from), "{from}");
        OK.replacen(from, to, 1)
    }
    /// `text` must report an error on `line` containing `msg`; every error needs a fix.
    fn at(text: &str, line: usize, msg: &str) {
        let e = parse(text).unwrap_err();
        let hit = e.iter().any(|e| e.line == line && e.message.contains(msg));
        assert!(hit, "{msg} @{line}: {e:?}\n{text}");
        assert!(e.iter().all(|e| !e.fix.is_empty()));
    }
    /// Same, for OK with `from` replaced by `to`.
    fn chk(from: &str, to: &str, line: usize, msg: &str) {
        at(&w(from, to), line, msg);
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
        assert_eq!((&t[1].depends[0][..], t[1].plan_id("M1").as_str(), &i.ucs[0].id[..]), ("M1/T1", "M1/T2", "UC-M1-1"));
        assert_eq!(parse(&format!("```\n## bad\n```\n~~~\n## bad\n~~~\n{OK}")).unwrap().items.len(), 1);
    }

    #[test]
    fn rules_1_to_3() {
        at(&OK.replace("# Plan: x\n", ""), 1, "no `# ` title");
        at("# P\n", 1, "no items");
        chk("## M1: One", "## MILESTONE1: One", 5, "item id");
        for h in ["## M1:", "## m1: One", "## M2 Two", "## M-2: One"] {
            chk("## M1: One", h, 5, H);
        }
        at(&format!("{OK}\n## M1: Again\n"), 26, "used twice");
        chk("area: core\n", "", 5, "no `area:`");
        chk("area: core", "area: Core!", 7, "area");
        chk("depends: seed", "depends: M9", 5, "not an item");
        chk("depends: seed", "depends: seed\narea: b", 9, "`area:` is given twice");
        chk("area: core", "area: core\ndepends: seed", 9, "`depends:` is given twice");
    }

    #[test]
    fn prose_headings_are_ignored() {
        for h in ["## Notes", "## Open questions", "## Milestone one"] {
            assert_eq!(parse(&format!("{OK}\n{h}\n\n- not a task\n")).unwrap().items.len(), 1, "{h}");
        }
    }

    #[test]
    fn rule5_tasks() {
        for b in ["* T2: x", "+ T2: x", "1. T2: x", "plain text"] {
            chk("- T2: Second", b, 17, "expected a `- T<n>: title` bullet");
        }
        chk("- T2:", "- T1:", 17, "used twice");
        chk("- T2:", "- X2:", 17, "task bullet");
        chk("touch: src/b.rs", "touch: ", 17, "no `touch:`");
        chk("size: 100", "size: 401", 16, "over 400");
        chk("size: 100", "size: ten", 16, "not an integer");
        chk("kind: flake", "kind: odd", 20, "kind");
        chk("kind: flake", "risk: low", 20, "risk");
        chk("kind: flake", "colour: red", 20, "unknown task key");
        chk("depends: T1", "depends: T9", 17, "not a task");
        chk("depends: T1", "depends: M3/T1", 17, "not a task");
        chk("size: 100", "size: 100\n  touch: b", 17, "`touch:` is given twice");
        chk("  touch: src/a.rs, tests/a.rs\n", "  just text\n", 15, "`key: value`");
        chk("- T1: First\n", "  touch: a\n- T1: First\n", 14, "before any task");
        chk("- T2: Second\n", "```\n- T2: Second\n```\n", 17, "code fence inside");
        let hi = parse(&w("size: 100", "size: 101\n  risk: high")).unwrap();
        assert!(hi.warnings[0].message.contains("risk high") && !hi.warnings[0].fix.is_empty());
        assert_eq!(hi.items[0].tasks[0].risk, "high");
    }

    #[test]
    fn cycles_and_critical_path() {
        let m2 = OK.replace("M1", "M2").replace("depends: seed", "depends: M1").replace("# Plan: x\n\nproject: x\n", "");
        at(&format!("{}\n{m2}", w("depends: seed", "depends: M2")), 5, "cycle: M1 -> M2 -> M1");
        chk("size: 100", "size: 100\n  depends: T2", 14, "cycle: M1/T1 -> M1/T2 -> M1/T1");
        chk("size: 100", "size: 100\n  depends: T1", 14, "cycle: M1/T1 -> M1/T1");
        let two = |d1: &str, d2: &str| format!("# P\n## M1: A\narea: a\n### Tasks\n- T1: t\n  touch: a\n{d1}### Done when\n- UC-M1-1: ok\n## M2: B\narea: b\n### Tasks\n- T1: t\n  touch: a\n{d2}### Done when\n- UC-M2-1: ok\n");
        at(&two("  depends: M2/T1\n", "  depends: M1/T1\n"), 5, "cycle: M1/T1 -> M2/T1 -> M1/T1");
        at(&two("  depends: M2/T1\n  size: ten\n", "  depends: M1/T1\n"), 5, "cycle: M1/T1");
        at(&two("  depends: M2/T1\n  size: ten\n", "  depends: M1/T1\n"), 8, "not an integer");
        assert!(parse(&two("  depends: M2/T1\n", "")).is_ok());
        let diamond = "# P\n## M1: A\narea: a\n### Tasks\n- T1: t\n  touch: a\n- T2: t\n  touch: a\n  depends: T1\n- T3: t\n  touch: a\n  depends: T1\n- T4: t\n  touch: a\n  depends: T2, T3\n### Done when\n- UC-M1-1: ok\n";
        assert!(parse(diamond).is_ok());
        let chain = |n: usize| {
            let tasks: String = (1..=n).map(|k| format!("- T{k}: t\n  touch: a\n{}", if k > 1 { format!("  depends: T{}\n", k - 1) } else { String::new() })).collect();
            format!("# P\n## M1: One\narea: a\n### Tasks\n{tasks}### Done when\n- UC-M1-1: ok\n")
        };
        assert!(parse(&chain(4)).is_ok());
        at(&chain(5), 16, "critical path of M1 is 5");
    }

    #[test]
    fn rule7_ucs_and_sections() {
        chk("UC-M1-1", "UC-M2-1", 24, "belongs to item M2");
        chk("UC-M1-1", "UC-M1-2", 24, "expected `UC-M1-1`");
        chk("UC-M1-1", "UC-M1-01", 24, "expected `UC-M1-1`");
        at(&format!("{OK}- UC-M1-1: again\n"), 25, "`UC-M1-1` is used twice");
        at(&format!("{OK}~~~\n"), 25, "code fence inside");
        chk("### Done when\n\n- UC-M1-1: it works.\n", "", 5, "no UC");
        chk("### Tasks", "### Other", 5, "no `### Tasks`");
    }

    #[test]
    fn descriptions_keep_fences_and_late_keys() {
        let body = "Words here.\ndepends: on review\n```\n## X: y\n- T9: z\n```";
        let p = parse(&w("Words here.", body)).unwrap();
        assert_eq!(p.items[0].description, body);
        assert_eq!((p.items[0].depends.len(), p.items[0].tasks.len()), (1, 2));
        chk("Words here.", "```\nopen", 10, "unclosed code fence");
        chk("Words here.", "~~~\n```\n", 10, "unclosed");
    }

    #[test]
    fn tolerates_crlf_bom_tabs_trailing_space_and_duplicates() {
        let want = parse(OK).unwrap();
        assert_eq!(parse(&OK.replace('\n', "\r\n")).unwrap(), want);
        assert_eq!(parse(&format!("\u{feff}{OK}")).unwrap(), want);
        assert_eq!(parse(&OK.replace("\n  ", "\n\t")).unwrap(), want);
        assert_eq!(parse(&OK.replace('\n', "  \n")).unwrap().items[0].tasks, want.items[0].tasks);
        let dup = w("depends: T1", "depends: T1, T1").replace("depends: seed", "depends: seed, seed");
        let p = parse(&dup).unwrap();
        assert_eq!((p.items[0].depends.len(), p.items[0].tasks[1].depends.len()), (1, 1));
    }

    #[test]
    fn all_errors_are_collected_with_describe() {
        let bad = w("area: core\n", "").replace("UC-M1-1", "UC-M1-3").replace("touch: src/b.rs", "touch:");
        let e = parse(&bad).unwrap_err();
        assert!(e.len() >= 3 && e.windows(2).all(|w| w[0].line <= w[1].line));
        assert!(e[0].describe("plans/x.md").starts_with("plans/x.md:5: "));
    }
}
