//! Gate profiles (spec §7): built-in TOML embedded in the binary, overridden per repo by
//! `.bosun/profiles/<name>.toml`.

use std::collections::BTreeMap;
use std::path::Path;

use serde::Deserialize;

use crate::output::BosunError;

/// One gate takes at most 8 minutes (spec §7).
pub const DEFAULT_BUDGET_SECS: u64 = 480;

const RUST: &str = include_str!("../../profiles/rust.toml");
const TS_PNPM: &str = include_str!("../../profiles/ts-pnpm.toml");

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Profile {
    pub adapters: Adapters,
    pub gates: BTreeMap<String, Gate>,
}

/// Adapter names; the pending and assertions adapters are used by test-integrity (Task 12).
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Adapters {
    pub packages: String,
    pub pending: String,
    pub assertions: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum WhenNoPackage {
    Skip,
    #[default]
    Run,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Gate {
    pub cmd: Vec<String>,
    /// Once per affected package, `{package}` replaced by its name. Appended to `cmd`, or put in
    /// place of an item that is exactly `{packages}` (for tools whose filters come first).
    #[serde(default)]
    pub each_package: Vec<String>,
    /// Appended instead when every package is affected.
    #[serde(default)]
    pub all: Vec<String>,
    #[serde(default)]
    pub when_no_package: WhenNoPackage,
    pub budget_secs: Option<u64>,
}

impl Gate {
    pub fn budget(&self) -> u64 {
        self.budget_secs.unwrap_or(DEFAULT_BUDGET_SECS)
    }

    /// Only gates with `each_package` depend on the affected packages.
    pub fn is_per_package(&self) -> bool {
        !self.each_package.is_empty()
    }

    /// `cmd` with the placeholders filled: an item that is exactly `{files}` becomes one argv item
    /// per file; `{diff}` inside an item is replaced by the diff file path.
    pub fn expand_cmd(&self, files: &[String], diff: Option<&str>) -> Vec<String> {
        let mut argv = Vec::new();
        for item in &self.cmd {
            if item == "{files}" {
                argv.extend(files.iter().cloned());
            } else if let Some(d) = diff {
                argv.push(item.replace("{diff}", d));
            } else {
                argv.push(item.clone());
            }
        }
        argv
    }

    /// `each_package` once per package, `{package}` replaced by its name.
    pub fn package_args(&self, names: &[String]) -> Vec<String> {
        names
            .iter()
            .flat_map(|n| {
                self.each_package
                    .iter()
                    .map(move |a| a.replace("{package}", n))
            })
            .collect()
    }

    /// Put the package arguments (`extra`) in place of a `{packages}` item of `argv`, else at the end.
    pub fn expand_packages(&self, argv: Vec<String>, extra: Vec<String>) -> Vec<String> {
        match argv.iter().position(|a| a == "{packages}") {
            Some(i) => {
                let mut out = argv;
                out.splice(i..=i, extra);
                out
            }
            None => argv.into_iter().chain(extra).collect(),
        }
    }

    pub fn needs_diff(&self) -> bool {
        self.cmd.iter().any(|a| a.contains("{diff}"))
    }
}

impl Profile {
    /// `<root>/.bosun/profiles/<name>.toml` if present, else the built-in profile.
    pub fn load(root: &Path, name: &str) -> Result<Profile, BosunError> {
        let file = format!(".bosun/profiles/{name}.toml");
        let (text, label) = match std::fs::read_to_string(root.join(&file)) {
            Ok(t) => (t, file),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => match name {
                "rust" => (RUST.to_string(), "profiles/rust.toml".to_string()),
                "ts-pnpm" => (TS_PNPM.to_string(), "profiles/ts-pnpm.toml".to_string()),
                _ => {
                    return Err(BosunError::usage(
                        format!("unknown profile `{name}`"),
                        format!("use `rust` or `ts-pnpm`, or add .bosun/profiles/{name}.toml"),
                    ));
                }
            },
            Err(e) => {
                return Err(BosunError::error(
                    format!("cannot read {file}: {e}"),
                    format!("check that {file} is a readable file"),
                ));
            }
        };
        toml::from_str(&text).map_err(|e| {
            BosunError::error(
                format!("{label}: {}", e.message()),
                format!("fix {label}: gates need `cmd`; see docs/spec.md section 7"),
            )
        })
    }

    pub fn gate(&self, name: &str) -> Result<&Gate, BosunError> {
        self.gates.get(name).ok_or_else(|| {
            let known: Vec<&str> = self.gates.keys().map(String::as_str).collect();
            BosunError::usage(
                format!("this profile has no gate `{name}`"),
                format!("use one of: {}", known.join(", ")),
            )
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn built_in_rust_profile_parses() {
        let p = Profile::load(Path::new("/nonexistent"), "rust").unwrap();
        assert_eq!(p.gates.len(), 6);
        assert_eq!(p.gates["test"].when_no_package, WhenNoPackage::Skip);
        assert_eq!(p.gates["fmt"].budget(), 480);
    }

    #[test]
    fn placeholders_expand() {
        let p = Profile::load(Path::new("/nonexistent"), "rust").unwrap();
        let m = &p.gates["mutants"];
        assert!(m.needs_diff());
        assert_eq!(
            m.expand_cmd(&[], Some("/t/d.diff")),
            ["cargo", "mutants", "--in-diff", "/t/d.diff"]
        );
        let g = Gate {
            cmd: vec!["fmt".into(), "{files}".into()],
            each_package: vec!["-p".into(), "{package}".into()],
            all: vec![],
            when_no_package: WhenNoPackage::Run,
            budget_secs: Some(5),
        };
        assert_eq!(
            g.expand_cmd(&["a".into(), "b".into()], None),
            ["fmt", "a", "b"]
        );
        assert_eq!(
            g.package_args(&["x".into(), "y".into()]),
            ["-p", "x", "-p", "y"]
        );
        assert_eq!(g.budget(), 5);
    }

    #[test]
    fn built_in_ts_pnpm_profile_parses() {
        let p = Profile::load(Path::new("/nonexistent"), "ts-pnpm").unwrap();
        assert_eq!(p.adapters.packages, "pnpm-workspace");
        for g in ["fmt", "lint", "types", "test", "deps"] {
            assert!(p.gates.contains_key(g), "{g}");
        }
        let t = &p.gates["test"];
        assert_eq!(t.when_no_package, WhenNoPackage::Skip);
        let args = t.package_args(&["a".into(), "b".into()]);
        assert_eq!(
            t.expand_packages(t.expand_cmd(&[], None), args),
            ["pnpm", "--filter", "a", "--filter", "b", "run", "test"]
        );
        assert_eq!(
            t.expand_packages(t.expand_cmd(&[], None), t.all.clone()),
            ["pnpm", "-r", "run", "test"]
        );
    }

    #[test]
    fn packages_placeholder_defaults_to_append() {
        let p = Profile::load(Path::new("/nonexistent"), "rust").unwrap();
        let t = &p.gates["test"];
        assert_eq!(
            t.expand_packages(t.expand_cmd(&[], None), vec!["-p".into(), "x".into()]),
            ["cargo", "nextest", "run", "--locked", "-p", "x"]
        );
    }

    #[test]
    fn override_and_errors() {
        let dir = tempfile::tempdir().unwrap();
        let prof = dir.path().join(".bosun/profiles");
        std::fs::create_dir_all(&prof).unwrap();
        std::fs::write(
            prof.join("rust.toml"),
            "[adapters]\npackages=\"p\"\npending=\"p\"\nassertions=\"a\"\n[gates.x]\ncmd=[\"x\"]\n",
        )
        .unwrap();
        let p = Profile::load(dir.path(), "rust").unwrap();
        assert!(p.gate("x").is_ok());
        let err = p.gate("fmt").unwrap_err();
        assert_eq!(err.exit, crate::output::Exit::Usage);
        assert!(err.fix.contains('x'));
        std::fs::write(prof.join("rust.toml"), "bogus = 1\n").unwrap();
        assert!(Profile::load(dir.path(), "rust").is_err());
        assert!(Profile::load(dir.path(), "nope").is_err());
    }
}
