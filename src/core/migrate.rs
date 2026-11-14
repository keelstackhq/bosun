//! `bosun migrate` logic: the ladder step, the `[mode]` edit, the owner's steps and the PR body.

use super::mode::ModeName;
use super::pin::parse_semver;

/// The first bosun release that reads a lighter `[mode]`.
pub const MODE_MIN: &str = "0.14.0";

/// The ladder, tightest first (`Ord` order).
pub const LADDER: [ModeName; 4] = [
    ModeName::Gatekeeper,
    ModeName::Steady,
    ModeName::Booster,
    ModeName::Autonomous,
];

/// The mode named `s`.
pub fn parse_mode(s: &str) -> Option<ModeName> {
    LADDER.into_iter().find(|m| m.as_str() == s)
}

/// Whether `to` involves the owner less than `from` (a later rung).
pub fn is_lighter(from: ModeName, to: ModeName) -> bool {
    to > from
}

/// The mode one PR moves to: a lighter target is reached one rung at a time unless `direct`; a
/// tighter or equal target is taken at once.
pub fn step(current: ModeName, target: ModeName, direct: bool) -> ModeName {
    if is_lighter(current, target) && !direct {
        LADDER[current as usize + 1]
    } else {
        target
    }
}

/// The rungs a move from `from` to `step` passes over (later than `from`, earlier than `step`).
pub fn skipped_rungs(from: ModeName, step: ModeName) -> Vec<ModeName> {
    LADDER
        .into_iter()
        .filter(|m| *m > from && *m < step)
        .collect()
}

/// The mode a migration branch name carries: `bosun/migrate-<step>` or `bosun/migrate-<step>-<date>`.
pub fn branch_step(branch: &str) -> Option<&str> {
    branch
        .strip_prefix("bosun/migrate-")?
        .split('-')
        .next()
        .filter(|s| !s.is_empty())
}

/// `text` without the `user:password@` part of any URL in it (git echoes remotes in errors).
pub fn redact_credentials(text: &str) -> String {
    static RE: std::sync::LazyLock<regex::Regex> =
        std::sync::LazyLock::new(|| regex::Regex::new(r"://[^/\s@]+@").expect("static regex"));
    RE.replace_all(text, "://").into_owned()
}

/// `a < b` for two `X.Y.Z` versions; false when either is not one.
pub fn semver_lt(a: &str, b: &str) -> bool {
    matches!((parse_semver(a), parse_semver(b)), (Some(x), Some(y)) if x < y)
}

/// Whether the pinned version (`None`: no `.bosun/version`) cannot read a lighter `[mode]`.
pub fn below_min(version: Option<&str>) -> bool {
    version.is_none_or(|v| parse_semver(v).is_none() || semver_lt(v, MODE_MIN))
}

/// `text` with `[mode] name` set to `name`, every other byte kept.
pub fn set_mode_name(text: &str, name: ModeName) -> String {
    let line = format!("name = \"{}\"", name.as_str());
    let lines: Vec<&str> = text.split_inclusive('\n').collect();
    let header = |l: &str| {
        let t = l.trim();
        t == "[mode]"
            || t.strip_prefix("[mode]")
                .is_some_and(|r| r.trim_start().starts_with('#'))
    };
    if let Some(at) = lines.iter().position(|l| header(l)) {
        let end = lines[at + 1..]
            .iter()
            .position(|l| l.trim_start().starts_with('['))
            .map_or(lines.len(), |p| at + 1 + p);
        let is_name = |l: &str| {
            l.trim_start()
                .strip_prefix("name")
                .is_some_and(|r| r.trim_start().starts_with('='))
        };
        let mut out: Vec<String> = lines.iter().map(|l| l.to_string()).collect();
        match (at + 1..end).find(|&i| is_name(lines[i])) {
            Some(i) => {
                let nl = if lines[i].ends_with('\n') { "\n" } else { "" };
                out[i] = format!("{line}{nl}");
            }
            None => {
                if !out[at].ends_with('\n') {
                    out[at].push('\n');
                }
                out.insert(at + 1, format!("{line}\n"));
            }
        }
        return out.concat();
    }
    let mut s = text.to_string();
    if !s.is_empty() {
        if !s.ends_with('\n') {
            s.push('\n');
        }
        if !s.ends_with("\n\n") {
            s.push('\n');
        }
    }
    s + &format!("[mode]\n{line}\n")
}

/// What only the owner can do (Routines), each a human-only step on a computer.
pub fn owner_steps(to: ModeName) -> Vec<String> {
    let mut v = Vec::new();
    if to >= ModeName::Steady {
        v.push("[computer] Janitor Routine: add the line `Then run \\`bosun heal\\` and make only the calls in its \\`next\\`; report data.prs.`".to_string());
    }
    if to >= ModeName::Booster {
        v.push("[computer] Curator Routine: replace its prompt with quickstart.md §4".to_string());
    }
    if to >= ModeName::Steady {
        v.push("[computer] Digest: run `bosun digest` daily (orchestrator)".to_string());
    }
    v
}

/// What the PR body reports.
pub struct BodyIn<'a> {
    pub from: ModeName,
    pub to: ModeName,
    pub step: ModeName,
    pub direct: bool,
    pub version: Option<&'a str>,
    pub binary: &'a str,
    /// Already formatted: the pinned and live registry issue.
    pub registry: String,
    pub epics_unmarked: &'a [u64],
    pub hull: String,
    /// The baseline metrics (compact), or why there are none.
    pub baseline: Result<String, String>,
    pub files: &'a [String],
}

/// The migration PR body.
pub fn body(b: &BodyIn) -> String {
    let lighter = is_lighter(b.from, b.step);
    let list = |v: &[u64]| {
        if v.is_empty() {
            "none".to_string()
        } else {
            v.iter()
                .map(|n| format!("#{n}"))
                .collect::<Vec<_>>()
                .join(", ")
        }
    };
    let baseline = match &b.baseline {
        Ok(m) => format!("baseline (30 days): {m}"),
        Err(why) => format!("baseline: unavailable ({why})"),
    };
    let mut s = format!(
        "## Survey\n\n- mode: {} -> {} (target {})\n- pinned bosun: {} (this binary {})\n- registry: {}\n- unmarked epics: {}\n- hull lock: {}\n- {baseline}\n\n",
        b.from.as_str(),
        b.step.as_str(),
        b.to.as_str(),
        b.version.unwrap_or("none"),
        b.binary,
        b.registry,
        list(b.epics_unmarked),
        b.hull,
    );
    s += "## Changes\n\n";
    for f in b.files {
        s += &format!("- `{f}`\n");
    }
    s += "\n## Owner: one approval\n\nApprove this exact head through the chat form (repo, PR, head SHA). The PR is R-gov";
    if lighter {
        s += ", and R-weaken because the mode gets lighter";
    }
    s += ".";
    let skipped = skipped_rungs(b.from, b.step);
    if b.direct && lighter && !skipped.is_empty() {
        let names: Vec<&str> = skipped.iter().map(|m| m.as_str()).collect();
        s += &format!(
            "\n\nThe owner chose --direct (skips {}); recorded here.",
            names.join(", ")
        );
    }
    s += "\n\n## Owner-only steps left\n\n";
    let steps = owner_steps(b.step);
    if steps.is_empty() {
        s += "- none\n";
    }
    for t in steps {
        s += &format!("- {t}\n");
    }
    s += "\n## After merge\n\n`bosun migrate --finish`: bosun join, then bosun plan amend for each plan; both idempotent.\n\n";
    s += &format!("<!-- bosun:migrate {} -->\n", b.to.as_str());
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use ModeName::*;

    #[test]
    fn a_lighter_target_is_one_rung_unless_direct() {
        assert_eq!(step(Gatekeeper, Booster, false), Steady);
        assert_eq!(step(Gatekeeper, Autonomous, false), Steady);
        assert_eq!(step(Steady, Autonomous, false), Booster);
        assert_eq!(step(Gatekeeper, Booster, true), Booster);
        assert_eq!(step(Booster, Autonomous, false), Autonomous);
    }

    #[test]
    fn a_tighter_or_equal_target_is_taken_at_once() {
        assert_eq!(step(Autonomous, Gatekeeper, false), Gatekeeper);
        assert_eq!(step(Booster, Steady, false), Steady);
        assert_eq!(step(Steady, Steady, false), Steady);
        assert!(is_lighter(Steady, Booster) && !is_lighter(Booster, Steady));
        assert!(!is_lighter(Steady, Steady));
    }

    #[test]
    fn modes_parse_by_name() {
        assert_eq!(parse_mode("booster"), Some(Booster));
        assert_eq!(parse_mode("fast"), None);
    }

    #[test]
    fn semver_compares_numbers_not_text() {
        assert!(semver_lt("0.9.0", "0.14.0"));
        assert!(!semver_lt("0.14.0", "0.14.0"));
        assert!(!semver_lt("0.15.1", "0.14.0"));
        assert!(!semver_lt("x", "0.14.0"));
        assert!(below_min(None) && below_min(Some("0.13.9")) && below_min(Some("v0.14.0")));
        assert!(!below_min(Some("0.14.0")));
    }

    fn mode_of(text: &str) -> Option<ModeName> {
        Config::parse(text, "t").unwrap().mode.name
    }

    #[test]
    fn set_mode_name_replaces_the_name_line_and_keeps_the_rest() {
        let t = "project = \"w\"\n\n[mode]\nname = \"gatekeeper\" \nbrake = \"off\"\n\n[review]\nmodel = \"x\"\n";
        let r = set_mode_name(t, Steady);
        assert_eq!(
            r,
            "project = \"w\"\n\n[mode]\nname = \"steady\"\nbrake = \"off\"\n\n[review]\nmodel = \"x\"\n"
        );
        assert_eq!(mode_of(&r), Some(Steady));
    }

    #[test]
    fn set_mode_name_inserts_after_the_header_and_ignores_other_tables() {
        let t = "[review]\nmodel = \"not the mode\"\n\n[mode]\nbrake = \"on\"\n";
        let r = set_mode_name(t, Booster);
        assert_eq!(
            r,
            "[review]\nmodel = \"not the mode\"\n\n[mode]\nname = \"booster\"\nbrake = \"on\"\n"
        );
        assert_eq!(mode_of(&r), Some(Booster));
    }

    #[test]
    fn set_mode_name_appends_a_table_with_one_blank_line() {
        let r = set_mode_name("project = \"w\"", Steady);
        assert_eq!(r, "project = \"w\"\n\n[mode]\nname = \"steady\"\n");
        let r = set_mode_name("project = \"w\"\n", Steady);
        assert_eq!(r, "project = \"w\"\n\n[mode]\nname = \"steady\"\n");
        assert_eq!(set_mode_name("", Steady), "[mode]\nname = \"steady\"\n");
        assert_eq!(mode_of(&r), Some(Steady));
    }

    #[test]
    fn owner_steps_grow_with_the_mode() {
        assert!(owner_steps(Gatekeeper).is_empty());
        let s = owner_steps(Steady).join("\n");
        assert!(s.contains("Janitor") && s.contains("bosun digest") && !s.contains("Curator"));
        assert!(owner_steps(Booster).join("\n").contains("Curator Routine"));
    }

    fn input<'a>(direct: bool, step: ModeName, files: &'a [String]) -> BodyIn<'a> {
        BodyIn {
            from: Gatekeeper,
            to: Booster,
            step,
            direct,
            version: Some("0.14.0"),
            binary: "0.14.0",
            registry: "pinned #1, live #1".into(),
            epics_unmarked: &[4, 9],
            hull: "no".into(),
            baseline: Err("timed out".into()),
            files,
        }
    }

    #[test]
    fn the_body_has_every_section_and_the_marker() {
        let files = vec![".bosun/bosun.toml".to_string()];
        let b = body(&input(false, Steady, &files));
        for s in [
            "## Survey",
            "## Changes",
            "## Owner: one approval",
            "## Owner-only steps left",
            "## After merge",
            "`.bosun/bosun.toml`",
            "R-weaken",
            "baseline: unavailable (timed out)",
            "#4, #9",
            "bosun migrate --finish",
            "<!-- bosun:migrate booster -->",
        ] {
            assert!(b.contains(s), "{s}\n{b}");
        }
        assert!(!b.contains("--direct") && !b.contains("orchestrator command"));
        let d = body(&input(true, Booster, &files));
        assert!(d.contains("The owner chose --direct (skips steady)"), "{d}");
    }

    #[test]
    fn the_direct_note_needs_a_lighter_move_that_skips_a_rung() {
        let files = vec![".bosun/bosun.toml".to_string()];
        // --direct onto the next rung skips nothing.
        assert!(!body(&input(true, Steady, &files)).contains("--direct"));
        // --direct to a tighter mode is not lighter.
        let mut tight = input(true, Gatekeeper, &files);
        tight.from = Autonomous;
        tight.to = Gatekeeper;
        assert!(!body(&tight).contains("--direct"));
        let mut far = input(true, Autonomous, &files);
        far.to = Autonomous;
        assert!(body(&far).contains("(skips steady, booster)"));
    }

    #[test]
    fn skipped_rungs_branch_steps_and_credentials() {
        assert_eq!(skipped_rungs(Gatekeeper, Booster), vec![Steady]);
        assert!(skipped_rungs(Steady, Booster).is_empty());
        assert!(skipped_rungs(Booster, Gatekeeper).is_empty());
        assert_eq!(branch_step("bosun/migrate-steady"), Some("steady"));
        assert_eq!(branch_step("bosun/migrate-steady-20261022"), Some("steady"));
        assert_eq!(branch_step("bosun/migrate"), None);
        assert_eq!(branch_step("bosun/pin"), None);
        assert_eq!(
            redact_credentials("fatal: https://bob:s3cret@github.com/a/b.git denied"),
            "fatal: https://github.com/a/b.git denied"
        );
        assert_eq!(redact_credentials("ssh://git@host/x"), "ssh://host/x");
        assert_eq!(redact_credentials("no url, a@b"), "no url, a@b");
    }
}
