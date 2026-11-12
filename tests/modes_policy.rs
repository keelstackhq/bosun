//! Modes PR 1b: preset policy, weaken findings for [mode], R-gov in the D-017 deny set, R-gate.

use bosun::config::Config;
use bosun::core::approve::Policy;
use bosun::core::risk::{Approval, Match, autonomous_excluded};
use bosun::core::weaken::text_findings;

const C: &str = ".bosun/bosun.toml";

const STEADY: &str = "[mode]\nname = \"steady\"\n";

fn w(base: &str, head: &str) -> Vec<String> {
    text_findings(C, Some(base), Some(head))
}

#[test]
fn an_explicit_policy_wins_over_the_preset() {
    let c = Config::parse(
        "[mode]\nname = \"booster\"\n[approvals]\npolicy = \"owner\"\n",
        "f.toml",
    )
    .unwrap();
    assert_eq!(c.approvals.policy(), Policy::Owner);
    assert_eq!(c.approvals.policy_source(), "explicit");
    let c = Config::parse("project = \"x\"\n", "f.toml").unwrap();
    assert_eq!(c.approvals.policy_source(), "default");
    assert!(c.approvals.policy.is_none());
}

#[test]
fn a_lighter_mode_is_a_weaken_finding_and_a_tighter_one_is_not() {
    let g = "[mode]\nname = \"gatekeeper\"\n";
    let b = "[mode]\nname = \"booster\"\n";
    assert_eq!(w("", b), [format!("{C}: mode gatekeeper → booster")]);
    assert_eq!(w(g, b), [format!("{C}: mode gatekeeper → booster")]);
    assert!(w(b, g).is_empty());
    assert!(w(b, "").is_empty());
    let off = "[mode]\nname = \"steady\"\nbrake = \"off\"\n";
    assert_eq!(w(b, off), [format!("{C}: mode.brake on → off")]);
    // Restating the tighter mode's own preset is no finding.
    assert!(w(STEADY, "[mode]\nname = \"gatekeeper\"\nbrake = \"off\"\n").is_empty());
    // A base [mode] that does not parse counts as gatekeeper.
    assert_eq!(
        w("[mode]\nname = \"x\"\n", b),
        [format!("{C}: mode gatekeeper → booster")]
    );
    assert!(w("[mode]\nname = \"steady\"\n", b).len() == 1);
}

#[test]
fn loosening_overrides_are_findings_and_helper_knobs_are_not() {
    let s = "[mode]\nname = \"steady\"\n";
    assert_eq!(
        w(s, "[mode]\nname = \"steady\"\npush_gates = \"ready\"\n"),
        [format!("{C}: mode.push_gates push → ready")]
    );
    assert_eq!(
        w(s, "[mode]\nname = \"steady\"\nbrake = \"off\"\n"),
        [format!("{C}: mode.brake on → off")]
    );
    // Helper knobs change who acts, not what passes (the file is R-gov anyway).
    assert!(
        w(
            "",
            "[mode]\nheal_main = \"revert-auto\"\nspawn_auto = \"on\"\n"
        )
        .is_empty()
    );
    assert!(w(s, "[mode]\nname = \"steady\"\nheal_pr = \"spawn\"\n").is_empty());
}

#[test]
fn an_unparseable_mode_at_head_is_a_finding() {
    let f = w("", "[mode]\nname = \"turbo\"\n");
    assert_eq!(f, [format!("{C}: mode parses → (unparseable)")]);
}

#[test]
fn an_allowlisted_r_gov_file_goes_to_the_owner() {
    let m = Match {
        rule: "R-gov".into(),
        why: String::new(),
        approval: Approval::Owner,
        evidence: vec![],
    };
    let files = [("src/core/session_rules.rs", None)];
    assert!(autonomous_excluded(&files, &[]).is_empty());
    assert_eq!(autonomous_excluded(&files, &[m]), ["R-gov"]);
}

#[test]
fn code_that_acts_without_the_owner_is_r_gate_in_both_rule_files() {
    let files = [
        "src/core/mode.rs",
        "src/core/digest.rs",
        "src/cli/digest.rs",
        "src/core/heal.rs",
        "src/cli/heal.rs",
        "src/core/metrics.rs",
        "src/cli/metrics.rs",
        "src/core/spawn.rs",
        "src/cli/spawn.rs",
        "src/core/wave.rs",
        "src/cli/wave.rs",
    ];
    for text in [
        include_str!("../src/core/risk_default.toml"),
        include_str!("../.bosun/risk.toml"),
    ] {
        let t: toml::Table = text.parse().unwrap();
        let rules = t["rule"].as_array().unwrap();
        let gate = rules
            .iter()
            .find(|r| r["id"].as_str() == Some("R-gate"))
            .unwrap();
        let paths: Vec<&str> = (gate["paths"].as_array().unwrap().iter())
            .map(|p| p.as_str().unwrap())
            .collect();
        for f in files {
            assert!(paths.contains(&f), "{f}");
            let dir = format!("{}/**", f.trim_end_matches(".rs"));
            assert!(paths.contains(&dir.as_str()), "{dir}");
        }
    }
}
