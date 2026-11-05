//! PR shape of a plan item (docs/plan-format.md, "Choosing the PR shape"): one PR for the whole
//! item (`single`) or one small PR per task (`split`).

use serde::Serialize;
use std::fmt;

use crate::core::glob;
use crate::core::plan::Item;
use crate::core::touch::companions;

/// Largest task of a split item.
pub const MAX_TASK: u32 = 400;
/// Largest single-PR item (D-013 bootstrap cap for low-risk PRs).
pub const MAX_SINGLE: u32 = 1500;
/// Planned size of a task without `size:`.
pub const DEFAULT_TASK_SIZE: u32 = 200;
/// A split pays off only above this many lines.
const SPLIT_ABOVE: u32 = 600;
/// ... and only when this many tasks can run side by side.
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
    /// The shape cannot be `single` whatever the planner writes.
    pub forced: bool,
    /// Planned changed lines of the item.
    pub total: u32,
    /// Tasks per task of the longest depends chain.
    pub width: f64,
    pub reasons: Vec<String>,
}

pub fn total(item: &Item) -> u32 {
    item.tasks
        .iter()
        .map(|t| t.size.unwrap_or(DEFAULT_TASK_SIZE))
        .sum()
}

/// Tasks in the longest depends chain inside the item (other items' tasks are ignored; a cycle
/// is reported elsewhere and only bounds the loop here).
fn chain(item: &Item) -> usize {
    let own = |d: &str| {
        let d = d.strip_prefix(&format!("{}/", item.id)).unwrap_or(d);
        item.tasks.iter().position(|t| t.id == d)
    };
    let mut len = vec![1usize; item.tasks.len()];
    for _ in 0..item.tasks.len() {
        for (k, t) in item.tasks.iter().enumerate() {
            let best = t.depends.iter().filter_map(|d| own(d)).map(|j| len[j]);
            len[k] = 1 + best.max().unwrap_or(0).min(item.tasks.len());
        }
    }
    len.into_iter().max().unwrap_or(1).min(item.tasks.len())
}

/// The first two tasks whose touch lists overlap, as `("M1/T1", "M1/T2")` globs.
fn coupled(item: &Item) -> Option<(String, String)> {
    let parsed: Vec<_> = item
        .tasks
        .iter()
        .map(|t| glob::parse_all(&companions(&t.touch)).unwrap_or_default())
        .collect();
    (0..parsed.len()).find_map(|a| {
        (a + 1..parsed.len()).find_map(|b| glob::first_overlap(&parsed[a], &parsed[b]))
    })
}

/// The automatic recommendation for `item`.
pub fn decide(item: &Item) -> Decision {
    let total = total(item);
    let width = item.tasks.len() as f64 / chain(item) as f64;
    let mk = |shape, forced, reasons: Vec<String>| Decision {
        shape,
        forced,
        total,
        width,
        reasons,
    };
    if let Some(t) = item.tasks.iter().find(|t| t.risk == "high") {
        let why = format!(
            "{} is risk: high: it needs its own small PR (D-013)",
            t.plan_id(&item.id)
        );
        return mk(Shape::Split, true, vec![why]);
    }
    if item.tasks.len() <= 1 {
        return mk(Shape::Single, false, vec!["one task".into()]);
    }
    if total > MAX_SINGLE {
        // A soft cap: `shape: single` may still keep the item whole (plan check warns).
        let why = format!("total {total} lines is over the {MAX_SINGLE}-line single-PR cap");
        return mk(Shape::Split, false, vec![why]);
    }
    let shared = coupled(item);
    if width >= SPLIT_WIDTH && total > SPLIT_ABOVE && shared.is_none() {
        let why = format!(
            "{} tasks run {width:.1} wide with no shared files and {total} lines",
            item.tasks.len()
        );
        return mk(Shape::Split, false, vec![why]);
    }
    let mut why = Vec::new();
    if let Some((a, b)) = shared {
        why.push(format!("tasks share files ({a} ~ {b})"));
    }
    if width < SPLIT_WIDTH {
        why.push(format!(
            "tasks form a chain of {}: no parallel gain",
            chain(item)
        ));
    }
    if total <= SPLIT_ABOVE {
        why.push(format!("{total} lines fit one review"));
    }
    if item.ucs.len() == 1 {
        why.push("one UC: the goal is testable only whole".into());
    }
    mk(Shape::Single, false, why)
}

/// The shape in force: the planner's choice, else the recommendation. `Err` carries the reason
/// when `single` is not allowed.
pub fn effective(item: &Item) -> Result<Decision, String> {
    let auto = decide(item);
    match item.shape {
        None => Ok(auto),
        Some(Shape::Split) => Ok(Decision {
            shape: Shape::Split,
            reasons: vec!["planner chose split".into()],
            ..auto
        }),
        Some(Shape::Single) if auto.forced => Err(auto.reasons.join("; ")),
        Some(Shape::Single) => Ok(Decision {
            shape: Shape::Single,
            reasons: vec!["planner chose single".into()],
            ..auto
        }),
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

    #[test]
    fn one_task_is_single() {
        let d = decide(&item(vec![task("T1", "a", &[], 900, "normal")], 2));
        assert_eq!((d.shape, d.forced), (Shape::Single, false));
        assert_eq!(d.reasons, ["one task"]);
    }

    #[test]
    fn risk_high_forces_split() {
        let d = decide(&item(
            vec![
                task("T1", "a", &[], 50, "high"),
                task("T2", "b", &[], 50, "normal"),
            ],
            2,
        ));
        assert_eq!((d.shape, d.forced), (Shape::Split, true));
        assert_eq!(
            d.reasons[0],
            "M1/T1 is risk: high: it needs its own small PR (D-013)"
        );
        // One risk: high task is split too, so its 400-line cap holds.
        let one = decide(&item(vec![task("T1", "a", &[], 900, "high")], 1));
        assert_eq!((one.shape, one.forced), (Shape::Split, true));
    }

    #[test]
    fn over_the_cap_recommends_split() {
        let d = decide(&wide(800));
        assert_eq!((d.shape, d.forced, d.total), (Shape::Split, false, 1600));
        assert!(d.reasons[0].contains("total 1600 lines is over the 1500-line"));
    }

    #[test]
    fn wide_and_big_without_shared_files_splits() {
        let d = decide(&wide(400));
        assert_eq!((d.shape, d.forced), (Shape::Split, false));
        assert_eq!(d.width, 2.0);
        assert_eq!(
            d.reasons,
            ["2 tasks run 2.0 wide with no shared files and 800 lines"]
        );
    }

    #[test]
    fn small_item_fits_one_review() {
        let d = decide(&wide(100));
        assert_eq!(d.shape, Shape::Single);
        assert_eq!(d.reasons, ["200 lines fit one review"]);
    }

    #[test]
    fn shared_files_stay_single() {
        let d = decide(&item(
            vec![
                task("T1", "src/core/**", &[], 400, "normal"),
                task("T2", "src/core/x.rs", &[], 400, "normal"),
            ],
            2,
        ));
        assert_eq!(d.shape, Shape::Single);
        assert_eq!(
            d.reasons,
            ["tasks share files (src/core/** ~ src/core/x.rs)"]
        );
    }

    #[test]
    fn chain_stays_single_and_one_uc_is_named() {
        let d = decide(&item(
            vec![
                task("T1", "a/**", &[], 400, "normal"),
                task("T2", "b/**", &["T1"], 400, "normal"),
                task("T3", "c/**", &["M1/T2", "X1/T1"], 400, "normal"),
            ],
            1,
        ));
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
    fn effective_honours_the_planner() {
        let mut i = wide(400);
        i.shape = Some(Shape::Single);
        let d = effective(&i).unwrap();
        assert_eq!(
            (d.shape, d.reasons[0].as_str()),
            (Shape::Single, "planner chose single")
        );
        i.shape = Some(Shape::Split);
        assert_eq!(effective(&i).unwrap().reasons, ["planner chose split"]);
        i.tasks[0].risk = "high".into();
        i.shape = Some(Shape::Single);
        assert!(effective(&i).unwrap_err().contains("risk: high"));
        i.shape = None;
        assert_eq!(effective(&i).unwrap().shape, Shape::Split);
    }
}
