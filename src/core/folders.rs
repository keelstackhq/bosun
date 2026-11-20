//! Monorepo folders (spec §3 `[[folders]]`, §8 scoping, §9 per-folder health). Pure logic: which
//! folders a PR touches, which folders a red one blocks, and protected globs re-rooted under a
//! folder. No folders configured means one repo: everything here then changes nothing.

use std::collections::BTreeSet;

use crate::core::glob::Glob;

/// One `[[folders]]` entry: a repo-relative directory and the folders it builds on.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Folder {
    pub path: String,
    pub depends_on: Vec<String>,
}

/// The per-folder CI check on main (spec §9): `ci-<path>`, a `/` in the path becomes `-`.
pub fn check_name(path: &str) -> String {
    format!("ci-{}", path.replace('/', "-"))
}

/// Why `folders` is not a valid list: a path that is empty, absolute, has `.`/`..`, `*`, a
/// trailing `/` or is nested in another; a duplicate; a `depends_on` naming no folder or itself;
/// a dependency cycle. `None`: valid.
pub fn invalid(folders: &[Folder]) -> Option<String> {
    let paths: Vec<&str> = folders.iter().map(|f| f.path.as_str()).collect();
    for p in &paths {
        let bad_seg = p.split('/').any(|s| s.is_empty() || s == "." || s == "..");
        let top = p.split('/').next().unwrap_or_default().to_ascii_lowercase();
        let reserved = top.starts_with('.') || ["governance", "src", "npm"].contains(&top.as_str());
        if p.is_empty() || bad_seg || reserved || p.contains(['*', '?', '\\', '{', '[']) {
            return Some(format!(
                "folder path {p:?} is not a plain relative directory"
            ));
        }
    }
    let mut seen = BTreeSet::new();
    for p in &paths {
        if !seen.insert(*p) {
            return Some(format!("folder path {p:?} is listed twice"));
        }
    }
    for a in &paths {
        if let Some(b) = paths.iter().find(|b| b.starts_with(&format!("{a}/"))) {
            return Some(format!("folder {b:?} is inside folder {a:?}"));
        }
    }
    let checks: BTreeSet<String> = paths.iter().map(|p| check_name(p)).collect();
    if checks.len() != paths.len() {
        return Some("two folders map to the same ci-<folder> check name".into());
    }
    for f in folders {
        for d in &f.depends_on {
            if d == &f.path {
                return Some(format!("folder {:?} depends on itself", f.path));
            }
            if !paths.contains(&d.as_str()) {
                return Some(format!(
                    "folder {:?} depends on unknown folder {d:?}",
                    f.path
                ));
            }
        }
    }
    folders
        .iter()
        .find(|f| dependents(folders, &f.path).contains(&f.path))
        .map(|f| format!("folders have a dependency cycle through {:?}", f.path))
}

/// The folder a path lies in, if any.
pub fn folder_of<'a>(folders: &'a [Folder], path: &str) -> Option<&'a str> {
    let path = path.trim_start_matches("./");
    folders
        .iter()
        .map(|f| f.path.as_str())
        .find(|f| path.starts_with(&format!("{f}/")))
}

/// The folders a PR touches. A path outside every folder (root files such as `Cargo.toml`,
/// `Cargo.lock`, `rust-toolchain.toml`, `deny.toml`, `.github/**`) touches every folder, and so
/// does a PR with no files (fail closed).
pub fn touched<'a>(
    folders: &[Folder],
    paths: impl IntoIterator<Item = &'a str>,
) -> BTreeSet<String> {
    let all = || folders.iter().map(|f| f.path.clone()).collect();
    let mut out = BTreeSet::new();
    let mut any = false;
    for p in paths {
        any = true;
        match folder_of(folders, p) {
            Some(f) => {
                out.insert(f.to_string());
            }
            None => return all(),
        }
    }
    if any { out } else { all() }
}

/// The folders that depend on `path`, directly or transitively (not `path` itself, unless in a
/// cycle).
pub fn dependents(folders: &[Folder], path: &str) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    let mut todo = vec![path.to_string()];
    while let Some(p) = todo.pop() {
        for f in folders.iter().filter(|f| f.depends_on.contains(&p)) {
            if out.insert(f.path.clone()) {
                todo.push(f.path.clone());
            }
        }
    }
    out
}

/// The folders a set of red folders blocks: the red ones and every folder depending on one.
pub fn blocked(folders: &[Folder], red: &BTreeSet<String>) -> BTreeSet<String> {
    let mut out = red.clone();
    for r in red {
        out.extend(dependents(folders, r));
    }
    out
}

/// A conclusion of a `ci-<folder>` check run that keeps the folder green while main is red:
/// `success`, `neutral` or `skipped`. Anything else, a pending run and no run at all make the
/// folder red (fail closed: with no folder checks every folder is red, the one-repo behaviour).
pub fn is_green_conclusion(c: Option<&str>) -> bool {
    matches!(c, Some("success" | "neutral" | "skipped"))
}

/// The red folders while main is red: those whose `ci-<folder>` conclusion (by check name) is
/// not green.
pub fn red_folders<'a>(
    folders: &[Folder],
    conclusion: impl Fn(&str) -> Option<&'a str>,
) -> BTreeSet<String> {
    folders
        .iter()
        .filter(|f| !is_green_conclusion(conclusion(&check_name(&f.path))))
        .map(|f| f.path.clone())
        .collect()
}

/// `globs` re-rooted under `root` (`""` is the repo root: unchanged). A glob that already starts
/// with `**` matches at any depth, so it is kept as is.
pub fn under(root: &str, globs: &[&str]) -> Vec<String> {
    let root = root.trim_matches('/');
    globs
        .iter()
        .map(|g| {
            if root.is_empty() || g.starts_with("**") {
                g.to_string()
            } else {
                format!("{root}/{g}")
            }
        })
        .collect()
}

/// [`under`] for parsed globs; a re-rooted glob that does not parse is dropped (it cannot, as
/// the original parsed and the root is a plain directory).
pub fn under_globs(root: &str, globs: &[Glob]) -> Vec<Glob> {
    let texts: Vec<&str> = globs.iter().map(Glob::as_str).collect();
    under(root, &texts)
        .iter()
        .filter_map(|g| Glob::parse(g).ok())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn f(path: &str, deps: &[&str]) -> Folder {
        Folder {
            path: path.into(),
            depends_on: deps.iter().map(|s| s.to_string()).collect(),
        }
    }

    fn set(xs: &[&str]) -> BTreeSet<String> {
        xs.iter().map(|s| s.to_string()).collect()
    }

    fn mono() -> Vec<Folder> {
        vec![
            f("bosun", &[]),
            f("splice", &["bosun"]),
            f("hull", &["splice"]),
            f("hoist", &[]),
        ]
    }

    #[test]
    fn check_names() {
        assert_eq!(check_name("bosun"), "ci-bosun");
        assert_eq!(check_name("tools/x"), "ci-tools-x");
    }

    #[test]
    fn validation() {
        assert_eq!(invalid(&mono()), None);
        assert_eq!(invalid(&[]), None);
        for bad in [
            vec![f("", &[])],
            vec![f("/abs", &[])],
            vec![f("a/", &[])],
            vec![f("../x", &[])],
            vec![f("a/*", &[])],
            vec![f(".github", &[])],
            vec![f(".bosun", &[])],
            vec![f("Governance", &[])],
            vec![f("src", &[])],
            vec![f("a", &[]), f("a", &[])],
            vec![f("a", &[]), f("a/b", &[])],
            vec![f("a/b", &[]), f("a-b", &[])],
            vec![f("a", &["a"])],
            vec![f("a", &["zz"])],
            vec![f("a", &["b"]), f("b", &["a"])],
        ] {
            assert!(invalid(&bad).is_some(), "{bad:?}");
        }
    }

    #[test]
    fn touched_folders_and_root_files() {
        let m = mono();
        assert_eq!(touched(&m, ["bosun/src/lib.rs"]), set(&["bosun"]));
        assert_eq!(
            touched(&m, ["bosun/a", "./hull/b"]),
            set(&["bosun", "hull"])
        );
        let all = set(&["bosun", "hoist", "hull", "splice"]);
        for root in [
            "Cargo.toml",
            "Cargo.lock",
            "rust-toolchain.toml",
            "deny.toml",
            ".github/workflows/ci.yml",
            "README.md",
            "bosunx/a",
        ] {
            assert_eq!(touched(&m, ["hoist/a", root]), all, "{root}");
        }
        assert_eq!(touched(&m, []), all);
        // A folder name alone (not a file under it) is a root path.
        assert_eq!(touched(&m, ["bosun"]), all);
    }

    #[test]
    fn red_blocks_its_transitive_dependents() {
        let m = mono();
        assert_eq!(
            blocked(&m, &set(&["bosun"])),
            set(&["bosun", "hull", "splice"])
        );
        assert_eq!(blocked(&m, &set(&["splice"])), set(&["hull", "splice"]));
        assert_eq!(blocked(&m, &set(&["hull"])), set(&["hull"]));
        assert_eq!(blocked(&m, &set(&["hoist"])), set(&["hoist"]));
        assert!(blocked(&m, &BTreeSet::new()).is_empty());
    }

    #[test]
    fn globs_rerooted() {
        assert_eq!(under("", &["src/**"]), ["src/**"]);
        assert_eq!(
            under("bosun", &["src/**", "**/action.yml", "Cargo.toml"]),
            ["bosun/src/**", "**/action.yml", "bosun/Cargo.toml"]
        );
        let g = under_globs("bosun", &[Glob::parse(".bosun/**").unwrap()]);
        assert!(g[0].matches("bosun/.bosun/approvers.toml"));
    }

    #[test]
    fn red_folders_fail_closed() {
        let m = mono();
        let red = red_folders(&m, |c| match c {
            "ci-bosun" => Some("failure"),
            "ci-splice" => Some("success"),
            "ci-hull" => Some("skipped"),
            _ => None,
        });
        assert_eq!(red, set(&["bosun", "hoist"]));
        assert_eq!(red_folders(&m, |_| None).len(), 4);
        for c in ["failure", "cancelled", "timed_out", "action_required"] {
            assert!(!is_green_conclusion(Some(c)));
        }
        assert!(!is_green_conclusion(None));
    }
}
