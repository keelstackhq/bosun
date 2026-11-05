//! PR shape of a plan item (docs/plan-format.md, "Choosing the PR shape"): one PR for the whole
//! item (`single`) or one PR per task (`split`). D-016: the shape follows the goal, never a line
//! cap; gated tasks always get their own single-task PR.

use serde::Serialize;
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

/// A task is gated when it is `risk: high` or its touch list (with companions) overlaps a risk path.
pub fn is_gated(t: &Task, gated: &[Glob]) -> bool {
    t.risk == "high" || touch_gated(&t.touch, gated)
}

/// The plan error for a gated task planned over [`MAX_GATED`] lines (at its `size:` line).
pub fn gated_size_error(t: &Task, gated: &[Glob]) -> Option<PlanError> {
    let size = t.size.filter(|&s| s > MAX_GATED && is_gated(t, gated))?;
    Some(PlanError {
        line: t.size_line,
        message: format!("size {size} is over {MAX_GATED} changed lines for a gated task"),
        fix: format!("split the gated work so each PR is at most {MAX_GATED} lines (D-016)"),
    })
}

/// [`gated_size_error`] for every task of `items`.
pub fn gated_size_errors(items: &[Item], gated: &[Glob]) -> Vec<PlanError> {
    (items.iter().flat_map(|i| &i.tasks))
        .filter_map(|t| gated_size_error(t, gated))
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
    let total = total(item);
    let (held, open): (Vec<&Task>, Vec<&Task>) =
        item.tasks.iter().partition(|t| is_gated(t, gated));
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
    let auto = decide(item, gated);
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
}
