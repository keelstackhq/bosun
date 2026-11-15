//! `.bosun/bosun.toml` and `.bosun/limits.toml` (spec §3, §5, §10). Unknown keys are errors.

use std::collections::BTreeMap;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::core::approve::Policy;
use crate::core::mode::{Effective, ModeConfig};
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
    pub cloud: Cloud,
    pub prices: BTreeMap<String, Price>,
    pub approvals: Approvals,
    /// Zero-touch onboarding (spec section 3): when true, `.bosun/intake.toml` on the default
    /// branch must be present and valid before agents are spawned. Release ordering: a pinned
    /// release rejects unknown keys, so a repo may set this only after its `.bosun/version`
    /// pins a release that reads it (0.5.0 or later, which is also added to KNOWN in
    /// tests/pinned_config.rs). Bosun's own `.bosun/bosun.toml` therefore does not set it yet.
    pub intake: bool,
    /// `[trailers]`: commit trailers for traceability (spec §4). Release ordering as for `intake`:
    /// set it only once `.bosun/version` pins a release that reads it.
    pub trailers: Trailers,
    /// `[checks]`: where the gates are posted (spec §15, D33). Release ordering as for `intake`:
    /// set it only once `.bosun/version` pins a release that reads it.
    pub checks: Checks,
    /// `[session]`: Claude Code session settings bosun hands out (spec §5). Release ordering as
    /// for `intake`: set it only once `.bosun/version` pins a release that reads it.
    pub session: Session,
    /// `[mode]`: a preset and knob overrides. Release ordering as for `intake`.
    #[serde(skip_serializing_if = "ModeConfig::is_unset")]
    pub mode: ModeConfig,
    /// `[routines]`: opt-in Routines (spec §5). Release ordering as for `intake`.
    #[serde(skip_serializing_if = "Routines::is_unset")]
    pub routines: Routines,
}

/// `[routines]`: the janitor is off unless `janitor = true`; `janitor_cron` (UTC) is its schedule.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields, default)]
pub struct Routines {
    pub janitor: bool,
    pub janitor_cron: String,
}

impl Default for Routines {
    fn default() -> Self {
        Routines {
            janitor: false,
            janitor_cron: "0 * * * *".into(),
        }
    }
}

impl Routines {
    fn is_unset(&self) -> bool {
        *self == Routines::default()
    }
}

/// `[session]`: the auto-compact point `bosun setup-snippet` prints. `autocompact_window`
/// (100000-1000000 tokens) is `CLAUDE_CODE_AUTO_COMPACT_WINDOW`; `autocompact_pct` (1-100) is
/// `CLAUDE_AUTOCOMPACT_PCT_OVERRIDE`, local sessions only (cloud sessions set it themselves).
/// Raw values, so a wrong type gets the named-key message of [`Session::resolve`].
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, Default)]
#[serde(deny_unknown_fields, default)]
pub struct Session {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub autocompact_pct: Option<toml::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub autocompact_window: Option<toml::Value>,
}

/// The checked `[session]` values: (pct, window).
pub type Autocompact = (Option<i64>, Option<i64>);

impl Session {
    /// Check both keys; an error names the key, its range and `file`.
    pub fn resolve(&self, file: &str) -> Result<Autocompact, BosunError> {
        let one = |key: &str, v: &Option<toml::Value>, lo: i64, hi: i64| match v {
            None => Ok(None),
            Some(toml::Value::Integer(n)) if (lo..=hi).contains(n) => Ok(Some(*n)),
            Some(_) => Err(BosunError::error(
                format!("{file}: session.{key} must be a whole number from {lo} to {hi}"),
                format!(
                    "edit {file}: set session.{key} to a number from {lo} to {hi}, or remove it"
                ),
            )),
        };
        Ok((
            one("autocompact_pct", &self.autocompact_pct, 1, 100)?,
            one(
                "autocompact_window",
                &self.autocompact_window,
                100_000,
                1_000_000,
            )?,
        ))
    }
}

/// How bosun posts `risk-check`, `review-verdict` and `main-open`: commit statuses (today),
/// statuses and App check runs (`dual`, the transition), or App check runs only (`checks`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum CheckMode {
    #[default]
    Statuses,
    Dual,
    Checks,
}

impl CheckMode {
    pub fn as_str(self) -> &'static str {
        match self {
            CheckMode::Statuses => "statuses",
            CheckMode::Dual => "dual",
            CheckMode::Checks => "checks",
        }
    }
}

/// `[checks]`: `app_id` and `app_slug` name the gate App; required unless `mode = "statuses"`.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, Default)]
#[serde(deny_unknown_fields, default)]
pub struct Checks {
    pub mode: CheckMode,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub app_id: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub app_slug: Option<String>,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            project: String::new(),
            stack: vec![Stack::default()],
            review: Review::default(),
            slack: None,
            cloud: Cloud::default(),
            prices: BTreeMap::new(),
            approvals: Approvals::default(),
            intake: false,
            trailers: Trailers::default(),
            checks: Checks::default(),
            session: Session::default(),
            mode: ModeConfig::default(),
            routines: Routines::default(),
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

/// `[cloud]`: the environment a relayed root starts in; absent, it inherits the caller's (ZH).
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, Default)]
#[serde(deny_unknown_fields, default)]
pub struct Cloud {
    pub environment_id: Option<String>,
}

/// `[approvals]`: `bootstrap_until` (`YYYY-MM-DD`, UTC, inclusive) is the last day of bootstrap
/// mode (D-012, spec §8); absent means bootstrap mode is off. `policy` says who approves once it
/// is over (D-014); absent means `owner` (fail safe).
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, Default)]
#[serde(deny_unknown_fields, default)]
pub struct Approvals {
    pub bootstrap_until: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub policy: Option<Policy>,
    /// Exact reviewer model ids whose verdicts count for two-verdicts, delegate and autonomous
    /// approval. Unset: any model counts. A new model id stops counting until enrolled here.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub review_models: Option<Vec<String>>,
    /// Opt in to the CI signer (`bosun approve --as signer`, D-017 keyless lane, spec §8). Off by
    /// default; turning it on is R-weaken (any `[approvals]` change is). A pinned release older
    /// than the one that adds this key fails to parse it: pin first, then set it.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub signer: bool,
    /// Globs autonomous approval (D-017) may cover. Unset: `src/**`, `tests/**`, `docs/**`,
    /// `changelog.d/**`, `.changeset/*.md`; the deny set and the denied rules always apply. A glob that does not parse allows nothing. Any `[approvals]` change is
    /// the owner's (R-weaken).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub autonomous_allow: Option<Vec<String>>,
    /// `policy` came from the `[mode]` preset, not from this table.
    #[serde(skip)]
    pub policy_preset: bool,
}

impl Approvals {
    pub fn policy(&self) -> Policy {
        self.policy.unwrap_or_default()
    }

    /// Who set the effective policy: `explicit` (`[approvals]`), `preset` (`[mode]`), `default`.
    pub fn policy_source(&self) -> &'static str {
        match (self.policy_preset, self.policy.is_some()) {
            (true, _) => "preset",
            (_, true) => "explicit",
            _ => "default",
        }
    }
}

/// `[trailers] require`: when true, `bosun pr` refuses a PR with a commit lacking
/// `Bosun-Session` or `Bosun-Task`; when false (default) it only notes them in `data`.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, Default)]
#[serde(deny_unknown_fields, default)]
pub struct Trailers {
    pub require: bool,
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
    /// Waves after which a root relays to a fresh root (spec §5.2).
    pub relay_waves: u32,
}

impl Default for WaveLimits {
    fn default() -> Self {
        WaveLimits {
            max_parallel: 6,
            relay_waves: 6,
        }
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

/// A 5-field cron expression: each field is a comma list of `*`, `N` or `A-B`, with an optional
/// `/step` (1 up to the field's range), inside the field's range (day of week 0-7; JAN-DEC in
/// the month and SUN-SAT in the day-of-week field, any case).
fn valid_cron(expr: &str) -> bool {
    const RANGES: [(u32, u32); 5] = [(0, 59), (0, 23), (1, 31), (1, 12), (0, 7)];
    const MONTHS: &[&str] = &[
        "JAN", "FEB", "MAR", "APR", "MAY", "JUN", "JUL", "AUG", "SEP", "OCT", "NOV", "DEC",
    ];
    const DAYS: &[&str] = &["SUN", "MON", "TUE", "WED", "THU", "FRI", "SAT"];
    let fields: Vec<&str> = expr.split_whitespace().collect();
    fields.len() == 5
        && fields
            .iter()
            .zip(RANGES)
            .enumerate()
            .all(|(i, (f, (lo, hi)))| {
                let names: (&[&str], u32) = match i {
                    3 => (MONTHS, 1),
                    4 => (DAYS, 0),
                    _ => (&[], 0),
                };
                f.split(',').all(|part| cron_part(part, lo, hi, names))
            })
}

/// One list item; `names` are the field's names and the number of the first one.
fn cron_part(part: &str, lo: u32, hi: u32, names: (&[&str], u32)) -> bool {
    let (range, step) = part
        .split_once('/')
        .map_or((part, None), |(r, s)| (r, Some(s)));
    let num = |t: &str| {
        let by_name =
            (names.0.iter().position(|n| n.eq_ignore_ascii_case(t))).map(|i| i as u32 + names.1);
        by_name.or_else(|| t.parse::<u32>().ok().filter(|n| (lo..=hi).contains(n)))
    };
    let range_ok = range == "*"
        || num(range).is_some()
        || range
            .split_once('-')
            .is_some_and(|(a, b)| matches!((num(a), num(b)), (Some(a), Some(b)) if a <= b));
    range_ok
        && step.is_none_or(|s| {
            s.parse::<u32>()
                .is_ok_and(|n| (1..=hi - lo + 1).contains(&n))
        })
}

impl Config {
    /// Parse the text of `.bosun/bosun.toml`; `file` is the name used in errors.
    pub fn parse(text: &str, file: &str) -> Result<Config, BosunError> {
        let mut c: Config = parse_toml(text, file)?;
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
        if c.routines.janitor && !valid_cron(&c.routines.janitor_cron) {
            return Err(BosunError::error(
                format!(
                    "{file}: routines.janitor_cron {:?} is not a valid 5-field cron expression (minute 0-59, hour 0-23, day 1-31, month 1-12 or JAN-DEC, weekday 0-7 or SUN-SAT; steps 1 up to the range)",
                    c.routines.janitor_cron
                ),
                format!(
                    "edit {file}: set routines.janitor_cron to five fields such as \"0 * * * *\", or remove it"
                ),
            ));
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
        c.session.resolve(file)?;
        if c.mode.policy.is_some() {
            return Err(BosunError::error(
                format!("{file}: policy is not a [mode] key"),
                format!("edit {file}: set [approvals] policy, or remove mode.policy"),
            ));
        }
        // The preset fills the policy only when `[approvals]` leaves it unset.
        if c.approvals.policy.is_none() {
            c.approvals.policy = c.mode.name.and_then(crate::core::mode::preset_policy);
            c.approvals.policy_preset = c.approvals.policy.is_some();
        }
        let k = &c.checks;
        if k.mode != CheckMode::Statuses {
            let id_ok = k.app_id.is_some_and(|i| i > 0);
            let slug_ok = k.app_slug.as_deref().is_some_and(|s| !s.trim().is_empty());
            let actions = k.app_id == Some(15368)
                || k.app_slug
                    .as_deref()
                    .is_some_and(|s| s.trim().eq_ignore_ascii_case("github-actions"));
            if actions {
                return Err(BosunError::error(
                    format!("{file}: checks.app_id or checks.app_slug names GitHub Actions"),
                    format!(
                        "edit {file}: set them to the dedicated gate App (spec §15); any workflow can post as GitHub Actions"
                    ),
                ));
            }
            if !id_ok || !slug_ok {
                return Err(BosunError::error(
                    format!(
                        "{file}: checks.mode = {:?} needs checks.app_id and checks.app_slug",
                        k.mode.as_str()
                    ),
                    format!(
                        "edit {file}: set checks.app_id (a positive number) and checks.app_slug to the gate App's id and slug, or set checks.mode = \"statuses\""
                    ),
                ));
            }
        }
        Ok(c)
    }

    /// The knobs `[mode]` resolves to (explicit > preset > default).
    pub fn mode_effective(&self) -> Effective {
        crate::core::mode::resolve(&self.mode)
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
        let l: Limits = parse_toml(text, file)?;
        if l.waves.relay_waves == 0 {
            return Err(BosunError::error(
                format!("{file}: waves.relay_waves must be at least 1"),
                format!("edit {file}: set waves.relay_waves to a positive number, or remove it"),
            ));
        }
        Ok(l)
    }

    /// Read `<root>/.bosun/limits.toml`; a missing file gives the defaults.
    pub fn load(root: &Path) -> Result<Limits, BosunError> {
        match read_optional(&root.join(LIMITS_FILE), LIMITS_FILE)? {
            Some(t) => Limits::parse(&t, LIMITS_FILE),
            None => Ok(Limits::default()),
        }
    }
}

#[cfg(test)]
mod cron_tests {
    use super::valid_cron;

    #[test]
    fn cron_fields_are_checked() {
        for ok in [
            "0 * * * *",
            "*/15 9-17 * * 1-5",
            "0,30 0 1 1 7",
            "0 0 * * MON",
            "0 0 * * MON-FRI",
            "0 0 1 JAN,JUL *",
            "0 0 * * mon-fri",
            "0 0 * Feb-Mar sun,sat",
            "*/59 */23 */31 */12 */7",
        ] {
            assert!(valid_cron(ok), "{ok}");
        }
        for bad in [
            "a b c d e",
            "* * * *",
            "60 * * * *",
            "* 24 * * *",
            "0 * 0 * *",
            "*/0 * * * *",
            "5-1 * * * *",
            "*/100 * * * *",
            "* */25 * * *",
            "0 0 * * FRI-MON",
            "0 0 * FOO *",
            "0 0 * * MON-",
            "0 0 JAN * *",
            "MON * * * *",
            "* * * * * *",
        ] {
            assert!(!valid_cron(bad), "{bad}");
        }
    }

    #[test]
    fn only_an_enabled_janitor_is_validated() {
        let off = "project = \"p\"\n[routines]\njanitor_cron = \"a b c d e\"\n";
        assert!(super::Config::parse(off, "f").is_ok());
        let on = format!("{off}janitor = true\n");
        let e = super::Config::parse(&on, "f").unwrap_err();
        assert!(!e.fix.is_empty());
    }
}
