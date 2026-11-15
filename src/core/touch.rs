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
    /// Already merged: a pair of two merged tasks is history, not a conflict to warn about.
    pub merged: bool,
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

/// The crate roots (`""` for the repo root, else `dir/`) whose `src/` the globs touch while none
/// of the globs touches that crate's `tests/`. A glob that starts with `tests/` or `**` counts
/// for the root; `<dir>/tests/..` for `<dir>/`. Order kept, deduped.
pub fn src_without_tests(globs: &[String]) -> Vec<String> {
    let root = |g: &str, dir: &str| -> Option<String> {
        let at = if g.starts_with(dir) {
            Some(0)
        } else {
            g.find(&format!("/{dir}")).map(|i| i + 1)
        };
        at.map(|i| g[..i].to_string())
    };
    let mut srcs: Vec<String> = Vec::new();
    for g in globs {
        if let Some(r) = root(g, "src/")
            && !srcs.contains(&r)
        {
            srcs.push(r);
        }
    }
    srcs.retain(|r| {
        !globs.iter().any(|g| {
            root(g, "tests/").as_deref() == Some(r)
                || (r.is_empty() && g.starts_with("**"))
                || g.starts_with(&format!("{r}**"))
        })
    });
    srcs
}

/// A changed file as a PR lists it: its status and, for a rename, the old name.
#[derive(Debug, Clone, PartialEq)]
pub struct Changed {
    pub path: String,
    pub status: String,
    pub previous: Option<String>,
    /// A plan file whose patch only ticks checkboxes ([`plan_tick_patch`]): never outside a lease.
    pub tick_only: bool,
}

/// A markdown file of the plans: `plans/**` or `docs/superpowers/plans/**` at the repo root.
pub fn is_plan_file(path: &str) -> bool {
    let lower = path.to_ascii_lowercase();
    lower.ends_with(".md")
        && (path.starts_with("plans/") || path.starts_with("docs/superpowers/plans/"))
}

/// Whether `new` is `old` with task checkboxes ticked: `- [ ]` to `- [x]` (or `*`) on lines of
/// the same count, at least one tick, nothing else changed.
pub fn is_plan_tick(old: &str, new: &str) -> bool {
    ticks(old.lines(), new.lines())
}

/// Whether the patch of a plan file only ticks checkboxes. A GitHub patch has no file headers, so
/// every `-` and `+` line counts: each change block (the lines between two context lines) must
/// remove as many lines as it adds, each pair a tick. `None` (no patch shown) never counts.
pub fn plan_tick_patch(path: &str, patch: Option<&str>) -> bool {
    let Some(patch) = patch.filter(|_| is_plan_file(path)) else {
        return false;
    };
    let (mut old, mut new): (Vec<&str>, Vec<&str>) = (Vec::new(), Vec::new());
    let mut ticked = 0;
    // A block ends at a context line, a hunk header or the end of the patch.
    let mut close = |old: &mut Vec<&str>, new: &mut Vec<&str>| {
        let ok = old.len() == new.len() && old.iter().zip(&*new).all(|(o, n)| flip(o, n));
        ticked += old.len();
        old.clear();
        new.clear();
        ok
    };
    for l in patch.lines() {
        match l.as_bytes().first() {
            Some(b'-') => old.push(&l[1..]),
            Some(b'+') => new.push(&l[1..]),
            Some(b'\\') => {}
            _ => {
                if !close(&mut old, &mut new) {
                    return false;
                }
            }
        }
    }
    close(&mut old, &mut new) && ticked > 0
}

/// A plan file that was only modified (never renamed, added or removed) with a tick-only patch.
pub fn plan_tick_file(
    path: &str,
    status: &str,
    previous: Option<&str>,
    patch: Option<&str>,
) -> bool {
    status == "modified" && previous.is_none() && plan_tick_patch(path, patch)
}

fn flip(o: &str, n: &str) -> bool {
    let t = o.trim_start();
    let pad = &o[..o.len() - t.len()];
    ["- [ ]", "* [ ]"].iter().any(|m| {
        t.strip_prefix(m)
            .is_some_and(|rest| n == format!("{pad}{}[x]{rest}", &m[..2]))
    })
}

fn ticks<'a>(old: impl Iterator<Item = &'a str>, new: impl Iterator<Item = &'a str>) -> bool {
    let (old, new): (Vec<_>, Vec<_>) = (old.collect(), new.collect());
    old.len() == new.len()
        && old.iter().zip(&new).all(|(o, n)| o == n || flip(o, n))
        && old.iter().zip(&new).any(|(o, n)| o != n)
}

/// A per-PR fragment: `changelog.d/*.md`, `docs/progress.d/*.md` or a changeset
/// `.changeset/<name>.md` (not `README.md`, `config.json` or a subdirectory file).
pub fn is_fragment(path: &str) -> bool {
    let one_level = |dir: &str| {
        path.strip_prefix(dir)
            .is_some_and(|f| !f.contains('/') && f.ends_with(".md"))
    };
    one_level("changelog.d/")
        || one_level("docs/progress.d/")
        || (one_level(".changeset/") && !path.eq_ignore_ascii_case(".changeset/README.md"))
}

/// The paths of `changed` that no `touch` glob covers: files that are modified, removed or
/// renamed (a rename checks the old name only; its new path is effectively added). Added and
/// copied files (the source is unchanged) and the fragments always pass. Any other status is
/// checked, not trusted. Sorted, deduped.
pub fn outside(changed: &[Changed], touch: &[String]) -> Vec<String> {
    // A bad glob is dropped alone; the rest still count.
    let globs = parsed(&companions(touch));
    let ticked: Vec<&str> = (changed.iter())
        .filter(|c| c.tick_only)
        .map(|c| c.path.as_str())
        .collect();
    let covered =
        |p: &str| is_fragment(p) || ticked.contains(&p) || globs.iter().any(|g| g.matches(p));
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
            if reach[i].contains(&j)
                || reach[j].contains(&i)
                || (tasks[i].merged && tasks[j].merged)
            {
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
            .filter(|&i| !tasks[i].merged && globs[i].iter().any(|g| g.overlaps(&k)))
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
            merged: false,
        }
    }

    #[test]
    fn two_merged_tasks_do_not_overlap_but_a_merged_and_an_open_one_do() {
        let (mut a, mut b, c) = (
            task("M1/T1", &["src/x.rs"], &[]),
            task("M1/T2", &["src/x.rs"], &[]),
            task("M1/T3", &["src/x.rs"], &[]),
        );
        a.merged = true;
        b.merged = true;
        let none: HashMap<String, Vec<String>> = HashMap::new();
        assert!(
            parallel_overlaps(&[a.clone(), b.clone()], &none, 9)
                .pairs
                .is_empty()
        );
        assert_eq!(parallel_overlaps(&[a, b, c], &none, 9).pairs.len(), 2);
    }

    #[test]
    fn plan_ticks_are_free_and_only_ticks() {
        let p = "docs/superpowers/plans/a.md";
        let tick = "@@ -1,2 +1,2 @@\n-- [ ] **T1** x\n+- [x] **T1** x\n ctx\n";
        assert!(plan_tick_patch(p, Some(tick)));
        assert!(plan_tick_patch(
            "plans/a.md",
            Some("-  * [ ] a\n+  * [x] a")
        ));
        assert!(!plan_tick_patch("src/a.md", Some(tick)));
        assert!(!plan_tick_patch(p, None));
        assert!(!plan_tick_patch(p, Some("-- [ ] a\n+- [x] b")));
        assert!(!plan_tick_patch(p, Some("-- [x] a\n+- [ ] a")));
        assert!(!plan_tick_patch(p, Some("+- [ ] new task")));
        assert!(!plan_tick_patch(
            p,
            Some("-- [ ] a\n+- [x] a\n+- [x] extra")
        ));
        assert!(is_plan_tick("- [ ] a\nb", "- [x] a\nb"));
        assert!(!is_plan_tick("b", "b"));
        let t = |tick_only| Changed {
            path: p.into(),
            status: "modified".into(),
            previous: None,
            tick_only,
        };
        assert!(outside(&[t(true)], &["src/**".into()]).is_empty());
        assert_eq!(outside(&[t(false)], &["src/**".into()]), [p]);
    }

    #[test]
    fn a_tick_patch_has_no_header_lines_and_pairs_inside_one_hunk() {
        let p = "plans/a.md";
        // GitHub patches carry no `---`/`+++` file headers: such lines are content.
        assert!(!plan_tick_patch(
            p,
            Some("@@ -1 +1,2 @@\n-- [ ] T1\n+- [x] T1\n+++ any text")
        ));
        assert!(!plan_tick_patch(
            p,
            Some("@@ -1,2 +1 @@\n-- [ ] T1\n--- risk: high\n+- [x] T1")
        ));
        // Two ticks in two hunks still count; a removal in one hunk and an add in another do not.
        let two = "@@ -1 +1 @@\n-- [ ] a\n+- [x] a\n@@ -9 +9 @@\n-- [ ] b\n+- [x] b";
        assert!(plan_tick_patch(p, Some(two)));
        let split = "@@ -1 +1,2 @@\n+- [x] a\n x\n@@ -9,2 +10 @@\n-- [ ] a\n y";
        assert!(!plan_tick_patch(p, Some(split)));
        // A change block is paired at its own position: removed and added counts must match.
        let moved = "@@ -1,3 +1,3 @@\n-- [ ] a\n ctx\n-- [ ] b\n+- [x] a\n+- [x] b\n ctx";
        assert!(!plan_tick_patch(p, Some(moved)));
    }

    #[test]
    fn only_a_plain_modified_plan_file_is_tick_only() {
        let tick = Some("@@ -1 +1 @@\n-- [ ] a\n+- [x] a");
        assert!(plan_tick_file("plans/a.md", "modified", None, tick));
        assert!(!plan_tick_file("plans/a.md", "renamed", None, tick));
        assert!(!plan_tick_file("plans/a.md", "added", None, tick));
        assert!(!plan_tick_file(
            "plans/a.md",
            "modified",
            Some("src/a.rs"),
            tick
        ));
    }

    #[test]
    fn a_plan_file_lives_in_plans_or_docs_superpowers_plans_only() {
        assert!(is_plan_file("plans/a.md"));
        assert!(is_plan_file("docs/superpowers/plans/a.md"));
        assert!(is_plan_file("docs/superpowers/plans/sub/a.MD"));
        assert!(!is_plan_file("src/plans/a.md"));
        assert!(!is_plan_file("docs/plans/a.md"));
        assert!(!is_plan_file("x/docs/superpowers/plans/a.md"));
        assert!(!is_plan_file("plans/a.txt"));
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
            tick_only: false,
            path: path.into(),
            status: status.into(),
            previous: previous.map(String::from),
        };
        let changed = [
            c("src/a.rs", "modified", None),
            c("src/new.rs", "added", None),
            c("Cargo.lock", "modified", None),
            c("changelog.d/x.md", "modified", None),
            c(".changeset/x.md", "modified", None),
            c(".changeset/README.md", "modified", None),
            c(".changeset/config.json", "removed", None),
            c("src/b.rs", "renamed", Some("lib/old.rs")),
            c("lib/z.rs", "removed", None),
            c("lib/copy.rs", "copied", Some("lib/src.rs")),
        ];
        // The bad glob is dropped alone.
        let touch = s(&["src/a.rs", "src/b.rs", "Cargo.toml", "[bad"]);
        assert_eq!(
            outside(&changed, &touch),
            s(&[
                ".changeset/README.md",
                ".changeset/config.json",
                "lib/old.rs",
                "lib/z.rs"
            ])
        );
    }
}
