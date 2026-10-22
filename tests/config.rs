//! Task 2 (S1b): config files, identity rules, time. Pure parsing functions take plain arguments.

use bosun::config::{Config, Limits, ReviewMode};

#[test]
fn limits_defaults_match_spec_section_5() {
    let l = Limits::default();
    assert_eq!(
        (
            l.sessions.max_depth,
            l.sessions.max_live_sessions,
            l.sessions.max_children
        ),
        (2, 4, 3)
    );
    assert_eq!(l.waves.max_parallel, 6);
    assert_eq!(
        (
            l.turns.build,
            l.turns.build_cap,
            l.turns.review,
            l.turns.fix_round
        ),
        (100, 150, 40, 40)
    );
    assert_eq!(
        (l.models.read.as_str(), l.models.build.as_str()),
        ("haiku", "sonnet")
    );
    assert_eq!(
        (l.models.review.as_str(), l.models.hard.as_str()),
        ("sonnet", "opus")
    );
    assert_eq!((l.budget.daily_usd, l.budget.per_task_usd), (150.0, 25.0));
    assert_eq!(Limits::parse("", "x").unwrap(), l);
}

#[test]
fn partial_limits_keep_other_defaults() {
    let l = Limits::parse(
        "[sessions]\nmax_children = 1\n[budget]\ndaily_usd = 10\n",
        "l",
    )
    .unwrap();
    assert_eq!(l.sessions.max_children, 1);
    assert_eq!(l.sessions.max_depth, 2);
    assert_eq!(l.budget.daily_usd, 10.0);
    assert_eq!(l.budget.per_task_usd, 25.0);
    assert_eq!(l.turns.build_cap, 150);
}

#[test]
fn unknown_key_names_file_and_key() {
    let err = Limits::parse("[sessions]\nmax_deep = 3\n", ".bosun/limits.toml").unwrap_err();
    assert_eq!(err.exit.code(), 1);
    assert!(
        err.message.contains(".bosun/limits.toml"),
        "{}",
        err.message
    );
    assert!(err.message.contains("max_deep"), "{}", err.message);
    assert!(err.fix.contains(".bosun/limits.toml"));
    let err = Config::parse("projct = \"x\"\n", ".bosun/bosun.toml").unwrap_err();
    assert!(err.message.contains("projct") && err.message.contains(".bosun/bosun.toml"));
    let err = Config::parse("project = [", ".bosun/bosun.toml").unwrap_err();
    assert_eq!(err.exit.code(), 1);
    assert!(err.message.contains(".bosun/bosun.toml"));
}

#[test]
fn config_defaults_and_full_file() {
    let c = Config::parse("", "b").unwrap();
    assert_eq!(c.project, "");
    assert_eq!(
        (
            c.stack.len(),
            c.stack[0].path.as_str(),
            c.stack[0].profile.as_str()
        ),
        (1, ".", "rust")
    );
    assert_eq!(c.review.mode, ReviewMode::Subagent);
    assert!(c.slack.is_none() && c.prices.is_empty());
    let c = Config::parse(
        "project = \"p\"\n[[stack]]\npath = \"web\"\nprofile = \"ts-pnpm\"\n[review]\nmode = \"ci\"\n\
         [slack]\nchannel = \"C1\"\nspawners = \"owners\"\nmay_spawn = true\nmay_build = false\nnotify = [\"halt\"]\n\
         [prices.opus]\ninput_per_mtok = 15.0\noutput_per_mtok = 75.0\n",
        "b",
    )
    .unwrap();
    assert_eq!(
        (c.project.as_str(), c.stack[0].profile.as_str()),
        ("p", "ts-pnpm")
    );
    assert_eq!(c.review.mode, ReviewMode::Ci);
    let s = c.slack.unwrap();
    assert_eq!(
        (s.channel.as_str(), s.may_spawn, s.may_build, s.notify.len()),
        ("C1", true, false, 1)
    );
    assert_eq!(c.prices["opus"].output_per_mtok, 75.0);
}
