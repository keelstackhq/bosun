//! PR shape of a plan item (docs/plan-format.md, "Choosing the PR shape"): one PR for the whole
//! item (`single`) or one PR per task (`split`). D-016: the shape follows the goal, never a line
//! cap; gated tasks always get their own single-task PR.

use serde::Serialize;
use std::collections::HashSet;
use std::fmt;

use crate::core::glob::{self, Glob};
use crate::core::plan::{Item, PlanError, Task};
use crate::core::risk::Rules;
use crate::core::touch::companions;

/// Planned size of a task without `size:`.
pub const DEFAULT_TASK_SIZE: u32 = 200;
/// D-016: a gated task is at most this many changed lines.
pub const MAX_GATED: u32 = 400;
/// Tasks side by side (tasks per task of the longest chain) needed for a split to pay off.
const SPLIT_WIDTH: f64 = 2.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Shape {
    Single,
    Split,
}

impl fmt::Display for Shape {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Shape::Single => "single",
            Shape::Split => "split",
        })
    }
}

impl Shape {
    pub fn parse(v: &str) -> Option<Shape> {
        match v {
            "single" => Some(Shape::Single),
            "split" => Some(Shape::Split),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Decision {
    pub shape: Shape,
    /// Planned changed lines of the item (drift signal only).
    pub total: u32,
    /// Tasks per task of the longest depends chain, over the non-gated tasks.
    pub width: f64,
    /// Plan ids of the gated tasks: each gets its own single-task PR.
    pub gated: Vec<String>,
    pub reasons: Vec<String>,
}

/// Every path glob of every risk rule: a task touching one is gated.
pub fn gated_globs(rules: &Rules) -> Vec<Glob> {
    rules.rules.iter().flat_map(|r| r.paths.clone()).collect()
}

/// Whether a touch list (with companions) reaches a gated path: a gated glob matches the touch
/// entry itself (`AGENTS.md`, `.bosun/limits.toml`, `.github/workflows/**`), or the entry overlaps
/// an anchored gated glob (`.github/**` ~ `.github/workflows/**`). A glob that matches anywhere
/// (`**/agents.md`) counts only by the first test, else every wide touch (`src/**`) would be
/// gated; risk-check on the PR still sees each real file.
pub fn touch_gated(touch: &[String], gated: &[Glob]) -> bool {
    let raw = companions(touch);
    raw.iter().any(|t| {
        // As risk-check, a path also matches in lower case (`AGENTS.md` ~ `**/agents.md`).
        let low = t.to_lowercase();
        let parsed = glob::Glob::parse(t).ok();
        gated.iter().any(|g| {
            g.matches(t)
                || g.matches(&low)
                || (!g.as_str().starts_with("**") && parsed.as_ref().is_some_and(|p| p.overlaps(g)))
        })
    })
}

/// [`touch_gated`], and every wildcard touch entry (`*`, `?`; companions included) is resolved
/// against `files`, the default branch's file list: gated when a file it matches is matched by a
/// gated glob too (raw or lower-case). A wildcard with no file list (`None`: unreadable or
/// truncated) cannot be decided and is an error, so callers fail closed.
pub fn touch_gated_in(
    touch: &[String],
    gated: &[Glob],
    files: Option<&[String]>,
) -> Result<bool, String> {
    if touch_gated(touch, gated) {
        return Ok(true);
    }
    for t in companions(touch)
        .iter()
        .filter(|t| t.contains('*') || t.contains('?'))
    {
        let Ok(parsed) = Glob::parse(t) else { continue };
        let Some(files) = files else {
            return Err(format!(
                "touch `{t}` cannot be checked for gated paths: the default branch's file list is not available (or truncated)"
            ));
        };
        let hit = files.iter().any(|f| {
            parsed.matches(f) && {
                let low = f.to_lowercase();
                gated.iter().any(|g| g.matches(f) || g.matches(&low))
            }
        });
        if hit {
            return Ok(true);
        }
    }
    Ok(false)
}

/// Plan ids (`<item>/<task>`) of the tasks gated only by resolving a wildcard touch against
/// `files` ([`touch_gated_in`]), and a plan error (at the task line) for each touch that cannot be
/// resolved. Tasks gated by [`is_gated`] are not listed: callers combine both.
pub fn resolve_gated(
    items: &[Item],
    gated: &[Glob],
    files: Option<&[String]>,
) -> (HashSet<String>, Vec<PlanError>) {
    let (mut set, mut errs) = (HashSet::new(), vec![]);
    for item in items {
        for t in item.tasks.iter().filter(|t| !is_gated(t, gated)) {
            match touch_gated_in(&t.touch, gated, files) {
                Ok(true) => {
                    set.insert(t.plan_id(&item.id));
                }
                Ok(false) => {}
                Err(message) => errs.push(PlanError {
                    line: t.line,
                    message,
                    fix: "narrow the touch to the paths the task edits, or mark it `risk: high`"
                        .into(),
                }),
            }
        }
    }
    (set, errs)
}

/// The gated rules of a plan import: risk-rule globs plus the tasks whose wildcard touch holds a
/// gated file ([`resolve_gated`]).
#[derive(Debug, Clone, Default)]
pub struct Gated {
    pub globs: Vec<Glob>,
    pub resolved: HashSet<String>,
}

impl Gated {
    pub fn is(&self, item_id: &str, t: &Task) -> bool {
        is_gated(t, &self.globs) || self.resolved.contains(&t.plan_id(item_id))
    }
}

/// Whether a task is gated, decided by the caller (item id, task).
pub type GatedBy<'a> = &'a dyn Fn(&str, &Task) -> bool;

/// A task is gated when it is `risk: high` or its touch list (with companions) overlaps a risk path.
pub fn is_gated(t: &Task, gated: &[Glob]) -> bool {
    t.risk == "high" || touch_gated(&t.touch, gated)
}

/// The plan error for a gated task planned over [`MAX_GATED`] lines (at its `size:` line).
pub fn gated_size_error(t: &Task, gated: &[Glob]) -> Option<PlanError> {
    gated_size_error_by(t, is_gated(t, gated))
}

fn gated_size_error_by(t: &Task, gated: bool) -> Option<PlanError> {
    let size = t.size.filter(|&s| s > MAX_GATED && gated)?;
    Some(PlanError {
        line: t.size_line,
        message: format!("size {size} is over {MAX_GATED} changed lines for a gated task"),
        fix: format!("split the gated work so each PR is at most {MAX_GATED} lines (D-016)"),
    })
}

/// [`gated_size_error`] for every task of `items`.
pub fn gated_size_errors(items: &[Item], gated: &[Glob]) -> Vec<PlanError> {
    gated_size_errors_by(items, &|_, t| is_gated(t, gated))
}

/// [`gated_size_errors`] with the caller's notion of gated.
pub fn gated_size_errors_by(items: &[Item], gated: GatedBy) -> Vec<PlanError> {
    (items
        .iter()
        .flat_map(|i| i.tasks.iter().map(move |t| (i, t))))
    .filter_map(|(i, t)| gated_size_error_by(t, gated(&i.id, t)))
    .collect()
}

pub fn total(item: &Item) -> u32 {
    item.tasks
        .iter()
        .map(|t| t.size.unwrap_or(DEFAULT_TASK_SIZE))
        .sum()
}

/// Tasks in the longest depends chain among `open` (other tasks are ignored; a cycle is reported
/// elsewhere and only bounds the loop here).
fn chain(item: &Item, open: &[&Task]) -> usize {
    let own = |d: &str| {
        let d = d.strip_prefix(&format!("{}/", item.id)).unwrap_or(d);
        open.iter().position(|t| t.id == d)
    };
    let mut len = vec![1usize; open.len()];
    for _ in 0..open.len() {
        for (k, t) in open.iter().enumerate() {
            let best = t.depends.iter().filter_map(|d| own(d)).map(|j| len[j]);
            len[k] = 1 + best.max().unwrap_or(0).min(open.len());
        }
    }
    len.into_iter().max().unwrap_or(1).min(open.len())
}

/// The first two tasks whose touch lists overlap, as globs.
fn coupled(open: &[&Task]) -> Option<(String, String)> {
    let parsed: Vec<_> = open
        .iter()
        .map(|t| glob::parse_all(&companions(&t.touch)).unwrap_or_default())
        .collect();
    (0..parsed.len()).find_map(|a| {
        (a + 1..parsed.len()).find_map(|b| glob::first_overlap(&parsed[a], &parsed[b]))
    })
}

/// The automatic recommendation for `item`. `gated` are the risk-rule path globs.
pub fn decide(item: &Item, gated: &[Glob]) -> Decision {
    decide_by(item, &|_, t| is_gated(t, gated))
}

/// [`decide`] with the caller's notion of gated.
pub fn decide_by(item: &Item, gated: GatedBy) -> Decision {
    let total = total(item);
    let (held, open): (Vec<&Task>, Vec<&Task>) =
        item.tasks.iter().partition(|t| gated(&item.id, t));
    let gated_ids: Vec<String> = held.iter().map(|t| t.plan_id(&item.id)).collect();
    let longest = chain(item, &open);
    let width = open.len() as f64 / longest.max(1) as f64;
    let mk = |shape, reasons: Vec<String>| Decision {
        shape,
        total,
        width,
        gated: gated_ids.clone(),
        reasons,
    };
    if open.len() <= 1 {
        let why = if held.is_empty() {
            "one task".to_string()
        } else {
            format!("{} gated tasks run alone", held.len())
        };
        return mk(Shape::Single, vec![why]);
    }
    let shared = coupled(&open);
    if width >= SPLIT_WIDTH && shared.is_none() && item.ucs.len() > 1 {
        let why = format!(
            "{} tasks run {width:.1} wide with no shared files",
            open.len()
        );
        return mk(Shape::Split, vec![why]);
    }
    let mut why = Vec::new();
    if let Some((a, b)) = shared {
        why.push(format!("tasks share files ({a} ~ {b})"));
    }
    if width < SPLIT_WIDTH {
        why.push(format!("tasks form a chain of {longest}: no parallel gain"));
    }
    if item.ucs.len() == 1 {
        why.push("one UC: the goal is testable only whole".into());
    }
    mk(Shape::Single, why)
}

/// The shape in force: the planner's choice, else the recommendation.
pub fn effective(item: &Item, gated: &[Glob]) -> Decision {
    effective_by(item, &|_, t| is_gated(t, gated))
}

/// [`effective`] with the caller's notion of gated.
pub fn effective_by(item: &Item, gated: GatedBy) -> Decision {
    let auto = decide_by(item, gated);
    match item.shape {
        None => auto,
        Some(shape) => Decision {
            shape,
            reasons: vec![format!("planner chose {shape}")],
            ..auto
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::plan::{Task, Uc};

    fn task(id: &str, touch: &str, deps: &[&str], size: u32, risk: &str) -> Task {
        Task {
            id: id.into(),
            touch: vec![touch.into()],
            depends: deps.iter().map(|d| d.to_string()).collect(),
            size: Some(size),
            risk: risk.into(),
            ..Task::default()
        }
    }
    fn item(tasks: Vec<Task>, ucs: usize) -> Item {
        let ucs = (0..ucs)
            .map(|n| Uc {
                id: format!("UC-M1-{n}"),
                text: String::new(),
                line: 0,
            })
            .collect();
        Item {
            id: "M1".into(),
            tasks,
            ucs,
            ..Item::default()
        }
    }
    fn wide(size: u32) -> Item {
        item(
            vec![
                task("T1", "a/**", &[], size, "normal"),
                task("T2", "b/**", &[], size, "normal"),
            ],
            2,
        )
    }
    fn globs(g: &[&str]) -> Vec<Glob> {
        g.iter().map(|g| Glob::parse(g).unwrap()).collect()
    }

    #[test]
    fn one_task_is_single_whatever_its_size() {
        let d = decide(&item(vec![task("T1", "a", &[], 9000, "normal")], 2), &[]);
        assert_eq!(d.shape, Shape::Single);
        assert_eq!(d.reasons, ["one task"]);
        assert!(d.gated.is_empty());
    }

    #[test]
    fn risk_high_tasks_are_gated_and_never_bundled() {
        let i = item(
            vec![
                task("T1", "a", &[], 50, "high"),
                task("T2", "b", &[], 50, "normal"),
            ],
            2,
        );
        let d = decide(&i, &[]);
        assert_eq!(d.gated, ["M1/T1"]);
        assert_eq!(d.shape, Shape::Single);
        assert_eq!(d.reasons, ["1 gated tasks run alone"]);
        let one = decide(&item(vec![task("T1", "a", &[], 900, "high")], 1), &[]);
        assert_eq!(
            (one.shape, one.gated),
            (Shape::Single, vec!["M1/T1".to_string()])
        );
    }

    #[test]
    fn a_risk_path_matches_in_lower_case_too() {
        let g = globs(&["**/agents.md"]);
        assert!(is_gated(&task("T1", "AGENTS.md", &[], 5, "normal"), &g));
        assert!(!is_gated(&task("T1", "src/a.rs", &[], 5, "normal"), &g));
    }

    #[test]
    fn a_wide_touch_is_gated_only_by_an_anchored_risk_path() {
        let g = globs(&["**/agents.md", ".github/workflows/**"]);
        // `src/**` could hold an agents.md, but that is risk-check's job on the real files.
        assert!(!touch_gated(&["src/**".into()], &g));
        assert!(touch_gated(&[".github/**".into()], &g));
        assert!(touch_gated(&[".github/workflows/ci.yml".into()], &g));
        assert!(touch_gated(&["docs/AGENTS.md".into()], &g));
    }

    #[test]
    fn a_touch_overlapping_a_risk_path_is_gated() {
        let g = globs(&[".github/workflows/**"]);
        let i = item(
            vec![
                task("T1", ".github/workflows/ci.yml", &[], 50, "normal"),
                task("T2", "a/**", &[], 50, "normal"),
                task("T3", "b/**", &[], 50, "normal"),
            ],
            2,
        );
        let d = decide(&i, &g);
        assert_eq!(d.gated, ["M1/T1"]);
        // The other two are independent and the item has two UCs: they split.
        assert_eq!(d.shape, Shape::Split);
        assert_eq!(d.reasons, ["2 tasks run 2.0 wide with no shared files"]);
    }

    #[test]
    fn size_never_decides() {
        let d = decide(&wide(5000), &[]);
        assert_eq!((d.shape, d.total), (Shape::Split, 10000));
        let d = decide(&wide(10), &[]);
        assert_eq!(d.shape, Shape::Split);
        assert_eq!(d.reasons, ["2 tasks run 2.0 wide with no shared files"]);
    }

    #[test]
    fn independent_tasks_of_a_one_uc_item_stay_single() {
        let mut i = wide(100);
        i.ucs.truncate(1);
        let d = decide(&i, &[]);
        assert_eq!(d.shape, Shape::Single);
        assert_eq!(d.reasons, ["one UC: the goal is testable only whole"]);
    }

    #[test]
    fn shared_files_stay_single() {
        let d = decide(
            &item(
                vec![
                    task("T1", "src/core/**", &[], 400, "normal"),
                    task("T2", "src/core/x.rs", &[], 400, "normal"),
                ],
                2,
            ),
            &[],
        );
        assert_eq!(d.shape, Shape::Single);
        assert_eq!(
            d.reasons,
            ["tasks share files (src/core/** ~ src/core/x.rs)"]
        );
    }

    #[test]
    fn chain_stays_single_and_one_uc_is_named() {
        let d = decide(
            &item(
                vec![
                    task("T1", "a/**", &[], 400, "normal"),
                    task("T2", "b/**", &["T1"], 400, "normal"),
                    task("T3", "c/**", &["M1/T2", "X1/T1"], 400, "normal"),
                ],
                1,
            ),
            &[],
        );
        assert_eq!(d.shape, Shape::Single);
        assert_eq!(
            d.reasons,
            [
                "tasks form a chain of 3: no parallel gain",
                "one UC: the goal is testable only whole"
            ]
        );
    }

    #[test]
    fn effective_honours_the_planner_and_single_is_never_an_error() {
        let mut i = wide(400);
        i.shape = Some(Shape::Single);
        let d = effective(&i, &[]);
        assert_eq!(
            (d.shape, d.reasons[0].as_str()),
            (Shape::Single, "planner chose single")
        );
        i.shape = Some(Shape::Split);
        assert_eq!(effective(&i, &[]).reasons, ["planner chose split"]);
        i.tasks[0].risk = "high".into();
        i.shape = Some(Shape::Single);
        let d = effective(&i, &[]);
        assert_eq!(
            (d.shape, d.gated),
            (Shape::Single, vec!["M1/T1".to_string()])
        );
        i.shape = None;
        assert_eq!(effective(&i, &[]).shape, Shape::Single);
    }

    fn real_globs() -> Vec<Glob> {
        gated_globs(&Rules::defaults())
    }
    fn files(f: &[&str]) -> Vec<String> {
        f.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn a_subtree_touch_holding_a_gated_file_is_gated() {
        let g = real_globs();
        for (touch, file) in [
            ("packages/**", "packages/a/hoist.toml"),
            ("crates/x/**", "crates/x/AGENTS.md"),
            ("web/**", "web/ui/action.yml"),
            ("web/**", "web/.npmrc"),
            ("pkg/**", "pkg/dev.sops.yaml"),
            ("pkg/**", "pkg/secrets.toml"),
        ] {
            let tree = files(&["src/a.rs", file]);
            assert_eq!(
                touch_gated_in(&[touch.to_string()], &g, Some(&tree)),
                Ok(true),
                "{touch} ~ {file}"
            );
        }
    }

    #[test]
    fn a_clean_subtree_is_not_gated_and_an_unknown_tree_is_an_error() {
        let g = real_globs();
        let t = vec!["packages/**".to_string()];
        let tree = files(&["packages/a/lib.rs", "hoist.toml"]);
        assert_eq!(touch_gated_in(&t, &g, Some(&tree)), Ok(false));
        let err = touch_gated_in(&t, &g, None).unwrap_err();
        assert!(err.contains("`packages/**` cannot be checked"), "{err}");
        // Explicit paths need no tree.
        assert_eq!(touch_gated_in(&["src/a.rs".into()], &g, None), Ok(false));
        assert_eq!(touch_gated_in(&["AGENTS.md".into()], &g, None), Ok(true));
    }

    #[test]
    fn resolve_gated_flags_subtree_tasks_and_reports_unresolvable_ones() {
        let g = real_globs();
        let i = item(
            vec![
                task("T1", "packages/**", &[], 50, "normal"),
                task("T2", "src/a.rs", &[], 50, "normal"),
            ],
            1,
        );
        let tree = files(&["packages/a/hoist.toml"]);
        let (set, errs) = resolve_gated(std::slice::from_ref(&i), &g, Some(&tree));
        assert!(errs.is_empty());
        assert_eq!(set.into_iter().collect::<Vec<_>>(), ["M1/T1"]);
        let (_, errs) = resolve_gated(&[i], &g, None);
        assert_eq!(errs.len(), 1);
        assert!(errs[0].fix.contains("risk: high"), "{}", errs[0].fix);
    }
}
