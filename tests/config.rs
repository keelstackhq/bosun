//! Task 2 (S1b): config files, identity rules, time. Pure parsing functions take plain arguments.

use bosun::config::{Config, Limits, ReviewMode};
use bosun::ctx::{Ctx, Origin, RepoId, parse_remote_url, parse_repo, session_id};
use bosun::time::{Timestamp, parse_rfc3339};

fn repo(o: &str, n: &str) -> Option<RepoId> {
    Some(RepoId {
        owner: o.into(),
        name: n.into(),
    })
}

#[test]
fn remote_url_forms() {
    for url in [
        "https://github.com/keelstackhq/bosun",
        "https://github.com/keelstackhq/bosun.git",
        "https://x-access-token:abc@github.com/keelstackhq/bosun.git",
        "git@github.com:keelstackhq/bosun.git",
        "git@github.com:keelstackhq/bosun",
        "ssh://git@github.com/keelstackhq/bosun.git",
        "ssh://git@github.com:22/keelstackhq/bosun.git\n",
    ] {
        assert_eq!(parse_remote_url(url), repo("keelstackhq", "bosun"), "{url}");
    }
    for bad in [
        "",
        "not a url",
        "https://github.com/onlyowner",
        "/local/path",
    ] {
        assert_eq!(parse_remote_url(bad), None, "{bad}");
    }
    assert_eq!(parse_repo("o/r"), repo("o", "r"));
    assert_eq!(parse_repo("o"), None);
    assert_eq!(parse_repo("o/r/x"), None);
}

#[test]
fn session_rules() {
    assert_eq!(session_id(Some("mine"), Some("cse_1"), "h"), "mine");
    assert_eq!(session_id(None, Some("cse_01AbC"), "h"), "session_01AbC");
    assert_eq!(session_id(None, Some("session_x"), "h"), "session_x");
    assert_eq!(session_id(Some(" "), None, "box"), "local-box");
    assert_eq!(session_id(None, None, "box"), "local-box");
}

#[test]
fn origin_rules() {
    assert_eq!(Origin::resolve(None, None).unwrap(), Origin::Local);
    assert_eq!(Origin::resolve(None, Some("1")).unwrap(), Origin::Spawned);
    assert_eq!(Origin::resolve(None, Some("0")).unwrap(), Origin::Local);
    assert_eq!(
        Origin::resolve(Some("slack"), Some("1")).unwrap(),
        Origin::Slack
    );
    assert_eq!(
        Origin::resolve(Some("routine"), None).unwrap(),
        Origin::Routine
    );
    let err = Origin::resolve(Some("robot"), None).unwrap_err();
    assert!(err.message.contains("BOSUN_ORIGIN") && err.message.contains("robot"));
    assert_eq!(
        serde_json::to_string(&Origin::Spawned).unwrap(),
        "\"spawned\""
    );
}

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

#[test]
fn ctx_loads_config_files_from_git_toplevel() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    let git = |args: &[&str]| {
        let s = std::process::Command::new("git")
            .arg("-C")
            .arg(root)
            .args(args)
            .status()
            .unwrap();
        assert!(s.success());
    };
    git(&["init", "-q"]);
    git(&["remote", "add", "origin", "git@github.com:acme/widgets.git"]);
    std::fs::create_dir_all(root.join(".bosun")).unwrap();
    std::fs::create_dir_all(root.join("sub")).unwrap();
    std::fs::write(
        root.join(".bosun/limits.toml"),
        "[waves]\nmax_parallel = 2\n",
    )
    .unwrap();
    // No bosun.toml: the project defaults to the repo name.
    let ctx = Ctx::load(Some(&root.join("sub"))).unwrap();
    assert_eq!(ctx.limits.waves.max_parallel, 2);
    assert_eq!(ctx.limits.sessions.max_depth, 2);
    if std::env::var_os("BOSUN_REPO").is_none() {
        assert_eq!(ctx.repo, repo("acme", "widgets").unwrap());
        assert_eq!(ctx.config.project, "widgets");
    }
}

#[test]
fn rfc3339_parse_and_display() {
    let t = parse_rfc3339("2026-10-22T12:30:45Z").unwrap();
    assert_eq!(t.to_string(), "2026-10-22T12:30:45Z");
    assert_eq!(parse_rfc3339("1970-01-01T00:00:00Z"), Some(Timestamp(0)));
    // Fraction ignored; offsets applied.
    assert_eq!(parse_rfc3339("2026-10-22T12:30:45.987Z"), Some(t));
    assert_eq!(parse_rfc3339("2026-10-22T14:30:45+02:00"), Some(t));
    assert_eq!(parse_rfc3339("2026-10-22T07:00:45-05:30"), Some(t));
    // Leap years: 2024 and 2000 have Feb 29; 1900 and 2100 do not.
    for s in [
        "2024-02-29T00:00:00Z",
        "2000-02-29T23:59:59Z",
        "2028-12-31T23:59:59Z",
        "1999-03-01T00:00:00Z",
    ] {
        assert_eq!(parse_rfc3339(s).unwrap().to_string(), s);
    }
    let secs = |s: &str| parse_rfc3339(s).unwrap().0;
    assert_eq!(
        secs("2024-03-01T00:00:00Z") - secs("2024-02-28T00:00:00Z"),
        2 * 86_400
    );
    assert_eq!(
        secs("2023-03-01T00:00:00Z") - secs("2023-02-28T00:00:00Z"),
        86_400
    );
    assert_eq!(parse_rfc3339("2023-02-29T00:00:00Z"), None);
    assert_eq!(parse_rfc3339("1900-02-29T00:00:00Z"), None);
    assert_eq!(parse_rfc3339("2100-02-29T00:00:00Z"), None);
    for bad in [
        "",
        "2026-10-22",
        "yesterday",
        "2026-13-01T00:00:00Z",
        "2026-10-22T25:00:00Z",
        "2026-10-22T10:00:00+0200",
    ] {
        assert_eq!(parse_rfc3339(bad), None, "{bad}");
    }
}

#[test]
fn day_start_and_plus_secs() {
    let t = parse_rfc3339("2026-10-22T23:59:59Z").unwrap();
    assert_eq!(t.day_start().to_string(), "2026-10-22T00:00:00Z");
    assert_eq!(t.day_start().day_start(), t.day_start());
    assert_eq!(t.plus_secs(1).to_string(), "2026-10-23T00:00:00Z");
    assert_eq!(
        t.plus_secs(1).day_start().to_string(),
        "2026-10-23T00:00:00Z"
    );
    let before_epoch = Timestamp(-1);
    assert_eq!(before_epoch.to_string(), "1969-12-31T23:59:59Z");
    assert_eq!(before_epoch.day_start().to_string(), "1969-12-31T00:00:00Z");
}

#[test]
fn bosun_now_override_is_validated() {
    assert_eq!(
        Timestamp::resolve(Some("2026-10-22T00:00:00Z")).unwrap().0,
        1_792_627_200
    );
    assert!(Timestamp::resolve(Some("garbage")).is_err());
    assert!(Timestamp::resolve(None).unwrap().0 > 1_700_000_000);
}
