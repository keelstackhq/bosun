//! Tool pins decide what CI installs, so `pins.toml` (hull's root file and bosun's
//! `.bosun/pins.toml`) is R-ci in the built-in rules, bosun's own rules and the channel's.

use bosun::core::risk::Rules;
use std::fs;
use std::path::Path;

fn r_ci_matches(rules: &Rules, path: &str) -> bool {
    let r = rules.rules.iter().find(|r| r.id == "R-ci").unwrap();
    r.paths.iter().any(|g| g.matches(path))
}

fn read(rel: &str) -> String {
    fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join(rel)).unwrap()
}

#[test]
fn pins_are_r_ci_in_the_built_in_rules() {
    let rules = Rules::defaults();
    for p in ["pins.toml", ".bosun/pins.toml"] {
        assert!(r_ci_matches(&rules, p), "{p}");
    }
}

#[test]
fn pins_are_r_ci_in_bosuns_own_rules() {
    let rules = Rules::parse(&read(".bosun/risk.toml")).unwrap();
    for p in ["pins.toml", ".bosun/pins.toml"] {
        assert!(r_ci_matches(&rules, p), "{p}");
    }
}

#[test]
fn pins_are_r_ci_in_the_channel_rules() {
    let text = read("channel/pieces/core/files/.bosun/risk.toml.jinja");
    let line = text
        .lines()
        .skip_while(|l| *l != "id = \"R-ci\"")
        .find(|l| l.starts_with("paths = "))
        .unwrap();
    assert!(
        line.contains("\"pins.toml\"") && line.contains("\".bosun/pins.toml\""),
        "{line}"
    );
}
