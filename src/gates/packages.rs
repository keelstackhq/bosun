//! The packages adapters (`cargo-metadata`): which workspace packages does a diff touch?

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
        // The generic adapter: the stack has no packages, so a per-package gate runs once.
        "none" | "generic" => Ok(Vec::new()),
        other => Err(BosunError::error(
            format!("unknown packages adapter `{other}`"),
            "set `packages` in the profile to a known adapter (`cargo-metadata`, `none`)",
        )),
    }
}

/// The packages a diff touches, by the profile's packages adapter. An adapter that `load` does
/// not know cannot get here; if it does, everything is affected.
pub fn affected(
    adapter: &str,
    stack_dir: &Path,
    changed: &[String],
    packages: &[Package],
) -> Affected {
    match adapter {
        "cargo-metadata" => affected_cargo(stack_dir, changed, packages),
        "none" | "generic" => Affected::All,
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
    fn unknown_adapter_is_an_error_with_a_fix() {
        let e = load("nope", Path::new("/r")).unwrap_err();
        assert!(e.message.contains("nope"), "{}", e.message);
        assert!(!e.fix.is_empty());
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
}
