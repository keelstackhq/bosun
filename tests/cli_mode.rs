mod common;

use bosun::config::Config;
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
fn gatekeeper_with_an_override_parses() {
    let c = Config::parse(
        "[mode]\nname = \"gatekeeper\"\nheal_main = \"revert-pr\"\n",
        "f.toml",
    )
    .unwrap();
    let e = c.mode_effective();
    assert_eq!(e.mode_source, Source::Explicit);
    assert_eq!(
        e.knobs.heal_main,
        Set {
            value: HealMain::RevertPr,
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
fn lighter_modes_are_refused_until_1b() {
    for m in ["steady", "booster", "autonomous"] {
        let msg = err(&format!("[mode]\nname = \"{m}\"\n"));
        assert!(msg.contains("is not active yet"), "{msg}");
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
    for (args, pr) in [
        (vec!["heal"], "3a"),
        (vec!["digest"], "4"),
        (vec!["metrics"], "5"),
        (vec!["wave", "tick", "7"], "6"),
    ] {
        let out = bosun().args(&args).output().unwrap();
        assert_eq!(out.status.code(), Some(1), "{args:?}");
        let v = one_object(&out.stdout);
        let fix = v["error"]["fix"].as_str().unwrap();
        assert!(fix.contains(&format!("plan PR {pr}")), "{fix}");
    }
}
