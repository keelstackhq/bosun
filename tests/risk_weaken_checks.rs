//! `[checks]` in .bosun/bosun.toml is a trust setting: any change is R-weaken evidence (spec §15).

use bosun::core::weaken::text_findings;

const C: &str = ".bosun/bosun.toml";
const DUAL: &str = "[checks]\nmode = \"dual\"\napp_id = 4242\napp_slug = \"gate\"\n";

#[test]
fn moving_checks_back_to_statuses_is_found() {
    let checks = DUAL.replace("dual", "checks");
    assert_eq!(
        text_findings(C, Some(&checks), Some(DUAL)),
        [".bosun/bosun.toml: checks.mode checks → dual"]
    );
    let none = text_findings(C, Some(DUAL), Some(""));
    assert!(
        none.contains(&".bosun/bosun.toml: checks.mode dual → (removed)".to_string()),
        "{none:?}"
    );
}

#[test]
fn another_app_is_found() {
    let other = DUAL.replace("4242", "7");
    assert_eq!(
        text_findings(C, Some(DUAL), Some(&other)),
        [".bosun/bosun.toml: checks.app_id 4242 → 7"]
    );
}

#[test]
fn adding_checks_is_found_and_an_unchanged_table_is_not() {
    assert!(!text_findings(C, Some(""), Some(DUAL)).is_empty());
    assert!(text_findings(C, Some(DUAL), Some(DUAL)).is_empty());
}
