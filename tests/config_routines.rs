use bosun::config::Config;

const F: &str = ".bosun/bosun.toml";

#[test]
fn the_janitor_is_off_by_default_and_hourly_when_on() {
    let c = Config::parse("", F).unwrap();
    assert!(!c.routines.janitor);
    assert_eq!(c.routines.janitor_cron, "0 * * * *");
    let c = Config::parse("[routines]\njanitor = true\n", F).unwrap();
    assert!(c.routines.janitor);
    assert_eq!(c.routines.janitor_cron, "0 * * * *");
}

#[test]
fn bad_routines_keys_are_errors() {
    for t in [
        "[routines]\njanitor = \"yes\"\n",
        "[routines]\nbogus = 1\n",
        "[routines]\njanitor_cron = \"\"\n",
        "[routines]\njanitor_cron = \"0 0 * * * *\"\n",
    ] {
        let e = Config::parse(t, F).unwrap_err();
        assert!(!e.fix.is_empty(), "{t}");
    }
}
