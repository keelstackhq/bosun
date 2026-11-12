mod common;

use bosun::config::Config;
use bosun::core::approve::Policy;
use bosun::core::mode::*;
use common::{bosun, one_object};

fn err(text: &str) -> String {
    Config::parse(text, "f.toml").unwrap_err().message
}

#[test]
fn no_mode_table_is_gatekeeper_and_serializes_unchanged() {
    let c = Config::parse("project = \"x\"\n", "f.toml").unwrap();
    assert_eq!(c.mode_effective().mode, ModeName::Gatekeeper);
    assert!(!toml::to_string(&c).unwrap().contains("[mode]"));
}

#[test]
fn gatekeeper_with_a_default_override_parses() {
    let c = Config::parse(
        "[mode]\nname = \"gatekeeper\"\nheal_main = \"off\"\n",
        "f.toml",
    )
    .unwrap();
    let e = c.mode_effective();
    assert_eq!(e.mode_source, Source::Explicit);
    assert_eq!(
        e.knobs.heal_main,
        Set {
            value: HealMain::Off,
            source: Source::Explicit
        }
    );
    assert_eq!(
        e.knobs.heal_pr,
        Set {
            value: HealPr::Off,
            source: Source::Preset
        }
    );
}

#[test]
fn lighter_modes_parse_and_fill_the_policy() {
    for (m, policy, source) in [
        ("steady", Policy::Owner, "default"),
        ("booster", Policy::Autonomous, "preset"),
        ("autonomous", Policy::Autonomous, "preset"),
    ] {
        let c = Config::parse(&format!("[mode]\nname = \"{m}\"\n"), "f.toml").unwrap();
        assert_eq!(c.approvals.policy(), policy, "{m}");
        assert_eq!(c.approvals.policy_source(), source, "{m}");
    }
}

#[test]
fn policy_unknown_knob_and_bad_value_are_refused() {
    assert_eq!(
        err("[mode]\npolicy = \"autonomous\"\n"),
        "f.toml: policy is not a [mode] key"
    );
    assert!(err("[mode]\nname = \"gatekeeper\"\nfoo = 1\n").contains("foo"));
    assert!(err("[mode]\nbrake = \"maybe\"\n").contains("maybe"));
    assert!(err("[mode]\nname = \"turbo\"\n").contains("turbo"));
}

#[test]
fn bosun_mode_prints_knobs_offline() {
    let out = bosun().arg("mode").output().unwrap();
    assert_eq!(out.status.code(), Some(0));
    let v = one_object(&out.stdout);
    assert_eq!(v["command"], "mode");
    let d = &v["data"];
    assert_eq!(d["mode"], "gatekeeper");
    assert_eq!(d["mode_source"], "default");
    assert_eq!(d["knobs"]["push_gates"]["value"], "ready");
    assert_eq!(d["knobs"]["heal_main"]["source"], "default");
    assert!(d["brake"].is_null());
}

#[test]
fn stubs_exit_1_with_a_fix() {
    for (args, pr) in [(vec!["wave", "tick", "7"], "6")] {
        let out = bosun().args(&args).output().unwrap();
        assert_eq!(out.status.code(), Some(1), "{args:?}");
        let v = one_object(&out.stdout);
        let fix = v["error"]["fix"].as_str().unwrap();
        assert!(fix.contains(&format!("plan PR {pr}")), "{fix}");
    }
}

#[test]
fn a_non_default_override_parses() {
    for k in [
        "heal_main = \"revert-auto\"",
        "spawn_auto = \"on\"",
        "push_gates = \"push\"",
    ] {
        for name in ["", "name = \"gatekeeper\"\n"] {
            let c = Config::parse(&format!("[mode]\n{name}{k}\n"), "f.toml").unwrap();
            let v = serde_json::to_value(c.mode_effective()).unwrap();
            let knob = k.split(' ').next().unwrap();
            assert_eq!(v["knobs"][knob]["source"], "explicit", "{k}");
            assert_eq!(c.approvals.policy_source(), "default", "{k}");
        }
    }
}
