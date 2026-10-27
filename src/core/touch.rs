//! Touch lists across a plan: lockfile companions, parallel overlap and hotspots.

use super::glob::{self, Glob};
use std::collections::{HashMap, HashSet};

/// One task (`<item>/<task>` ids, touch with companions) as the overlap check sees it.
#[derive(Debug, Clone, PartialEq)]
pub struct TaskTouch {
    pub id: String,
    pub item: String,
    pub touch: Vec<String>,
    pub depends: Vec<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Overlap {
    pub a: String,
    pub b: String,
    pub glob_a: String,
    pub glob_b: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Hotspot {
    pub globs: Vec<String>,
    pub tasks: Vec<String>,
}

fn sibling(path: &str, name: &str) -> String {
    match path.rsplit_once('/') {
        Some((dir, _)) => format!("{dir}/{name}"),
        None => name.to_string(),
    }
}

fn is_file(path: &str, name: &str) -> bool {
    path == name || path.ends_with(&format!("/{name}"))
}

/// The input plus the lockfiles a manifest implies: `Cargo.toml` gives the `Cargo.lock` beside
/// it and at the root; `package.json` gives `pnpm-lock.yaml` the same way. Deduped, order kept.
pub fn companions(globs: &[String]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut add = |g: String| {
        if !out.contains(&g) {
            out.push(g);
        }
    };
    for g in globs {
        add(g.clone());
        for (manifest, lock) in [
            ("Cargo.toml", "Cargo.lock"),
            ("package.json", "pnpm-lock.yaml"),
        ] {
            if is_file(g, manifest) {
                add(sibling(g, lock));
                add(lock.to_string());
            }
        }
    }
    out
}

/// A changed file as a PR lists it: its status and, for a rename, the old name.
#[derive(Debug, Clone, PartialEq)]
pub struct Changed {
    pub path: String,
    pub status: String,
    pub previous: Option<String>,
}

/// Files a PR may always change: the per-branch fragments (CLAUDE.md), which never conflict.
const FRAGMENTS: [&str; 2] = ["changelog.d/*.md", "docs/progress.d/*.md"];

/// The paths of `changed` that no `touch` glob covers: files that are modified, removed or
/// renamed (a rename checks the old name only; its new path is effectively added). Added and
/// copied files (the source is unchanged) and the fragments always pass. Any other status is
/// checked, not trusted. Sorted, deduped.
pub fn outside(changed: &[Changed], touch: &[String]) -> Vec<String> {
    // A bad glob is dropped alone; the rest still count.
    let globs = parsed(&companions(touch));
    let free = parsed(&FRAGMENTS.map(String::from));
    let covered = |p: &str| globs.iter().chain(&free).any(|g| g.matches(p));
    let mut out: Vec<String> = Vec::new();
    for c in changed {
        // A rename's new path is effectively added: only the old name is edited.
        let p = match (c.status.as_str(), &c.previous) {
            ("added" | "copied", _) => continue,
            ("renamed", Some(old)) => old,
            _ => &c.path,
        };
        if !covered(p) && !out.contains(p) {
            out.push(p.clone());
        }
    }
    out.sort();
    out
}

/// `plan::parse` refuses invalid touch globs, so none is skipped for plan input.
fn parsed(globs: &[String]) -> Vec<Glob> {
    globs.iter().filter_map(|g| Glob::parse(g).ok()).collect()
}

/// For each task, the tasks it reaches through `depends`. `item_depends` maps an item to the
/// items it depends on (`seed` and unknown ids are ignored): each task of the item then depends
/// on every task of those items.
fn reach(tasks: &[TaskTouch], item_depends: &HashMap<String, Vec<String>>) -> Vec<HashSet<usize>> {
    let pos: HashMap<&str, usize> = tasks
        .iter()
        .enumerate()
        .map(|(i, t)| (t.id.as_str(), i))
        .collect();
    let adj: Vec<Vec<usize>> = tasks
        .iter()
        .map(|t| {
            let mut v: Vec<usize> = t
                .depends
                .iter()
                .filter_map(|d| pos.get(d.as_str()).copied())
                .collect();
            for item in item_depends.get(&t.item).into_iter().flatten() {
                v.extend((0..tasks.len()).filter(|&j| &tasks[j].item == item));
            }
            v
        })
        .collect();
    (0..tasks.len())
        .map(|s| {
            let (mut seen, mut stack) = (HashSet::new(), vec![s]);
            while let Some(n) = stack.pop() {
                for &m in &adj[n] {
                    if seen.insert(m) {
                        stack.push(m);
                    }
                }
            }
            seen
        })
        .collect()
}

/// Parallel overlaps: the first `keep` pairs, and how many more were found before the scan
/// stopped. `capped` is true when it stopped at `MAX_COUNTED` pairs, so `more` is a lower bound.
#[derive(Debug, Default, PartialEq)]
pub struct Overlaps {
    pub pairs: Vec<Overlap>,
    pub more: usize,
    pub capped: bool,
}

/// The scan stops once this many overlapping pairs are known.
pub const MAX_COUNTED: usize = 1000;

/// Pairs of tasks whose touch overlaps and where neither reaches the other through `depends`.
/// Order: by task position. Keeps the first `keep` pairs and counts further ones up to
/// `MAX_COUNTED` in all, then stops, so a huge plan does not pay for the full quadratic scan.
pub fn parallel_overlaps(
    tasks: &[TaskTouch],
    item_depends: &HashMap<String, Vec<String>>,
    keep: usize,
) -> Overlaps {
    let reach = reach(tasks, item_depends);
    let globs: Vec<Vec<Glob>> = tasks.iter().map(|t| parsed(&t.touch)).collect();
    let mut out = Overlaps::default();
    for i in 0..tasks.len() {
        for j in i + 1..tasks.len() {
            if reach[i].contains(&j) || reach[j].contains(&i) {
                continue;
            }
            let Some((ga, gb)) = glob::first_overlap(&globs[i], &globs[j]) else {
                continue;
            };
            if out.pairs.len() < keep {
                out.pairs.push(Overlap {
                    a: tasks[i].id.clone(),
                    b: tasks[j].id.clone(),
                    glob_a: ga,
                    glob_b: gb,
                });
            } else {
                out.more += 1;
            }
            if out.pairs.len() + out.more >= MAX_COUNTED {
                out.capped = true;
                return out;
            }
        }
    }
    out
}

/// Globs that hit at least `min` tasks, a glob hitting a task when it overlaps any glob of that
/// task. Only tasks that overlap each other pairwise count, and not when `depends` already puts
/// them all in one order. Globs with the same task set are merged, in first-seen order.
pub fn hotspots(
    tasks: &[TaskTouch],
    item_depends: &HashMap<String, Vec<String>>,
    min: usize,
) -> Vec<Hotspot> {
    let reach = reach(tasks, item_depends);
    let globs: Vec<Vec<Glob>> = tasks.iter().map(|t| parsed(&t.touch)).collect();
    let mut keys: Vec<&str> = Vec::new();
    for g in tasks.iter().flat_map(|t| &t.touch) {
        if !keys.contains(&g.as_str()) {
            keys.push(g);
        }
    }
    let mut out: Vec<Hotspot> = Vec::new();
    for key in keys {
        let Ok(k) = Glob::parse(key) else { continue };
        let hit: Vec<usize> = (0..tasks.len())
            .filter(|&i| globs[i].iter().any(|g| g.overlaps(&k)))
            .collect();
        let near: Vec<Vec<Glob>> = hit
            .iter()
            .map(|&i| {
                globs[i]
                    .iter()
                    .filter(|g| g.overlaps(&k))
                    .cloned()
                    .collect()
            })
            .collect();
        let pairs: Vec<(usize, usize)> = (0..hit.len())
            .flat_map(|a| (a + 1..hit.len()).map(move |b| (a, b)))
            .collect();
        let ordered = |&(a, b): &(usize, usize)| {
            reach[hit[a]].contains(&hit[b]) || reach[hit[b]].contains(&hit[a])
        };
        let touching = |&(a, b): &(usize, usize)| glob::first_overlap(&near[a], &near[b]).is_some();
        if hit.len() < min || !pairs.iter().all(touching) || pairs.iter().all(ordered) {
            continue;
        }
        let ids: Vec<String> = hit.iter().map(|&i| tasks[i].id.clone()).collect();
        match out.iter_mut().find(|h| h.tasks == ids) {
            Some(h) => h.globs.push(key.to_string()),
            None => out.push(Hotspot {
                globs: vec![key.to_string()],
                tasks: ids,
            }),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(v: &[&str]) -> Vec<String> {
        v.iter().map(|x| x.to_string()).collect()
    }

    fn task(id: &str, touch: &[&str], depends: &[&str]) -> TaskTouch {
        TaskTouch {
            id: id.into(),
            item: id.split('/').next().unwrap().into(),
            touch: companions(&s(touch)),
            depends: s(depends),
        }
    }

    #[test]
    fn companions_add_lockfiles_once() {
        let all = [
            "a/Cargo.toml",
            "Cargo.toml",
            "w/package.json",
            "x-Cargo.toml",
        ];
        assert_eq!(
            companions(&s(&all)).join(" "),
            "a/Cargo.toml a/Cargo.lock Cargo.lock Cargo.toml w/package.json \
w/pnpm-lock.yaml pnpm-lock.yaml x-Cargo.toml"
        );
    }

    #[test]
    fn the_overlap_scan_stops_at_the_cap() {
        let t: Vec<TaskTouch> = (0..60)
            .map(|k| task(&format!("M1/T{k}"), &["src/**"], &[]))
            .collect();
        let o = parallel_overlaps(&t, &HashMap::new(), 20);
        assert_eq!(
            (o.pairs.len(), o.more, o.capped),
            (20, MAX_COUNTED - 20, true)
        );
        let o = parallel_overlaps(&t[..7], &HashMap::new(), 20);
        assert_eq!((o.pairs.len(), o.more, o.capped), (20, 1, false));
    }

    #[test]
    fn hotspots_need_parallel_tasks_and_merge_globs() {
        let none = HashMap::new();
        let t = [
            task("M1/T1", &["src/lib.rs", "src/cli/mod.rs"], &[]),
            task("M1/T2", &["src/lib.rs", "src/cli/mod.rs", "a.rs"], &[]),
            task("M1/T3", &["src/*.rs", "src/cli/*.rs"], &[]),
            task("M1/T4", &["b.rs"], &[]),
        ];
        let h = hotspots(&t, &none, 3);
        assert_eq!(h.len(), 1);
        assert_eq!(h[0].globs.len(), 4);
        assert_eq!(h[0].tasks, s(&["M1/T1", "M1/T2", "M1/T3"]));
        let split = [
            task("M1/T1", &["src/**"], &[]),
            task("M1/T2", &["src/a.rs", "docs/x.md"], &[]),
            task("M1/T3", &["src/b.rs", "docs/x.md"], &[]),
        ];
        assert!(hotspots(&split, &none, 3).is_empty());
    }

    #[test]
    fn outside_checks_the_old_name_of_a_rename_and_spares_added_and_copied_files() {
        let c = |path: &str, status: &str, previous: Option<&str>| Changed {
            path: path.into(),
            status: status.into(),
            previous: previous.map(String::from),
        };
        let changed = [
            c("src/a.rs", "modified", None),
            c("src/new.rs", "added", None),
            c("Cargo.lock", "modified", None),
            c("changelog.d/x.md", "modified", None),
            c("src/b.rs", "renamed", Some("lib/old.rs")),
            c("lib/z.rs", "removed", None),
            c("lib/copy.rs", "copied", Some("lib/src.rs")),
        ];
        // The bad glob is dropped alone.
        let touch = s(&["src/a.rs", "src/b.rs", "Cargo.toml", "[bad"]);
        assert_eq!(outside(&changed, &touch), s(&["lib/old.rs", "lib/z.rs"]));
    }
}
