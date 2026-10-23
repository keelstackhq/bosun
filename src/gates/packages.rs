//! The packages adapters (`cargo-metadata`, `pnpm-workspace`): which workspace packages does a
//! diff touch?

use std::path::{Path, PathBuf};
use std::process::Command;

use serde::Deserialize;

use crate::output::BosunError;

#[derive(Debug, Clone, PartialEq)]
pub struct Package {
    pub name: String,
    /// Absolute directory of the package manifest.
    pub dir: PathBuf,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Affected {
    /// Nothing that belongs to a package changed.
    None,
    /// The named packages (workspace order).
    Some(Vec<String>),
    /// A workspace-wide input changed, or `--all`.
    All,
}

#[derive(Deserialize)]
struct Metadata {
    packages: Vec<RawPackage>,
}

#[derive(Deserialize)]
struct RawPackage {
    name: String,
    manifest_path: PathBuf,
}

/// The packages of the stack at `stack_dir`, found by the profile's packages adapter.
pub fn load(adapter: &str, stack_dir: &Path) -> Result<Vec<Package>, BosunError> {
    match adapter {
        "cargo-metadata" => load_cargo(stack_dir),
        "pnpm-workspace" => load_pnpm(stack_dir),
        other => Err(BosunError::error(
            format!("unknown packages adapter `{other}`"),
            "set `packages` in the profile to `cargo-metadata` or `pnpm-workspace`",
        )),
    }
}

/// The packages a diff touches, by the profile's packages adapter (see `affected_cargo`,
/// `affected_pnpm`).
pub fn affected(
    adapter: &str,
    stack_dir: &Path,
    changed: &[String],
    packages: &[Package],
) -> Affected {
    if adapter == "pnpm-workspace" {
        affected_pnpm(stack_dir, changed, packages)
    } else {
        affected_cargo(stack_dir, changed, packages)
    }
}

/// Run `cargo metadata --no-deps --format-version 1` in `stack_dir`.
fn load_cargo(stack_dir: &Path) -> Result<Vec<Package>, BosunError> {
    let out = Command::new("cargo")
        .args(["metadata", "--no-deps", "--format-version", "1"])
        .current_dir(stack_dir)
        .output()
        .map_err(|e| {
            BosunError::error(
                format!("cannot run cargo metadata: {e}"),
                "install the Rust toolchain (rustup) and make sure `cargo` is on PATH",
            )
        })?;
    if !out.status.success() {
        return Err(BosunError::error(
            format!(
                "cargo metadata failed in {}: {}",
                stack_dir.display(),
                String::from_utf8_lossy(&out.stderr).trim()
            ),
            "fix the Cargo.toml errors above, or set the stack path in .bosun/bosun.toml",
        ));
    }
    parse(&out.stdout)
}

pub fn parse(json: &[u8]) -> Result<Vec<Package>, BosunError> {
    let md: Metadata = serde_json::from_slice(json).map_err(|e| {
        BosunError::error(
            format!("cannot read the cargo metadata output: {e}"),
            "run `cargo metadata --no-deps --format-version 1` and fix what it reports",
        )
    })?;
    Ok(md
        .packages
        .into_iter()
        .map(|p| Package {
            dir: p
                .manifest_path
                .parent()
                .unwrap_or(Path::new(""))
                .to_path_buf(),
            name: p.name,
        })
        .collect())
}

/// Changed paths are relative to `stack_dir`. Each goes to the package with the longest
/// manifest-dir prefix. A change to the toolchain or workspace files, or to a Rust-relevant file
/// (`.rs`, `.toml`, `.lock`) outside every package, means all packages; other files (docs) are
/// not part of any package and are ignored.
pub fn affected_cargo(stack_dir: &Path, changed: &[String], packages: &[Package]) -> Affected {
    let mut hit: Vec<&str> = Vec::new();
    for rel in changed {
        let rel_path = Path::new(rel);
        if matches!(
            rel.as_str(),
            "Cargo.toml" | "Cargo.lock" | "rust-toolchain.toml"
        ) || rel_path.starts_with(".cargo")
        {
            return Affected::All;
        }
        let abs = stack_dir.join(rel_path);
        let owner = packages
            .iter()
            .filter(|p| abs.starts_with(&p.dir))
            .max_by_key(|p| p.dir.components().count());
        match owner {
            Some(p) => {
                if !hit.contains(&p.name.as_str()) {
                    hit.push(&p.name);
                }
            }
            None => {
                let ext = rel_path.extension().and_then(|e| e.to_str());
                if matches!(ext, Some("rs" | "toml" | "lock")) {
                    return Affected::All;
                }
            }
        }
    }
    if hit.is_empty() {
        return Affected::None;
    }
    Affected::Some(
        packages
            .iter()
            .filter(|p| hit.contains(&p.name.as_str()))
            .map(|p| p.name.clone())
            .collect(),
    )
}

/// The `packages:` globs of a `pnpm-workspace.yaml` (a line-based reader: the block list under
/// `packages:`, quotes and comments dropped). `!glob` entries are kept for the caller to exclude.
pub fn parse_workspace_globs(yaml: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut in_list = false;
    for raw in yaml.lines() {
        let line = raw.split(" #").next().unwrap_or("");
        let line = if line.trim_start().starts_with('#') {
            ""
        } else {
            line
        };
        if line.trim().is_empty() {
            continue;
        }
        if !line.starts_with([' ', '\t', '-']) {
            in_list = line.trim_end() == "packages:";
            continue;
        }
        if let (true, Some(item)) = (in_list, line.trim().strip_prefix('-')) {
            let item = item.trim().trim_matches(['"', '\'']);
            if !item.is_empty() {
                out.push(item.to_string());
            }
        }
    }
    out
}

/// Keep the directories (relative, `/`-separated) that match an include glob and no `!` glob.
pub fn select_dirs(dirs: &[String], globs: &[String]) -> Vec<String> {
    let parse = |g: &str| crate::core::glob::Glob::parse(g.trim_end_matches('/')).ok();
    let inc: Vec<_> = globs
        .iter()
        .filter(|g| !g.starts_with('!'))
        .filter_map(|g| parse(g))
        .collect();
    let exc: Vec<_> = globs
        .iter()
        .filter_map(|g| g.strip_prefix('!'))
        .filter_map(parse)
        .collect();
    dirs.iter()
        .filter(|d| inc.iter().any(|g| g.matches(d)) && !exc.iter().any(|g| g.matches(d)))
        .cloned()
        .collect()
}

/// Relative paths of every directory under `dir` that holds a `package.json` (the root itself is
/// not listed; `node_modules` and dot directories are skipped).
fn package_dirs(dir: &Path, rel: &str, out: &mut Vec<String>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let mut subs: Vec<_> = entries.flatten().filter(|e| e.path().is_dir()).collect();
    subs.sort_by_key(|e| e.file_name());
    for e in subs {
        let name = e.file_name().to_string_lossy().into_owned();
        if name == "node_modules" || name.starts_with('.') {
            continue;
        }
        let sub = if rel.is_empty() {
            name
        } else {
            format!("{rel}/{name}")
        };
        if e.path().join("package.json").is_file() {
            out.push(sub.clone());
        }
        package_dirs(&e.path(), &sub, out);
    }
}

/// Read `pnpm-workspace.yaml` in `stack_dir`, expand its globs to directories with a
/// `package.json`, and read each package's `name`.
fn load_pnpm(stack_dir: &Path) -> Result<Vec<Package>, BosunError> {
    let file = stack_dir.join("pnpm-workspace.yaml");
    let yaml = std::fs::read_to_string(&file).map_err(|e| {
        BosunError::error(
            format!("cannot read {}: {e}", file.display()),
            "add a pnpm-workspace.yaml with a `packages:` list at the stack path",
        )
    })?;
    let mut dirs = Vec::new();
    package_dirs(stack_dir, "", &mut dirs);
    let mut out = Vec::new();
    for rel in select_dirs(&dirs, &parse_workspace_globs(&yaml)) {
        let manifest = stack_dir.join(&rel).join("package.json");
        let name = std::fs::read_to_string(&manifest)
            .ok()
            .and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok())
            .and_then(|v| v["name"].as_str().map(String::from))
            .ok_or_else(|| {
                BosunError::error(
                    format!("{} has no readable `name`", manifest.display()),
                    "give the package a `name` in its package.json",
                )
            })?;
        out.push(Package {
            name,
            dir: stack_dir.join(&rel),
        });
    }
    Ok(out)
}

/// Like `affected_cargo`: each changed path goes to the package with the longest dir prefix. A
/// root workspace or toolchain file, or a TypeScript-relevant file (`.ts`, `.tsx`, `.js`, `.json`,
/// `.yaml`, ...) in no package, means all packages; other files (docs) affect none.
pub fn affected_pnpm(stack_dir: &Path, changed: &[String], packages: &[Package]) -> Affected {
    let mut hit: Vec<&str> = Vec::new();
    for rel in changed {
        let rel_path = Path::new(rel);
        if matches!(
            rel.as_str(),
            "package.json" | "pnpm-lock.yaml" | "pnpm-workspace.yaml" | ".npmrc" | ".nvmrc"
        ) {
            return Affected::All;
        }
        let abs = stack_dir.join(rel_path);
        let owner = packages
            .iter()
            .filter(|p| abs.starts_with(&p.dir))
            .max_by_key(|p| p.dir.components().count());
        match owner {
            Some(p) => {
                if !hit.contains(&p.name.as_str()) {
                    hit.push(&p.name);
                }
            }
            None => {
                let ext = rel_path.extension().and_then(|e| e.to_str());
                if matches!(
                    ext,
                    Some("ts" | "tsx" | "js" | "jsx" | "mjs" | "cjs" | "json" | "yaml" | "yml")
                ) {
                    return Affected::All;
                }
            }
        }
    }
    if hit.is_empty() {
        return Affected::None;
    }
    Affected::Some(
        packages
            .iter()
            .filter(|p| hit.contains(&p.name.as_str()))
            .map(|p| p.name.clone())
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pkgs() -> Vec<Package> {
        vec![
            Package {
                name: "a".into(),
                dir: "/r/a".into(),
            },
            Package {
                name: "b".into(),
                dir: "/r/b".into(),
            },
            Package {
                name: "b-inner".into(),
                dir: "/r/b/inner".into(),
            },
        ]
    }

    fn aff(files: &[&str]) -> Affected {
        let files: Vec<String> = files.iter().map(|s| s.to_string()).collect();
        affected_cargo(Path::new("/r"), &files, &pkgs())
    }

    #[test]
    fn longest_prefix_wins() {
        assert_eq!(
            aff(&["b/inner/src/x.rs"]),
            Affected::Some(vec!["b-inner".into()])
        );
        assert_eq!(
            aff(&["b/src/x.rs", "a/x.rs", "b/y.rs"]),
            Affected::Some(vec!["a".into(), "b".into()])
        );
    }

    #[test]
    fn workspace_inputs_mean_all() {
        for f in [
            "Cargo.toml",
            "Cargo.lock",
            "rust-toolchain.toml",
            ".cargo/config.toml",
            "build.rs",
        ] {
            assert_eq!(aff(&[f]), Affected::All, "{f}");
        }
    }

    #[test]
    fn docs_are_ignored() {
        assert_eq!(aff(&["docs/x.md", "README.md"]), Affected::None);
    }

    #[test]
    fn parses_metadata() {
        let json = br#"{"packages":[{"name":"a","manifest_path":"/r/a/Cargo.toml"}]}"#;
        assert_eq!(
            parse(json).unwrap(),
            vec![Package {
                name: "a".into(),
                dir: "/r/a".into()
            }]
        );
    }

    #[test]
    fn workspace_globs_parse() {
        let y = "# c\npackages:\n  - 'packages/*'\n  - \"apps/**\" # x\n  - '!**/test/**'\ncatalog:\n  - nope\n";
        assert_eq!(
            parse_workspace_globs(y),
            ["packages/*", "apps/**", "!**/test/**"]
        );
        assert_eq!(parse_workspace_globs("packages:\n- a\n- b\n"), ["a", "b"]);
        assert!(parse_workspace_globs("name: x\n").is_empty());
    }

    #[test]
    fn globs_select_dirs() {
        let dirs: Vec<String> = ["packages/a", "packages/b", "packages/b/test", "tools/x"]
            .map(String::from)
            .to_vec();
        let g: Vec<String> = ["packages/*", "tools/*", "!tools/x"]
            .map(String::from)
            .to_vec();
        assert_eq!(select_dirs(&dirs, &g), ["packages/a", "packages/b"]);
    }

    #[test]
    fn pnpm_affected_follows_the_cargo_rules() {
        let pk = vec![
            Package {
                name: "a".into(),
                dir: "/r/packages/a".into(),
            },
            Package {
                name: "b".into(),
                dir: "/r/packages/b".into(),
            },
        ];
        let aff = |f: &[&str]| {
            let f: Vec<String> = f.iter().map(|s| s.to_string()).collect();
            affected_pnpm(Path::new("/r"), &f, &pk)
        };
        assert_eq!(
            aff(&["packages/a/src/x.ts"]),
            Affected::Some(vec!["a".into()])
        );
        for f in [
            "package.json",
            "pnpm-lock.yaml",
            "tsconfig.base.json",
            "eslint.config.js",
        ] {
            assert_eq!(aff(&[f]), Affected::All, "{f}");
        }
        assert_eq!(aff(&["docs/x.md", "README.md"]), Affected::None);
    }
}
