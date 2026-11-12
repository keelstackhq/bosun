//! Modes (design 2026-11-11): presets, resolution with a source per knob, and the pure brake.

use serde::{Deserialize, Serialize};

use super::approve::Policy;

macro_rules! knob {
    ($name:ident { $($v:ident = $s:literal),+ }) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Deserialize, Serialize)]
        #[serde(rename_all = "kebab-case")]
        pub enum $name { $($v),+ }
        impl $name {
            pub fn as_str(self) -> &'static str {
                match self { $($name::$v => $s),+ }
            }
        }
    };
}
// The ladder, loosest owner involvement last (`Ord` is the ladder order).
knob!(ModeName {
    Gatekeeper = "gatekeeper", Steady = "steady", Booster = "booster", Autonomous = "autonomous"
});
knob!(PushGates { Ready = "ready", Push = "push" });
knob!(HealPr { Off = "off", Notify = "notify", Spawn = "spawn" });
knob!(HealMain { Off = "off", RevertPr = "revert-pr", RevertAuto = "revert-auto" });
knob!(ApprovalsBatch { PerPr = "per-pr", Daily = "daily" });
knob!(SpawnAuto { Off = "off", On = "on" });
knob!(BudgetFeature { Off = "off", Warn = "warn", Ask = "ask" });
knob!(Brake { Off = "off", On = "on" });
knob!(CuratorLoosen { Off = "off", Propose = "propose" });

/// One knob's effective value and the source that set it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct Set<T> {
    pub value: T,
    pub source: Source,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Source {
    Explicit,
    Preset,
    Default,
}

/// The effective mode and knobs (`bosun mode` prints it as is).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct Effective {
    pub mode: ModeName,
    pub mode_source: Source,
    pub knobs: EffKnobs,
}

/// Each knob with its value per mode: gatekeeper, steady, booster, autonomous.
macro_rules! knobs {
    ($($f:ident: $t:ident = [$g:ident, $s:ident, $b:ident, $a:ident]),+ $(,)?) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq)]
        pub struct Knobs { $(pub $f: $t),+ }

        #[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
        pub struct EffKnobs { $(pub $f: Set<$t>),+ }

        /// `[mode]`: `name` fills the knobs left unset; `policy` is parsed only to refuse it.
        #[derive(Debug, Clone, PartialEq, Default, Deserialize, Serialize)]
        #[serde(deny_unknown_fields, default)]
        pub struct ModeConfig {
            #[serde(skip_serializing_if = "Option::is_none")]
            pub name: Option<ModeName>,
            $(#[serde(skip_serializing_if = "Option::is_none")]
            pub $f: Option<$t>,)+
            #[serde(skip_serializing_if = "Option::is_none")]
            pub policy: Option<toml::Value>,
        }

        /// The mode x knob table of the design.
        pub fn preset(m: ModeName) -> Knobs {
            Knobs { $($f: [$t::$g, $t::$s, $t::$b, $t::$a][m as usize]),+ }
        }

        impl ModeConfig {
            pub fn is_unset(&self) -> bool {
                *self == ModeConfig::default()
            }
            /// The first knob overridden away from its gatekeeper value.
            pub fn off_default(&self) -> Option<&'static str> {
                let g = preset(ModeName::Gatekeeper);
                [$(self.$f.is_some_and(|v| v != g.$f).then_some(stringify!($f))),+]
                    .into_iter().flatten().next()
            }
        }

        /// Explicit override > preset of the named mode > default (the gatekeeper value).
        pub fn resolve(c: &ModeConfig) -> Effective {
            let base = preset(c.name.unwrap_or(ModeName::Gatekeeper));
            let (mode_source, from) = match c.name {
                Some(_) => (Source::Explicit, Source::Preset),
                None => (Source::Default, Source::Default),
            };
            Effective {
                mode: c.name.unwrap_or(ModeName::Gatekeeper),
                mode_source,
                knobs: EffKnobs { $($f: match c.$f {
                    Some(value) => Set { value, source: Source::Explicit },
                    None => Set { value: base.$f, source: from },
                }),+ },
            }
        }
    };
}
knobs!(
    push_gates: PushGates = [Ready, Push, Push, Push],
    heal_pr: HealPr = [Off, Notify, Spawn, Spawn],
    heal_main: HealMain = [Off, RevertPr, RevertPr, RevertAuto],
    approvals_batch: ApprovalsBatch = [PerPr, Daily, Daily, Daily],
    spawn_auto: SpawnAuto = [Off, Off, On, On],
    budget_feature: BudgetFeature = [Off, Warn, Ask, Ask],
    brake: Brake = [Off, On, On, On],
    curator_loosen: CuratorLoosen = [Off, Off, Propose, Propose],
);

/// The approval policy a mode implies (used from plan PR 1b; not wired yet).
pub fn preset_policy(m: ModeName) -> Option<Policy> {
    (m >= ModeName::Booster).then_some(Policy::Autonomous)
}

/// Brake trips: main red this often, reverts this many, red-CI rate points over the target.
pub const BRAKE_MAIN_RED: u32 = 2;
pub const BRAKE_REVERTS: u32 = 2;
pub const BRAKE_RATE_MARGIN_PCT: u32 = 10;

/// Counts over the brake window (7 days), read from GitHub by the caller.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BrakeWindow {
    pub days: u32,
    pub main_red: u32,
    pub reverts: u32,
    pub failing_heads: u32,
    pub pushed_heads: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BrakeState {
    pub on: bool,
    pub reasons: Vec<String>,
}

/// The red-CI rate target of a mode in percent; the gatekeeper has none.
pub fn red_ci_target(m: ModeName) -> Option<u32> {
    match m {
        ModeName::Gatekeeper => None,
        ModeName::Steady => Some(30),
        ModeName::Booster => Some(20),
        ModeName::Autonomous => Some(15),
    }
}

/// Recomputed on every read; nothing is stored, so nothing can lift it.
pub fn brake(mode: ModeName, w: &BrakeWindow) -> BrakeState {
    let mut reasons = Vec::new();
    let d = w.days;
    if w.main_red >= BRAKE_MAIN_RED {
        reasons.push(format!("main went red {} times in {d} days", w.main_red));
    }
    if w.reverts >= BRAKE_REVERTS {
        reasons.push(format!("{} PRs were reverted in {d} days", w.reverts));
    }
    if let Some(t) = red_ci_target(mode)
        && w.pushed_heads > 0
        && u64::from(w.failing_heads) * 100
            >= u64::from(t + BRAKE_RATE_MARGIN_PCT) * u64::from(w.pushed_heads)
    {
        reasons.push(format!(
            "red-CI rate {}/{} is {BRAKE_RATE_MARGIN_PCT} points over the {t} % target of {}",
            w.failing_heads,
            w.pushed_heads,
            mode.as_str()
        ));
    }
    BrakeState {
        on: !reasons.is_empty(),
        reasons,
    }
}
