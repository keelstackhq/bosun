//! The governance files bosun runs on itself parse, match the spec defaults, and the hook
//! commands in .claude/settings.json are real bosun commands.

mod common;

use bosun::config::{Config, Limits};
use bosun::core::risk::{Rules, parse_owners};
use common::bosun;
use serde_json::{Value, json};
use std::fs;
use std::path::Path;

fn read(path: &str) -> String {
    fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join(path)).unwrap()
}

#[test]
fn bosun_toml_and_limits_parse() {
    let config = Config::parse(&read(".bosun/bosun.toml"), ".bosun/bosun.toml").unwrap();
    assert_eq!(config.project, "bosun");
    let limits = Limits::parse(&read(".bosun/limits.toml"), ".bosun/limits.toml").unwrap();
    let defaults = Limits::parse("", "defaults").unwrap();
    assert_eq!(
        limits, defaults,
        "limits.toml must equal the spec §5 defaults"
    );
}

#[test]
fn risk_rules_cover_the_defaults() {
    let rules = Rules::parse(&read(".bosun/risk.toml")).unwrap();
    let ids = |r: &Rules| r.rules.iter().map(|x| x.id.clone()).collect::<Vec<_>>();
    assert_eq!(ids(&rules), ids(&Rules::defaults()));
}

#[test]
fn shipped_rules_are_all_owner_rules() {
    // The pinned release must accept the `approval` key before a rule here may use
    // `approval = "two-verdicts"` (plan 4b): until then every shipped rule is an owner rule.
    use bosun::core::risk::Approval;
    for rules in [
        Rules::parse(&read(".bosun/risk.toml")).unwrap(),
        Rules::defaults(),
    ] {
        for r in &rules.rules {
            assert_eq!(r.approval, Approval::Owner, "{}", r.id);
        }
    }
}

#[test]
fn owners_name_the_repo_owner() {
    let owners = parse_owners(&read("governance/owners.toml")).unwrap();
    assert_eq!(owners[0].github, "urbanisierung");
    assert!(read(".github/CODEOWNERS").contains("/.bosun/ "));
}

#[test]
fn settings_hooks_run_bosun_hook_commands() {
    let settings: Value = serde_json::from_str(&read(".claude/settings.json")).unwrap();
    let mut commands = Vec::new();
    for event in ["SessionStart", "PreToolUse"] {
        for group in settings["hooks"][event].as_array().unwrap() {
            for h in group["hooks"].as_array().unwrap() {
                commands.push(h["command"].as_str().unwrap().to_string());
            }
        }
    }
    assert_eq!(
        commands,
        vec!["bosun hook session-start", "bosun hook pre-tool-use"]
    );
    // A merge through Bash is refused by the configured PreToolUse command.
    let input = json!({"session_id": "x", "tool_name": "Bash",
        "tool_input": {"command": "gh pr merge 3"}, "cwd": "/"});
    let args: Vec<&str> = commands[1].split_whitespace().skip(1).collect();
    let out = bosun()
        .current_dir(std::env::temp_dir())
        .args(&args)
        .write_stdin(input.to_string())
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(0));
    let v: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["hookSpecificOutput"]["permissionDecision"], "deny");
}
