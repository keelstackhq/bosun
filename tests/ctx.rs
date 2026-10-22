//! Identity rules (repo, session, origin) and loading a Ctx from a checkout.

use bosun::ctx::{Ctx, Origin, RepoId, parse_remote_url, parse_repo, session_id};

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
