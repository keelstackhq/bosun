//! Gate profiles (spec §7): built-in TOML embedded in the binary, overridden per repo by
//! `.bosun/profiles/<name>.toml`.

use std::collections::BTreeMap;
use std::path::Path;

use serde::Deserialize;

use crate::output::BosunError;

/// One gate takes at most 8 minutes (spec §7).
pub const DEFAULT_BUDGET_SECS: u64 = 480;

const RUST: &str = include_str!("../../profiles/rust.toml");

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Profile {
    /// The stack manifest; a stack without it (at head and base) has nothing to check yet.
    #[serde(default)]
    pub manifest: Option<String>,
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
    /// Patterns for the generic `regex` adapters; absent in profiles with built-in adapters.
    #[serde(default)]
    pub regex: RegexKeys,
}

/// `[adapters.regex]`: what the `pending = "regex"` and `assertions = "regex"` adapters look for.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Default)]
#[serde(deny_unknown_fields, default)]
pub struct RegexKeys {
    /// Globs (relative to the stack directory) of the test files.
    pub test_files: Vec<String>,
    /// Regexes; a line that matches one is a skip marker.
    pub pending: Vec<String>,
    /// Regexes; every match counts as one assertion.
    pub assertions: Vec<String>,
}

/// The adapter names bosun knows (closed set): a typo such as `regexp` must not pass as full.
// pnpm-workspace is the ts-pnpm profile's adapter (#122); listed here so the two PRs compose in either order.
pub const PACKAGES_ADAPTERS: [&str; 4] = ["cargo-metadata", "pnpm-workspace", "none", "generic"];
pub const PENDING_ADAPTERS: [&str; 3] = ["rust-ignore", "ts-skip", "regex"];
pub const ASSERTIONS_ADAPTERS: [&str; 3] = ["rust", "ts", "regex"];

impl Adapters {
    /// Err(message, fix) for the first adapter name outside the closed sets.
    pub fn validate(&self) -> Result<(), (String, String)> {
        let sets: [(&str, &String, &[&str]); 3] = [
            ("packages", &self.packages, &PACKAGES_ADAPTERS),
            ("pending", &self.pending, &PENDING_ADAPTERS),
            ("assertions", &self.assertions, &ASSERTIONS_ADAPTERS),
        ];
        for (key, name, valid) in sets {
            if !valid.contains(&name.as_str()) {
                return Err((
                    format!("unknown {key} adapter `{name}`"),
                    format!("set `{key}` in [adapters] to one of: {}", valid.join(", ")),
                ));
            }
        }
        Ok(())
    }

    /// Does test-integrity use the generic regex adapters for this profile?
    pub fn uses_regex(&self) -> bool {
        self.pending == "regex" || self.assertions == "regex"
    }

    /// The generic adapter kind the profile relies on (`regex` or `generic`), if any. A stack
    /// with one gets reduced integrity (spec section 7).
    pub fn generic_kind(&self) -> Option<&'static str> {
        if self.uses_regex() {
            Some("regex")
        } else if matches!(self.packages.as_str(), "none" | "generic") {
            Some("generic")
        } else {
            None
        }
    }
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
    /// A file in the stack directory the gate needs; the gate is skipped when it is missing at
    /// the head and at the base.
    #[serde(default)]
    pub requires: Option<String>,
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
                _ => {
                    return Err(BosunError::usage(
                        format!("unknown profile `{name}`"),
                        format!("use `rust`, or add .bosun/profiles/{name}.toml"),
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
        let profile: Profile = toml::from_str(&text).map_err(|e| {
            BosunError::error(
                format!("{label}: {}", e.message()),
                format!("fix {label}: gates need `cmd`; see docs/spec.md section 7"),
            )
        })?;
        profile
            .adapters
            .validate()
            .map_err(|(msg, fix)| BosunError::error(format!("{label}: {msg}"), fix))?;
        Ok(profile)
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
        assert_eq!(p.manifest.as_deref(), Some("Cargo.toml"));
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
            requires: None,
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
    fn packages_placeholder_replaces_the_item_or_appends() {
        let p = Profile::load(Path::new("/nonexistent"), "rust").unwrap();
        let t = &p.gates["test"];
        assert_eq!(
            t.expand_packages(t.expand_cmd(&[], None), vec!["-p".into(), "x".into()]),
            ["cargo", "nextest", "run", "--locked", "-p", "x"]
        );
        let argv: Vec<String> = ["tool", "{packages}", "run"].map(String::from).to_vec();
        assert_eq!(
            t.expand_packages(argv, vec!["--filter".into(), "x".into()]),
            ["tool", "--filter", "x", "run"]
        );
    }

    #[test]
    fn requires_is_an_optional_gate_key() {
        let t = "[adapters]\npackages = \"cargo-metadata\"\npending = \"rust-ignore\"\nassertions = \"rust\"\n\
                 [gates.deps]\ncmd = [\"x\"]\nrequires = \"Cargo.lock\"\n[gates.fmt]\ncmd = [\"y\"]\n";
        let p: Profile = toml::from_str(t).unwrap();
        assert_eq!(p.gates["deps"].requires.as_deref(), Some("Cargo.lock"));
        assert_eq!(p.gates["fmt"].requires, None);
    }

    #[test]
    fn regex_keys_are_optional_and_name_the_generic_kind() {
        let head = "[adapters]\npackages = \"cargo-metadata\"\n";
        let gates = "[gates.x]\ncmd = [\"x\"]\n";
        let old = format!("{head}pending = \"rust-ignore\"\nassertions = \"rust\"\n{gates}");
        let p: Profile = toml::from_str(&old).unwrap();
        assert_eq!(p.adapters.regex, RegexKeys::default());
        assert_eq!(p.adapters.generic_kind(), None);
        let new = format!(
            "[adapters]\npackages = \"none\"\npending = \"regex\"\nassertions = \"regex\"\n\
             [adapters.regex]\ntest_files = [\"**/test_*.py\"]\npending = [\"skip\"]\n{gates}"
        );
        let p: Profile = toml::from_str(&new).unwrap();
        assert_eq!(p.adapters.regex.test_files, ["**/test_*.py"]);
        assert!(p.adapters.regex.assertions.is_empty());
        assert_eq!(p.adapters.generic_kind(), Some("regex"));
        let none = format!(
            "[adapters]\npackages = \"none\"\npending = \"x\"\nassertions = \"y\"\n{gates}"
        );
        let p: Profile = toml::from_str(&none).unwrap();
        assert_eq!(p.adapters.generic_kind(), Some("generic"));
    }

    #[test]
    fn adapter_names_are_a_closed_set() {
        let dir = tempfile::tempdir().unwrap();
        let prof = dir.path().join(".bosun/profiles");
        std::fs::create_dir_all(&prof).unwrap();
        let text = "[adapters]\npackages = \"none\"\npending = \"regexp\"\nassertions = \"regex\"\n\
                    [gates.x]\ncmd = [\"x\"]\n";
        std::fs::write(prof.join("p.toml"), text).unwrap();
        let e = Profile::load(dir.path(), "p").unwrap_err();
        assert!(
            e.message.contains("pending adapter `regexp`"),
            "{}",
            e.message
        );
        assert!(e.fix.contains("rust-ignore, ts-skip, regex"), "{}", e.fix);
    }

    #[test]
    fn override_and_errors() {
        let dir = tempfile::tempdir().unwrap();
        let prof = dir.path().join(".bosun/profiles");
        std::fs::create_dir_all(&prof).unwrap();
        std::fs::write(
            prof.join("rust.toml"),
            "[adapters]\npackages=\"none\"\npending=\"ts-skip\"\nassertions=\"ts\"\n[gates.x]\ncmd=[\"x\"]\n",
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
