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
        // The generic adapter: the stack has no packages, so a per-package gate runs once.
        "none" | "generic" => Ok(Vec::new()),
        other => Err(BosunError::error(
            format!("unknown packages adapter `{other}`"),
            "set `packages` in the profile to a known adapter (`cargo-metadata`, `pnpm-workspace`, `none`)",
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
    match adapter {
        "cargo-metadata" => affected_cargo(stack_dir, changed, packages),
        "pnpm-workspace" => affected_pnpm(stack_dir, changed, packages),
        _ => Affected::All,
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

fn workspace_error(message: String, fix: &str) -> BosunError {
    BosunError::error(message, fix)
}

/// The text of a YAML line without its `#` comment.
fn yaml_code(line: &str) -> &str {
    match line.trim_start().starts_with('#') {
        true => "",
        false => line.split(" #").next().unwrap_or(""),
    }
}

fn unquote(item: &str) -> &str {
    item.trim().trim_matches(['"', '\''])
}

/// The `packages:` globs of a `pnpm-workspace.yaml` (a line-based reader). Both forms work: the
/// block list under `packages:` and the flow list `packages: [a, 'b', "c"]` on one line. Quotes
/// and comments are dropped; `!glob` entries are kept for the caller to exclude. Any other value
/// after `packages:` is an error, not an empty list.
pub fn parse_workspace_globs(yaml: &str) -> Result<Vec<String>, BosunError> {
    const FIX: &str = "write `packages:` in pnpm-workspace.yaml as a block list (`  - 'packages/*'`) or a one-line list (`packages: ['packages/*']`)";
    let mut out = Vec::new();
    let mut in_list = false;
    for raw in yaml.lines() {
        let line = yaml_code(raw);
        if line.trim().is_empty() {
            continue;
        }
        if !line.starts_with([' ', '\t', '-']) {
            in_list = false;
            let Some(value) = line.trim_end().strip_prefix("packages:") else {
                continue;
            };
            let value = value.trim();
            if value.is_empty() {
                in_list = true;
            } else if let Some(inner) = value.strip_prefix('[').and_then(|v| v.strip_suffix(']')) {
                out.extend(
                    inner
                        .split(',')
                        .map(unquote)
                        .filter(|i| !i.is_empty())
                        .map(String::from),
                );
            } else {
                return Err(workspace_error(
                    format!("pnpm-workspace.yaml: cannot read `packages: {value}`"),
                    FIX,
                ));
            }
        } else if let (true, Some(item)) = (in_list, line.trim().strip_prefix('-')) {
            let item = unquote(item);
            if !item.is_empty() {
                out.push(item.to_string());
            }
        }
    }
    Ok(out)
}

type Glob = crate::core::glob::Glob;

/// The include and the `!` exclude globs of a `packages:` list. A glob that does not parse
/// (braces, classes) is an error, not a dropped entry.
fn compile_globs(globs: &[String]) -> Result<(Vec<Glob>, Vec<Glob>), BosunError> {
    let parse = |g: &str| {
        Glob::parse(g.trim_end_matches('/')).map_err(|e| {
            workspace_error(
                format!("pnpm-workspace.yaml: {e}"),
                "use only `*`, `?` and `**` in `packages:` globs, one glob per line",
            )
        })
    };
    let mut inc = Vec::new();
    let mut exc = Vec::new();
    for g in globs {
        match g.strip_prefix('!') {
            Some(rest) => exc.push(parse(rest)?),
            None => inc.push(parse(g)?),
        }
    }
    Ok((inc, exc))
}

/// Keep the directories (relative, `/`-separated) that match an include glob and no `!` glob.
pub fn select_dirs(dirs: &[String], globs: &[String]) -> Result<Vec<String>, BosunError> {
    let (inc, exc) = compile_globs(globs)?;
    Ok(dirs
        .iter()
        .filter(|d| inc.iter().any(|g| g.matches(d)) && !exc.iter().any(|g| g.matches(d)))
        .cloned()
        .collect())
}

/// Directories that are not searched for packages (they can be huge or cyclic) unless a
/// `packages:` glob matches the directory itself. `node_modules` and dot directories never are.
const SKIP_DIRS: [&str; 4] = ["target", "dist", "build", "coverage"];

/// Relative paths of every directory under `dir` that holds a `package.json` (the root itself is
/// not listed). `node_modules`, dot directories and symlinks are not entered; `SKIP_DIRS` only
/// when no include glob in `inc` matches that directory.
fn package_dirs(dir: &Path, rel: &str, inc: &[Glob], out: &mut Vec<String>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    // `DirEntry::file_type` does not follow symlinks, so a link loop cannot recurse.
    let mut subs: Vec<_> = entries
        .flatten()
        .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
        .collect();
    subs.sort_by_key(|e| e.file_name());
    for e in subs {
        let name = e.file_name().to_string_lossy().into_owned();
        if name == "node_modules" || name.starts_with('.') {
            continue;
        }
        let sub = if rel.is_empty() {
            name.clone()
        } else {
            format!("{rel}/{name}")
        };
        if SKIP_DIRS.contains(&name.as_str()) && !inc.iter().any(|g| g.matches(&sub)) {
            continue;
        }
        if e.path().join("package.json").is_file() {
            out.push(sub.clone());
        }
        package_dirs(&e.path(), &sub, inc, out);
    }
}

/// Read `pnpm-workspace.yaml` in `stack_dir`, expand its globs to directories with a
/// `package.json`, and read each package's `name`. A package.json without a `name` is skipped.
fn load_pnpm(stack_dir: &Path) -> Result<Vec<Package>, BosunError> {
    let file = stack_dir.join("pnpm-workspace.yaml");
    let yaml = std::fs::read_to_string(&file).map_err(|e| {
        BosunError::error(
            format!("cannot read {}: {e}", file.display()),
            "add a pnpm-workspace.yaml with a `packages:` list at the stack path",
        )
    })?;
    let globs = parse_workspace_globs(&yaml)?;
    let (inc, _) = compile_globs(&globs)?;
    let mut dirs = Vec::new();
    package_dirs(stack_dir, "", &inc, &mut dirs);
    let mut out = Vec::new();
    for rel in select_dirs(&dirs, &globs)? {
        let manifest = stack_dir.join(&rel).join("package.json");
        let name = std::fs::read_to_string(&manifest)
            .ok()
            .and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok())
            .and_then(|v| v["name"].as_str().map(String::from));
        if let Some(name) = name {
            out.push(Package {
                name,
                dir: stack_dir.join(&rel),
            });
        }
    }
    Ok(out)
}

/// Like `affected_cargo`: each changed path goes to the package with the longest dir prefix,
/// whatever its extension (a `.snap`, `.css` or `.md` inside a package affects it). A root
/// `package.json`, `pnpm-lock.yaml`, `pnpm-workspace.yaml`, `.npmrc` or `.nvmrc`, or a file with
/// extension ts, tsx, mts, cts, js, jsx, mjs, cjs, json, yaml or yml in no package, means all
/// packages; other files outside every package (docs) affect none.
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
                    Some(
                        "ts" | "tsx"
                            | "mts"
                            | "cts"
                            | "js"
                            | "jsx"
                            | "mjs"
                            | "cjs"
                            | "json"
                            | "yaml"
                            | "yml"
                    )
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
    fn the_generic_adapter_has_no_packages_and_affects_everything() {
        for name in ["none", "generic"] {
            assert_eq!(load(name, Path::new("/r")).unwrap(), vec![]);
            let files = vec!["a.py".to_string()];
            assert_eq!(affected(name, Path::new("/r"), &files, &[]), Affected::All);
        }
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
    fn unknown_adapter_is_an_error_with_a_fix() {
        let e = load("nope", Path::new("/r")).unwrap_err();
        assert!(e.message.contains("nope"), "{}", e.message);
        assert!(!e.fix.is_empty());
    }

    #[test]
    fn workspace_globs_parse() {
        let y = "# c\npackages:\n  - 'packages/*'\n  - \"apps/**\" # x\n  - '!**/test/**'\ncatalog:\n  - nope\n";
        assert_eq!(
            parse_workspace_globs(y).unwrap(),
            ["packages/*", "apps/**", "!**/test/**"]
        );
        assert_eq!(
            parse_workspace_globs("packages:\n- a\n- b\n").unwrap(),
            ["a", "b"]
        );
        assert!(parse_workspace_globs("name: x\n").unwrap().is_empty());
    }

    #[test]
    fn flow_list_parses_and_odd_values_are_errors() {
        assert_eq!(
            parse_workspace_globs("packages: [a, 'b/*', \"c\"] # x\ncatalog: [z]\n").unwrap(),
            ["a", "b/*", "c"]
        );
        for bad in [
            "packages: packages/*\n",
            "packages: [a,\n  b]\n",
            "packages: {a: 1}\n",
        ] {
            let e = parse_workspace_globs(bad).unwrap_err();
            assert!(!e.fix.is_empty(), "{bad}");
        }
        let dirs = vec!["packages/a".to_string()];
        let e = select_dirs(&dirs, &["packages/{a,b}".to_string()]).unwrap_err();
        assert!(e.fix.contains("`*`"), "{}", e.fix);
    }

    #[test]
    fn globs_select_dirs() {
        let dirs: Vec<String> = ["packages/a", "packages/b", "packages/b/test", "tools/x"]
            .map(String::from)
            .to_vec();
        let g: Vec<String> = ["packages/*", "tools/*", "!tools/x"]
            .map(String::from)
            .to_vec();
        assert_eq!(
            select_dirs(&dirs, &g).unwrap(),
            ["packages/a", "packages/b"]
        );
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
            "vitest.config.mts",
            "eslint.config.cts",
        ] {
            assert_eq!(aff(&[f]), Affected::All, "{f}");
        }
        assert_eq!(aff(&["docs/x.md", "README.md"]), Affected::None);
        // Whatever the extension, a file inside a package affects it.
        for f in [
            "__snapshots__/x.snap",
            "a.css",
            "q.sql",
            "README.md",
            "fixtures/in.bin",
        ] {
            let path = format!("packages/a/{f}");
            assert_eq!(aff(&[&path]), Affected::Some(vec!["a".into()]), "{f}");
        }
    }

    fn write(root: &Path, rel: &str, text: &str) {
        let p = root.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, text).unwrap();
    }

    #[test]
    fn load_pnpm_reads_flow_lists_and_skips_odd_dirs() {
        let t = tempfile::tempdir().unwrap();
        let r = t.path();
        write(
            r,
            "pnpm-workspace.yaml",
            "packages: ['packages/*', \"tools/**\"]\n",
        );
        write(r, "packages/a/package.json", "{\"name\":\"@w/a\"}");
        write(r, "packages/nameless/package.json", "{}");
        write(
            r,
            "packages/a/dist/x/package.json",
            "{\"name\":\"dist-pkg\"}",
        );
        write(r, "tools/x/y/package.json", "{\"name\":\"y\"}");
        // A symlink loop must neither hang nor add packages.
        #[cfg(unix)]
        std::os::unix::fs::symlink(r, r.join("packages/a/loop")).unwrap();
        let got = load_pnpm(r).unwrap();
        let names: Vec<_> = got.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names, ["@w/a", "y"]);
    }

    #[test]
    fn a_package_named_build_is_found_when_a_glob_matches_it() {
        let t = tempfile::tempdir().unwrap();
        let r = t.path();
        write(r, "pnpm-workspace.yaml", "packages:\n  - 'packages/*'\n");
        write(r, "packages/build/package.json", "{\"name\":\"build\"}");
        write(r, "packages/dist/package.json", "{\"name\":\"dist\"}");
        write(r, "packages/a/package.json", "{\"name\":\"a\"}");
        // Not matched by a glob: still skipped, as are node_modules and dot dirs.
        write(r, "packages/a/build/package.json", "{\"name\":\"nested\"}");
        write(r, "packages/node_modules/package.json", "{\"name\":\"nm\"}");
        write(r, "packages/.hid/package.json", "{\"name\":\"hid\"}");
        let got = load_pnpm(r).unwrap();
        let names: Vec<_> = got.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names, ["a", "build", "dist"]);
    }
}
