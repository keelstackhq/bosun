//! `[checks]` in .bosun/bosun.toml and the table form of hoist.toml `required_checks` (spec §15).

use bosun::config::{CheckMode, Config};
use bosun::core::checks::{CheckSource, RequiredCheck, hoist_required, hoist_required_sources};

fn parse(t: &str) -> Result<Config, bosun::output::BosunError> {
    Config::parse(t, ".bosun/bosun.toml")
}

#[test]
fn checks_default_to_statuses_with_no_app() {
    let c = parse("").unwrap();
    assert_eq!(c.checks.mode, CheckMode::Statuses);
    assert_eq!(
        (c.checks.app_id, c.checks.app_slug.as_deref()),
        (None, None)
    );
    let c = parse("[checks]\nmode = \"statuses\"\n").unwrap();
    assert_eq!(c.checks.mode, CheckMode::Statuses);
}

#[test]
fn dual_and_checks_read_the_app() {
    for (text, mode) in [("dual", CheckMode::Dual), ("checks", CheckMode::Checks)] {
        let c = parse(&format!(
            "[checks]\nmode = \"{text}\"\napp_id = 4242\napp_slug = \"keelstackhq-bosun-gate\"\n"
        ))
        .unwrap();
        assert_eq!(c.checks.mode, mode);
        assert_eq!(c.checks.app_id, Some(4242));
        assert_eq!(c.checks.app_slug.as_deref(), Some("keelstackhq-bosun-gate"));
        assert_eq!(mode.as_str(), text);
    }
}

#[test]
fn dual_or_checks_without_the_app_is_an_error_with_a_fix() {
    for text in [
        "[checks]\nmode = \"dual\"\n",
        "[checks]\nmode = \"checks\"\napp_id = 4242\n",
        "[checks]\nmode = \"checks\"\napp_slug = \"x\"\n",
        "[checks]\nmode = \"dual\"\napp_id = 0\napp_slug = \"x\"\n",
        "[checks]\nmode = \"dual\"\napp_id = 1\napp_slug = \"\"\n",
    ] {
        let e = parse(text).unwrap_err();
        assert!(e.message.contains("checks."), "{text}: {}", e.message);
        assert!(e.fix.contains(".bosun/bosun.toml"), "{text}: {}", e.fix);
    }
}

#[test]
fn unknown_mode_or_key_is_an_error() {
    assert!(parse("[checks]\nmode = \"neutral\"\n").is_err());
    assert!(parse("[checks]\nmode = \"statuses\"\nsource = \"x\"\n").is_err());
}

#[test]
fn hoist_required_reads_strings_and_tables() {
    let t = r#"[github]
required_checks = [
  "ci-ok",
  { context = "risk-check", app = "keelstackhq-bosun-gate" },
  { context = "main-open", integration_id = 15368 },
]
"#;
    assert_eq!(
        hoist_required(t).unwrap(),
        vec!["ci-ok", "risk-check", "main-open"]
    );
    assert_eq!(
        hoist_required_sources(t).unwrap(),
        vec![
            RequiredCheck {
                context: "ci-ok".into(),
                source: CheckSource::Any
            },
            RequiredCheck {
                context: "risk-check".into(),
                source: CheckSource::App("keelstackhq-bosun-gate".into())
            },
            RequiredCheck {
                context: "main-open".into(),
                source: CheckSource::Integration(15368)
            },
        ]
    );
}

#[test]
fn a_table_without_a_context_or_with_two_sources_is_not_read() {
    for bad in [
        "[github]\nrequired_checks = [{ app = \"x\" }]\n",
        "[github]\nrequired_checks = [{ context = \"a\", app = \"x\", integration_id = 1 }]\n",
        "[github]\nrequired_checks = [{ context = \"a\", app = 3 }]\n",
        "[github]\nrequired_checks = [{ context = \"a\", colour = \"x\" }]\n",
        "[github]\nrequired_checks = [1]\n",
    ] {
        assert_eq!(hoist_required(bad), None, "{bad}");
        assert_eq!(hoist_required_sources(bad), None, "{bad}");
    }
}

#[test]
fn table_entries_still_add_to_the_defaults() {
    let t = "[github]\nrequired_checks = [{ context = \"extra\", integration_id = 1 }]\n";
    let got = bosun::core::checks::required_checks(None, Some(t));
    assert_eq!(got.last().map(String::as_str), Some("extra"));
    assert_eq!(got.len(), 5);
}
