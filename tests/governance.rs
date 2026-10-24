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
fn shipped_rules_have_the_decided_approval_class() {
    use bosun::core::risk::Approval;
    for rules in [
        Rules::parse(&read(".bosun/risk.toml")).unwrap(),
        Rules::defaults(),
    ] {
        let ids: Vec<&str> = rules.rules.iter().map(|r| r.id.as_str()).collect();
        assert_eq!(
            ids,
            [
                "R-gov",
                "R-ci",
                "R-gate",
                "R-ci-review",
                "R-secrets",
                "R-deps",
                "R-prod-write"
            ]
        );
        for r in &rules.rules {
            let want = if r.id == "R-ci-review" {
                Approval::TwoVerdicts
            } else {
                Approval::Owner
            };
            assert_eq!(r.approval, want, "{}", r.id);
        }
    }
}

#[test]
fn builtin_rules_equal_the_repo_rules() {
    let repo = Rules::parse(&read(".bosun/risk.toml")).unwrap();
    let built = Rules::defaults();
    assert_eq!(repo.rules.len(), built.rules.len());
    for (a, b) in repo.rules.iter().zip(&built.rules) {
        assert_eq!(a.id, b.id);
        assert_eq!(a.why, b.why, "{}", a.id);
        let paths = |r: &bosun::core::risk::Rule| {
            r.paths
                .iter()
                .map(|g| g.as_str().to_string())
                .collect::<Vec<_>>()
        };
        assert_eq!(paths(a), paths(b), "{}", a.id);
        assert_eq!(
            a.content.as_ref().map(|c| c.as_str()),
            b.content.as_ref().map(|c| c.as_str()),
            "{}",
            a.id
        );
        assert_eq!(a.new_dependency, b.new_dependency, "{}", a.id);
        assert_eq!(a.capability, b.capability, "{}", a.id);
        assert_eq!(a.approval, b.approval, "{}", a.id);
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
