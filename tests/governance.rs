//! The governance files bosun runs on itself parse, match the spec defaults, and the hook
//! commands in .claude/settings.json are real bosun commands.

mod common;

use bosun::config::{Config, Limits};
use bosun::core::risk::{Approval, Rules, parse_owners};
use common::bosun;
use serde_json::{Value, json};
use std::fs;
use std::path::Path;

fn read(path: &str) -> String {
    fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join(path)).unwrap()
}

/// The committed copy of `path` (HEAD), else the working-tree file. CI puts the base's `.bosun/`
/// over the checkout before the gates, so a PR that changes the built-in rules and
/// `.bosun/risk.toml` together is judged on what it commits.
fn committed(path: &str) -> String {
    std::process::Command::new("git")
        .args(["show", &format!("HEAD:{path}")])
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .output()
        .ok()
        .filter(|o| o.status.success())
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .unwrap_or_else(|| read(path))
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

/// The only rules bosun's own .bosun/risk.toml may delegate (it does from the 0.2.1 pin on).
/// The tests accept owner or delegate for them: CI runs the gates with the base commit's
/// .bosun/, so a PR that changes the class is tested against the old file until it merges.
const REPO_DELEGATES: &[&str] = &["R-ci", "R-gate", "R-deps"];

/// risk.yml judges every PR with the release pinned in .bosun/version, so the rules on main must
/// parse with it: `approval = "delegate"` needs 0.2.1 or later.
#[test]
fn repo_rules_parse_with_the_pinned_release() {
    let pin = read(".bosun/version").trim().to_string();
    if pin == "unreleased" {
        return; // risk.yml builds main itself
    }
    let parts: Vec<u32> = pin.split('.').map(|p| p.parse().unwrap()).collect();
    let knows_delegate = parts.as_slice() >= [0, 2, 1].as_slice();
    let rules = Rules::parse(&read(".bosun/risk.toml")).unwrap();
    for r in &rules.rules {
        assert!(
            knows_delegate || r.approval != Approval::Delegate,
            "{}: `approval = \"delegate\"` breaks risk-check under the pinned bosun {pin}",
            r.id
        );
    }
}

#[test]
fn risk_rules_cover_the_defaults() {
    let rules = Rules::parse(&read(".bosun/risk.toml")).unwrap();
    let ids = |r: &Rules| r.rules.iter().map(|x| x.id.clone()).collect::<Vec<_>>();
    assert_eq!(ids(&rules), ids(&Rules::defaults()));
}

#[test]
fn shipped_rules_have_the_decided_approval_class() {
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
    }
    // The built-in rules never delegate: a repo opts in in its own .bosun/risk.toml.
    let class = |r: &bosun::core::risk::Rule, repo: bool| match r.id.as_str() {
        "R-ci-review" => Approval::TwoVerdicts,
        id if repo && REPO_DELEGATES.contains(&id) && r.approval == Approval::Delegate => {
            Approval::Delegate
        }
        _ => Approval::Owner,
    };
    for r in &Rules::parse(&read(".bosun/risk.toml")).unwrap().rules {
        assert_eq!(r.approval, class(r, true), "{}", r.id);
    }
    for r in &Rules::defaults().rules {
        assert_eq!(r.approval, class(r, false), "{}", r.id);
    }
}

#[test]
fn builtin_rules_equal_the_repo_rules() {
    let repo = Rules::parse(&committed(".bosun/risk.toml")).unwrap();
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
        // The approval class may differ: only the repo file opts in to delegation.
        if a.approval != Approval::Delegate {
            assert_eq!(a.approval, b.approval, "{}", a.id);
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

#[test]
fn every_shipped_path_glob_matches_a_concrete_path_with_its_class() {
    for (rules, repo) in [
        (Rules::parse(&read(".bosun/risk.toml")).unwrap(), true),
        (Rules::defaults(), false),
    ] {
        for rule in &rules.rules {
            for g in &rule.paths {
                let text = g.as_str();
                let concrete = match text.strip_suffix("/**") {
                    Some(dir) if dir == ".github/workflows" => format!("{dir}/nested/x.yml"),
                    Some(dir) => format!("{dir}/inner.rs"),
                    None => text.to_string(),
                };
                assert!(g.matches(&concrete), "{} / {text}: {concrete}", rule.id);
                // No other rule may match it with a different class: the strictest wins.
                let want = match rule.id.as_str() {
                    "R-ci-review" => Approval::TwoVerdicts,
                    id if repo
                        && REPO_DELEGATES.contains(&id)
                        && rule.approval == Approval::Delegate =>
                    {
                        Approval::Delegate
                    }
                    _ => Approval::Owner,
                };
                assert_eq!(rule.approval, want, "{}", rule.id);
                for other in &rules.rules {
                    if other.paths.iter().any(|o| o.matches(&concrete)) && other.id != rule.id {
                        assert!(
                            matches!(other.approval, Approval::Owner | Approval::Delegate),
                            "{concrete}: {}",
                            other.id
                        );
                    }
                }
            }
        }
    }
}

#[test]
fn review_inputs_and_legacy_toolchain_name_are_covered() {
    let rules = Rules::parse(&read(".bosun/risk.toml")).unwrap();
    let id_of = |path: &str| {
        rules
            .rules
            .iter()
            .find(|r| r.paths.iter().any(|g| g.matches(path)))
            .map(|r| r.id.clone())
    };
    assert_eq!(id_of("docs/lessons.toml").as_deref(), Some("R-gov"));
    assert_eq!(id_of("rust-toolchain").as_deref(), Some("R-ci"));
    assert_eq!(id_of("rust-toolchain.toml").as_deref(), Some("R-ci"));
    assert_eq!(id_of("Cargo.toml").as_deref(), Some("R-gate"));
    for f in [
        "src/cli/gate.rs",
        "src/config.rs",
        "src/cli/review_ci.rs",
        "src/cli/review_pack.rs",
        "src/core/review.rs",
        "src/core/agreement.rs",
        "src/core/state.rs",
        "src/core/lease.rs",
        "src/ctx.rs",
        "src/registry.rs",
        "src/time.rs",
        "src/time/x.rs",
    ] {
        assert_eq!(id_of(f).as_deref(), Some("R-gate"), "{f}");
    }
}
