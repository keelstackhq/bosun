//! `.github/workflows/cargo-deny-npm.yml` republishes upstream cargo-deny's prebuilt Linux
//! binaries as `@keelstackhq/cargo-deny` (launcher) and two platform packages: a manual run in
//! environment `release`, the tarballs checked against upstream's `.sha256`, npm trusted
//! publishing with provenance, platform packages before the launcher.

use std::fs;
use std::path::Path;

fn read(rel: &str) -> String {
    fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join(rel)).unwrap()
}

fn yaml() -> String {
    read(".github/workflows/cargo-deny-npm.yml")
}

fn json(rel: &str) -> serde_json::Value {
    serde_json::from_str(&read(rel)).unwrap()
}

/// The `run:` scripts: the `run:` line and its indented block.
fn run_scripts(yaml: &str) -> Vec<String> {
    let lines: Vec<&str> = yaml.lines().collect();
    let mut out = Vec::new();
    for (i, line) in lines.iter().enumerate() {
        let t = line.trim_start();
        let Some(rest) = t.strip_prefix("run:").or_else(|| t.strip_prefix("- run:")) else {
            continue;
        };
        let indent = line.len() - t.len();
        let mut script = rest.to_string();
        for next in &lines[i + 1..] {
            let ni = next.len() - next.trim_start().len();
            if !next.trim().is_empty() && ni <= indent {
                break;
            }
            script.push('\n');
            script.push_str(next);
        }
        out.push(script);
    }
    out
}

#[test]
fn runs_only_by_hand_with_a_version_input() {
    let y = yaml();
    let on = &y[y.find("\non:\n").unwrap()..y.find("\npermissions:").unwrap()];
    let keys: Vec<&str> = on
        .lines()
        .filter(|l| l.starts_with("  ") && !l.starts_with("   "))
        .map(str::trim)
        .collect();
    assert_eq!(keys, ["workflow_dispatch:"], "{on}");
    assert!(on.contains("    inputs:\n      version:\n"), "{on}");
    assert!(on.contains("        required: true"), "{on}");
}

#[test]
fn publishes_from_main_in_environment_release_with_oidc_only() {
    let y = yaml();
    assert!(y.contains("\npermissions:\n  contents: read\n\n"), "{y}");
    let jobs = &y[y.find("\njobs:\n").unwrap()..];
    assert!(jobs.contains("\n    environment: release\n"), "{jobs}");
    assert!(
        jobs.contains("\n    permissions:\n      contents: read\n      id-token: write\n"),
        "{jobs}"
    );
    assert!(
        jobs.contains("if: github.ref == 'refs/heads/main'"),
        "{jobs}"
    );
    assert!(!y.contains("contents: write"), "{y}");
    assert!(!y.contains("secrets."), "no token: trusted publishing only");
    assert!(!y.contains("NODE_AUTH_TOKEN"), "{y}");
    // A publish run is never cancelled.
    assert!(y.contains("cancel-in-progress: false"), "{y}");
}

#[test]
fn every_action_is_pinned_by_sha() {
    let y = yaml();
    let uses: Vec<&str> = y
        .lines()
        .filter_map(|l| l.trim_start().strip_prefix("- uses: "))
        .collect();
    assert!(uses.len() >= 2, "{uses:?}");
    for u in uses {
        let at = u.split_whitespace().next().unwrap();
        let sha = at.rsplit_once('@').unwrap().1;
        assert!(
            sha.len() == 40 && sha.chars().all(|c| c.is_ascii_hexdigit()),
            "not pinned by SHA: {u}"
        );
    }
    assert!(y.contains("persist-credentials: false"), "{y}");
}

#[test]
fn downloads_both_musl_tarballs_and_checks_them_against_upstream_sha256() {
    let y = yaml();
    assert!(
        y.contains("https://github.com/EmbarkStudios/cargo-deny/releases/download/"),
        "{y}"
    );
    for target in ["x86_64-unknown-linux-musl", "aarch64-unknown-linux-musl"] {
        assert!(y.contains(target), "{target}");
    }
    assert!(y.contains("cargo-deny-${VERSION}-${target}.tar.gz"), "{y}");
    assert!(y.contains("\"$base/$tarball.sha256\""), "{y}");
    assert!(y.contains("sha256sum -c"), "{y}");
    // The version is a plain x.y.z before it reaches a URL or npm.
    assert!(y.contains("^[0-9]+\\.[0-9]+\\.[0-9]+$"), "{y}");
    // The x64 binary runs and reports the version it was published as.
    assert!(
        y.contains("test \"$got\" = \"cargo-deny ${VERSION}\""),
        "{y}"
    );
}

#[test]
fn sets_the_version_then_publishes_the_platform_packages_before_the_launcher() {
    let y = yaml();
    let publish: Vec<&str> = y
        .lines()
        .filter(|l| l.contains("npm publish"))
        .map(str::trim)
        .collect();
    assert_eq!(publish.len(), 1, "one publish loop: {publish:?}");
    assert!(
        publish[0].contains("--provenance --access public"),
        "{publish:?}"
    );
    let order = "for p in cargo-deny-linux-x64 cargo-deny-linux-arm64 cargo-deny; do";
    assert!(y.contains(order), "{y}");
    assert!(y.find("npm pkg set").unwrap() < y.find(order).unwrap());
    for dep in ["cargo-deny-linux-x64", "cargo-deny-linux-arm64"] {
        assert!(
            y.contains(&format!(
                "\"optionalDependencies.@keelstackhq/{dep}=$VERSION\""
            )),
            "{dep}"
        );
    }
    // npm >= 11.5.1 does the OIDC exchange, as in release.yml.
    assert!(y.contains("npm install -g npm@12.2.0"), "{y}");
}

#[test]
fn event_data_reaches_scripts_through_env_only() {
    let y = yaml();
    assert!(y.contains("VERSION: ${{ inputs.version }}"), "{y}");
    let scripts = run_scripts(&y);
    assert!(!scripts.is_empty());
    for s in scripts {
        assert!(!s.contains("${{"), "expression in a run script: {s}");
    }
}

#[test]
fn the_packages_name_the_upstream_binary_and_agree() {
    let launcher = json("npm/cargo-deny/package.json");
    assert_eq!(launcher["name"], "@keelstackhq/cargo-deny");
    assert_eq!(launcher["bin"]["cargo-deny"], "bin/cargo-deny.js");
    let version = launcher["version"].as_str().unwrap().to_string();
    let opt = launcher["optionalDependencies"].as_object().unwrap();
    let mut keys: Vec<&String> = opt.keys().collect();
    keys.sort();
    assert_eq!(
        keys,
        [
            "@keelstackhq/cargo-deny-linux-arm64",
            "@keelstackhq/cargo-deny-linux-x64"
        ]
    );
    for (platform, cpu) in [("linux-x64", "x64"), ("linux-arm64", "arm64")] {
        let name = format!("@keelstackhq/cargo-deny-{platform}");
        assert_eq!(opt[&name].as_str(), Some(version.as_str()), "{name}");
        let pkg = json(&format!("npm/cargo-deny-{platform}/package.json"));
        assert_eq!(pkg["name"], name);
        assert_eq!(pkg["version"].as_str(), Some(version.as_str()), "{name}");
        assert_eq!(pkg["os"], serde_json::json!(["linux"]), "{name}");
        assert_eq!(pkg["cpu"], serde_json::json!([cpu]), "{name}");
        assert_eq!(pkg["files"], serde_json::json!(["bin/"]), "{name}");
        assert_eq!(pkg["publishConfig"]["access"], "public", "{name}");
        // Upstream's license, not bosun's.
        assert_eq!(pkg["license"], "MIT OR Apache-2.0", "{name}");
    }
    assert_eq!(launcher["license"], "MIT OR Apache-2.0");
}

#[test]
fn ci_runs_every_npm_launcher_test() {
    let ci = read(".github/workflows/ci.yml");
    assert!(ci.contains("node --test npm/*/test/*.test.js"), "ci.yml");
}
