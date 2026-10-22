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
    /// Appended once per affected package, `{package}` replaced by its name.
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
}
