//! The top-level `intake` key of `.bosun/bosun.toml` (zero-touch onboarding, spec section 3).

use bosun::config::Config;

#[test]
fn intake_defaults_to_false() {
    assert!(!Config::default().intake);
    assert!(
        !Config::parse("project = \"app\"\n", "bosun.toml")
            .unwrap()
            .intake
    );
}

#[test]
fn intake_true_and_false_parse() {
    for (text, want) in [("intake = true\n", true), ("intake = false\n", false)] {
        assert_eq!(Config::parse(text, "bosun.toml").unwrap().intake, want);
    }
}

#[test]
fn intake_of_the_wrong_type_is_an_error() {
    for text in ["intake = \"yes\"\n", "intake = 1\n", "[intake]\nx = 1\n"] {
        let e = Config::parse(text, ".bosun/bosun.toml").unwrap_err();
        assert!(e.message.contains("bosun.toml"), "{text}: {}", e.message);
    }
}

#[test]
fn unknown_keys_stay_an_error() {
    assert!(Config::parse("intakes = true\n", "bosun.toml").is_err());
}
