mod common;

use common::{TOKEN_CANARY, bosun, one_object};

/// Minimal valid arguments for every command (S1 Done when).
const CALLS: [&[&str]; 25] = [
    &["join"],
    &["claim", "4"],
    &["heartbeat"],
    &["release", "4"],
    &["reap"],
    &["spawn", "7"],
    &["wave", "plan", "7"],
    &["wait", "--pr", "3"],
    &["msg", "session_x", "hello"],
    &["gate", "test"],
    &["risk-check", "3"],
    &["test-integrity"],
    &["review-pack", "3"],
    &["verdict", "post"],
    &["pr", "4"],
    &["ci-log", "3"],
    &["status"],
    &["ask", "4", "why?"],
    &["lesson", "add", "text"],
    &["lesson", "hit", "L-001"],
    &["hook", "pre-tool-use"],
    &["hook", "session-start"],
    &["doctor"],
    &["budget", "record", "4", "--provider", "generic"],
<<<<<<< HEAD
    &["revert", "3"],
=======
    &["fail", "4", "--signature", "E0308"],
>>>>>>> origin/main
];

#[test]
fn help_lists_every_command() {
    let out = bosun().arg("--help").output().unwrap();
    assert!(out.status.success());
    let help = String::from_utf8(out.stdout).unwrap();
    for name in bosun::cli::COMMANDS {
        assert!(
            help.contains(&format!("  {name} ")),
            "missing {name} in:\n{help}"
        );
    }
}

#[test]
fn every_command_prints_one_envelope() {
    // An empty directory, not this repo: `gate test` must not run this crate's own tests.
    let empty = tempfile::tempdir().unwrap();
    for call in CALLS {
        if call[0] == "hook" {
            continue; // hooks print the Claude Code hook protocol, not an envelope (cli_hook.rs)
        }
        let out = bosun()
            .current_dir(empty.path())
            .args(call)
            .write_stdin("")
            .output()
            .unwrap();
        let v = one_object(&out.stdout);
        for key in [
            "version",
            "command",
            "ok",
            "exit_code",
            "data",
            "error",
            "next",
        ] {
            assert!(v.get(key).is_some(), "{call:?}: no {key} in {v}");
        }
        assert_eq!(v["version"], 1);
        assert_eq!(
            i64::from(out.status.code().unwrap()),
            v["exit_code"].as_i64().unwrap(),
            "{call:?}"
        );
        if v["ok"] == false {
            assert!(
                !v["error"]["fix"].as_str().unwrap().trim().is_empty(),
                "{call:?}"
            );
        }
        let all = format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
        assert!(!all.contains(TOKEN_CANARY), "{call:?} printed the token");
    }
}

#[test]
fn bad_flag_is_usage_exit_2_with_envelope() {
    let out = bosun().args(["claim", "--nope"]).output().unwrap();
    assert_eq!(out.status.code(), Some(2));
    let v = one_object(&out.stdout);
    assert_eq!(v["command"], "claim");
    assert_eq!(v["exit_code"], 2);
    assert_eq!(v["error"]["kind"], "usage");
    assert!(
        v["error"]["fix"]
            .as_str()
            .unwrap()
            .contains("bosun claim --help")
    );
}

#[test]
fn unknown_command_is_usage() {
    let out = bosun().arg("merge").output().unwrap();
    assert_eq!(out.status.code(), Some(2));
    let v = one_object(&out.stdout);
    assert_eq!(v["command"], "unknown");
    assert_eq!(v["error"]["kind"], "usage");
}

#[test]
fn version_exits_0() {
    let out = bosun().arg("--version").output().unwrap();
    assert_eq!(out.status.code(), Some(0));
    assert!(String::from_utf8(out.stdout).unwrap().starts_with("bosun "));
}

#[test]
fn json_flag_is_accepted() {
    let out = bosun().args(["--json", "status"]).output().unwrap();
    one_object(&out.stdout);
}

#[test]
fn no_command_is_usage_with_envelope() {
    for args in [&[][..], &["wave"][..]] {
        let out = bosun().args(args).output().unwrap();
        assert_eq!(out.status.code(), Some(2), "{args:?}");
        let v = one_object(&out.stdout);
        assert_eq!(v["error"]["kind"], "usage");
    }
}
