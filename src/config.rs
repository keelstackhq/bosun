//! `.bosun/bosun.toml` and `.bosun/limits.toml` (spec §3, §5, §10). Unknown keys are errors.

use std::collections::BTreeMap;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::output::BosunError;

pub const CONFIG_FILE: &str = ".bosun/bosun.toml";
pub const LIMITS_FILE: &str = ".bosun/limits.toml";

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields, default)]
pub struct Config {
    /// Empty when absent; `Ctx::load` fills in the repo name.
    pub project: String,
    pub stack: Vec<Stack>,
    pub review: Review,
    pub slack: Option<Slack>,
    pub prices: BTreeMap<String, Price>,
    pub approvals: Approvals,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            project: String::new(),
            stack: vec![Stack::default()],
            review: Review::default(),
            slack: None,
            prices: BTreeMap::new(),
            approvals: Approvals::default(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields, default)]
pub struct Stack {
    pub path: String,
    pub profile: String,
}

impl Default for Stack {
    fn default() -> Self {
        Stack {
            path: ".".into(),
            profile: "rust".into(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum ReviewMode {
    #[default]
    Subagent,
    Ci,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields, default)]
pub struct Review {
    pub mode: ReviewMode,
    /// Model of `bosun review-ci` (mode b).
    pub model: String,
    pub max_tokens: u32,
    /// `review-ci` refuses a diff with more patch bytes than this; it never truncates.
    pub max_diff_bytes: u64,
}

impl Default for Review {
    fn default() -> Self {
        Review {
            mode: ReviewMode::default(),
            model: "claude-opus-5-5".into(),
            max_tokens: 8192,
            max_diff_bytes: 200_000,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields, default)]
pub struct Slack {
    pub channel: String,
    pub spawners: String,
    pub may_spawn: bool,
    pub may_build: bool,
    pub notify: Vec<String>,
}

impl Default for Slack {
    fn default() -> Self {
        Slack {
            channel: String::new(),
            spawners: "owners".into(),
            may_spawn: true,
            may_build: false,
            notify: Vec::new(),
        }
    }
}

/// `[approvals]`: `bootstrap_until` (`YYYY-MM-DD`, UTC, inclusive) is the last day of bootstrap
/// mode (D-012, spec §8); absent means bootstrap mode is off.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, Default)]
#[serde(deny_unknown_fields, default)]
pub struct Approvals {
    pub bootstrap_until: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Price {
    pub input_per_mtok: f64,
    pub output_per_mtok: f64,
}

/// `.bosun/limits.toml`; defaults are the table in spec §5.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, Default)]
#[serde(deny_unknown_fields, default)]
pub struct Limits {
    pub sessions: SessionLimits,
    pub waves: WaveLimits,
    pub turns: TurnLimits,
    pub models: Models,
    pub budget: Budget,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields, default)]
pub struct SessionLimits {
    pub max_depth: u32,
    pub max_live_sessions: u32,
    pub max_children: u32,
}

impl Default for SessionLimits {
    fn default() -> Self {
        SessionLimits {
            max_depth: 2,
            max_live_sessions: 4,
            max_children: 3,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields, default)]
pub struct WaveLimits {
    pub max_parallel: u32,
}

impl Default for WaveLimits {
    fn default() -> Self {
        WaveLimits { max_parallel: 6 }
    }
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields, default)]
pub struct TurnLimits {
    pub build: u32,
    pub build_cap: u32,
    pub review: u32,
    pub fix_round: u32,
}

impl Default for TurnLimits {
    fn default() -> Self {
        TurnLimits {
            build: 100,
            build_cap: 150,
            review: 40,
            fix_round: 40,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields, default)]
pub struct Models {
    pub read: String,
    pub build: String,
    pub review: String,
    pub hard: String,
}

impl Default for Models {
    fn default() -> Self {
        Models {
            read: "haiku".into(),
            build: "sonnet".into(),
            review: "sonnet".into(),
            hard: "opus".into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields, default)]
pub struct Budget {
    pub daily_usd: f64,
    pub per_task_usd: f64,
}

impl Default for Budget {
    fn default() -> Self {
        Budget {
            daily_usd: 150.0,
            per_task_usd: 25.0,
        }
    }
}

fn parse_toml<T: for<'de> Deserialize<'de>>(text: &str, file: &str) -> Result<T, BosunError> {
    toml::from_str(text).map_err(|e| {
        let line = e
            .span()
            .map(|s| text[..s.start.min(text.len())].matches('\n').count() + 1);
        let at = line.map(|l| format!(" (line {l})")).unwrap_or_default();
        BosunError::error(
            format!("{file}{at}: {}", e.message()),
            format!("edit {file}: remove or rename the key named in this message, or fix the TOML syntax; valid keys are in docs/spec.md"),
        )
    })
}

fn read_optional(path: &Path, label: &str) -> Result<Option<String>, BosunError> {
    match std::fs::read_to_string(path) {
        Ok(s) => Ok(Some(s)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(BosunError::error(
            format!("cannot read {label}: {e}"),
            format!("check that {label} is a readable file"),
        )),
    }
}

impl Config {
    /// Parse the text of `.bosun/bosun.toml`; `file` is the name used in errors.
    pub fn parse(text: &str, file: &str) -> Result<Config, BosunError> {
        let c: Config = parse_toml(text, file)?;
        for (key, zero) in [
            ("max_tokens", c.review.max_tokens == 0),
            ("max_diff_bytes", c.review.max_diff_bytes == 0),
        ] {
            if zero {
                return Err(BosunError::error(
                    format!("{file}: review.{key} must be at least 1"),
                    format!("edit {file}: set review.{key} to a positive number, or remove it"),
                ));
            }
        }
        if let Some(d) = &c.approvals.bootstrap_until {
            let day = crate::time::parse_rfc3339(&format!("{d}T00:00:00Z"));
            if day.is_none_or(|t| !t.to_string().starts_with(d.as_str())) {
                return Err(BosunError::error(
                    format!("{file}: approvals.bootstrap_until {d:?} is not a YYYY-MM-DD date"),
                    format!(
                        "edit {file}: set approvals.bootstrap_until to a date like \"2026-11-30\", or remove it"
                    ),
                ));
            }
        }
        Ok(c)
    }

    /// Read `<root>/.bosun/bosun.toml`; a missing file gives the defaults.
    pub fn load(root: &Path) -> Result<Config, BosunError> {
        match read_optional(&root.join(CONFIG_FILE), CONFIG_FILE)? {
            Some(t) => Config::parse(&t, CONFIG_FILE),
            None => Ok(Config::default()),
        }
    }
}

impl Limits {
    /// Parse the text of `.bosun/limits.toml`; missing keys take the spec §5 defaults.
    pub fn parse(text: &str, file: &str) -> Result<Limits, BosunError> {
        parse_toml(text, file)
    }

    /// Read `<root>/.bosun/limits.toml`; a missing file gives the defaults.
    pub fn load(root: &Path) -> Result<Limits, BosunError> {
        match read_optional(&root.join(LIMITS_FILE), LIMITS_FILE)? {
            Some(t) => Limits::parse(&t, LIMITS_FILE),
            None => Ok(Limits::default()),
        }
    }
}
