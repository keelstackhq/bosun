//! R-secrets: no false positives on code and test fixtures, real secrets still caught (spec §8).
//! Lines are written with placeholders (`@TOK@`, `@SEC@`, `@AKEY@`) so this file does not match the
//! rule that judges its own PR.
mod common;

use common::fake::FakeGithub;
use common::run;
use serde_json::Value;

const NOW: &str = "2026-10-22T10:00:00Z";
const BASE: &str = "base0000";

fn fill(line: &str) -> String {
    line.replace("@TOK@", "token")
        .replace("@SEC@", "secret")
        .replace("@AKEY@", "api_key")
}

/// Risk-check of PR 1 that adds `line` to `path`, under the built-in rules.
fn check(path: &str, line: &str) -> (i32, Value) {
    let fake = FakeGithub::start();
    fake.pull(1, "change", "feature", "");
    fake.content(
        "governance/owners.toml",
        BASE,
        "[[owner]]\ngithub = \"ada\"\n",
    );
    let patch = format!("@@ -0,0 +1,1 @@\n+{line}");
    fake.pull_file(1, path, "modified", 1, 0, Some(&patch));
    run(&fake, "session_test", NOW, &["risk-check", "1"])
}

fn secret_match(path: &str, line: &str) -> bool {
    let (code, v) = check(path, &fill(line));
    let hit = v["data"]["matches"]
        .as_array()
        .is_some_and(|m| m.iter().any(|m| m["rule"] == "R-secrets"));
    assert_eq!(hit, code == 5, "{v}");
    assert!(
        !v.to_string().contains("s3cr3t"),
        "evidence copies the line: {v}"
    );
    hit
}

#[test]
fn code_bindings_are_not_secrets() {
    assert!(!secret_match("src/auth.rs", "    let @TOK@ = v;"));
    assert!(!secret_match(
        "src/auth.rs",
        "    let mut @AKEY@ = cfg.@AKEY@.clone();"
    ));
    assert!(!secret_match("web/a.ts", "const @TOK@ = await getToken();"));
}

#[test]
fn literals_in_code_still_match() {
    assert!(secret_match(
        "src/auth.rs",
        "    let @TOK@ = \"s3cr3t-value\";"
    ));
    assert!(secret_match(
        "src/auth.rs",
        "    let @TOK@ = r\"s3cr3t-value\";"
    ));
    assert!(secret_match("config/app.env", "API_@TOK@ = s3cr3t"));
    assert!(secret_match(
        "config/app.toml",
        "client_@SEC@=s3cr3t-value-xyz"
    ));
}

#[test]
fn test_files_and_fixtures_are_not_secrets() {
    assert!(!secret_match(
        "tests/cli_login.rs",
        "    let body = \"@TOK@ = \\\"fake-service\\\"\";"
    ));
    assert!(!secret_match(
        "crates/x/tests/fixtures/app.env",
        "client_@SEC@=s3cr3t-value-xyz"
    ));
    assert!(!secret_match("src/login_test.go", "@TOK@ = \"fake\""));
    assert!(!secret_match(
        "web/src/login.spec.ts",
        "const t = '@TOK@ = fake';"
    ));
    assert!(!secret_match("src/snapshots/x__y.snap", "@TOK@ = fake"));
}

#[test]
fn the_marker_needs_a_reason() {
    assert!(!secret_match(
        "config/app.toml",
        "@TOK@ = \"s3cr3t-public-demo\" # bosun:allow-secret public demo key from the vendor docs"
    ));
    assert!(secret_match(
        "config/app.toml",
        "@TOK@ = \"s3cr3t\" # bosun:allow-secret"
    ));
    assert!(secret_match(
        "config/app.toml",
        "@TOK@ = \"s3cr3t\" # bosun:allow-secret  -->"
    ));
}

#[test]
fn real_key_formats_are_never_exempt() {
    let gh = format!("ghp_{}", "a1B2c3D4e5F6g7H8i9J0k1L2m3N4o5P6q7R8");
    let age = format!(
        "AGE-SECRET-KEY-{}",
        "1QQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQ"
    );
    let lines = [
        age,
        "-----BEGIN OPENSSH PRIVATE KEY-----".to_string(),
        format!("    let t = \"{gh}\";"),
        format!("x = {gh} # bosun:allow-secret it is fake, honest"),
        format!("key: AKIA{}", "ABCDEFGHIJKLMNOP"),
        format!("anthropic: sk-ant-{}", "api03-abcdefghijklmnopqrstuv"),
        format!("SLACK=xoxb-{}", "1234567890-abcdef"),
    ];
    for line in &lines {
        assert!(secret_match("tests/fixtures/keys.txt", line), "{line}");
        assert!(secret_match("src/main.rs", line), "{line}");
    }
}
