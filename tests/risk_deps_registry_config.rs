//! Package-source config redirects where every dependency comes from, so any change to it is
//! R-deps (spec §8): Cargo's `.cargo/config.toml` (`[source]` replace-with, `[registries]`) and
//! npm, pnpm and yarn's `.npmrc` and `.yarnrc*` (registry and scope lines).
use bosun::core::risk::{ChangedFile, Rules, evaluate};
use std::path::Path;

const CONFIGS: [&str; 7] = [
    ".cargo/config.toml",
    ".cargo/config",
    "crates/app/.cargo/config.toml",
    ".npmrc",
    "web/.npmrc",
    ".yarnrc",
    "web/.yarnrc.yml",
];

/// The committed text: CI puts the base's `.bosun/` over the checkout before the gates, so the
/// file on disk is the base's there (tests/governance.rs reads it the same way).
fn committed(rel: &str) -> String {
    let root = env!("CARGO_MANIFEST_DIR");
    std::process::Command::new("git")
        .args(["show", &format!("HEAD:{rel}")])
        .current_dir(root)
        .output()
        .ok()
        .filter(|o| o.status.success())
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .unwrap_or_else(|| std::fs::read_to_string(Path::new(root).join(rel)).unwrap())
}

fn rules_of(rel: &str) -> Rules {
    let text = committed(rel);
    // The channel template: render the jinja tags away (every piece on).
    let text: String = text
        .split("{%")
        .enumerate()
        .map(|(i, part)| {
            if i == 0 {
                part
            } else {
                part.split_once("%}").map_or(part, |(_, rest)| rest)
            }
        })
        .collect();
    Rules::parse(&text).unwrap()
}

fn deps_paths(rules: &Rules) -> Vec<String> {
    CONFIGS
        .iter()
        .filter(|path| {
            let f = ChangedFile {
                path: path.to_string(),
                ..Default::default()
            };
            evaluate(rules, &[f], &[], &[])
                .iter()
                .any(|m| m.rule == "R-deps")
        })
        .map(|p| p.to_string())
        .collect()
}

#[test]
fn every_rule_set_counts_package_source_config_as_r_deps() {
    let all: Vec<String> = CONFIGS.iter().map(|p| p.to_string()).collect();
    assert_eq!(deps_paths(&Rules::defaults()), all, "built-in rules");
    assert_eq!(
        deps_paths(&rules_of(".bosun/risk.toml")),
        all,
        ".bosun/risk.toml"
    );
    assert_eq!(
        deps_paths(&rules_of(
            "channel/pieces/core/files/.bosun/risk.toml.jinja"
        )),
        all,
        "the channel's rules"
    );
    // Any case: a case-insensitive file system reads `.NPMRC` as `.npmrc`.
    let f = ChangedFile {
        path: "Web/.NPMRC".into(),
        ..Default::default()
    };
    assert!(
        evaluate(&Rules::defaults(), &[f], &[], &[])
            .iter()
            .any(|m| m.rule == "R-deps")
    );
}

#[test]
fn other_dot_files_are_not_r_deps() {
    for path in [
        ".cargo/credentials.toml.example",
        "npmrc.md",
        ".yarn/releases/x.cjs",
        "docs/.npmrc.txt",
    ] {
        let f = ChangedFile {
            path: path.into(),
            ..Default::default()
        };
        let m = evaluate(&Rules::defaults(), &[f], &[], &[]);
        assert!(!m.iter().any(|m| m.rule == "R-deps"), "{path}");
    }
}
