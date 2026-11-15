//! Audit fixes for risk (spec section 8): added lines, secret shapes, exemptions, CI paths,
//! lockfile sources, glob case and binary files. Fake secrets are built at run time.
use bosun::core::risk::{ChangedFile, FileView, Rules, added_lines, evaluate, unseen_diff};

fn file(path: &str, line: &str) -> ChangedFile {
    ChangedFile {
        path: path.into(),
        previous_path: None,
        added_lines: vec![line.into()],
    }
}

fn hit(rules: &Rules, path: &str, line: &str, id: &str) -> bool {
    evaluate(rules, &[file(path, line)], &[], &[])
        .iter()
        .any(|m| m.rule == id)
}

#[test]
fn an_added_line_starting_with_plus_plus_is_kept() {
    let patch = "--- a/x\n+++ b/x\n@@ -1 +1,2 @@\n+++ a counter\n+ok\n";
    assert_eq!(added_lines(patch), ["++ a counter", "ok"]);
    // Only the headers before the first hunk are skipped.
    assert_eq!(added_lines("+++ b/x\n"), Vec::<String>::new());
}

#[test]
fn secret_shapes_match_r_secrets() {
    let rules = Rules::defaults();
    let (s, k, p) = ("SECRET", "KEY", "PASS");
    let cases = [
        format!("{s}_{k}=abc"),
        format!("AWS_{s}_ACCESS_{k}=abc"),
        format!("{p}WORD=abc"),
        format!("private_{}=abc", k.to_lowercase()),
        format!("\"api{k}\": \"abc\""),
        "token: \"abc\"".to_string(),
        format!("url = postgres://user:{}@db.internal/x", "pw1"),
    ];
    for line in &cases {
        assert!(hit(&rules, "src/conf.txt", line, "R-secrets"), "{line}");
    }
}

#[test]
fn marker_and_name_do_not_exempt_a_secret() {
    let rules = Rules::defaults();
    let line = format!("{}_{}=abc # bosun:allow-secret fake", "SECRET", "KEY");
    assert!(hit(&rules, "src/conf.txt", &line, "R-secrets"));
    let line = format!("{}_{}=abc", "SECRET", "KEY");
    for p in [
        "src/test_conf.txt",
        "src/conf_test.go",
        "web/a.spec.ts",
        "x.snap",
    ] {
        assert!(hit(&rules, p, &line, "R-secrets"), "{p}");
    }
    // tests/ and fixtures/ directories stay exempt.
    for p in ["tests/conf.txt", "crates/x/fixtures/conf.txt"] {
        assert!(!hit(&rules, p, &line, "R-secrets"), "{p}");
    }
}

#[test]
fn build_and_script_files_are_r_ci() {
    let rules = Rules::defaults();
    for p in [
        "scripts/x.sh",
        "Makefile",
        "sub/Makefile",
        "justfile",
        "Dockerfile",
        "svc/Dockerfile",
        ".husky/pre-commit",
        ".github/dependabot.yml",
    ] {
        assert!(hit(&rules, p, "x", "R-ci"), "{p}");
    }
}

#[test]
fn lockfile_source_host_changes_are_r_deps() {
    let rules = Rules::defaults();
    let ok = [
        (
            "Cargo.lock",
            "source = \"registry+https://github.com/rust-lang/crates.io-index\"",
        ),
        (
            "package-lock.json",
            "\"resolved\": \"https://registry.npmjs.org/a/-/a-1.0.0.tgz\",",
        ),
        (
            "pnpm-lock.yaml",
            "resolved: https://registry.npmjs.org/a/-/a-1.0.0.tgz",
        ),
        ("Cargo.lock", "name = \"serde\""),
    ];
    for (p, l) in ok {
        assert!(!hit(&rules, p, l, "R-deps"), "{p} {l}");
    }
    let bad = [
        (
            "Cargo.lock",
            "source = \"git+https://evil.example/x?rev=1#abc\"",
        ),
        (
            "Cargo.lock",
            "source = \"registry+https://evil.example/index\"",
        ),
        (
            "package-lock.json",
            "\"resolved\": \"https://evil.example/a.tgz\",",
        ),
        ("pnpm-lock.yaml", "resolved: https://evil.example/a.tgz"),
        ("yarn.lock", "  resolved \"https://evil.example/a.tgz#abc\""),
    ];
    for (p, l) in bad {
        assert!(hit(&rules, p, l, "R-deps"), "{p} {l}");
    }
}

#[test]
fn a_rule_glob_with_capitals_matches_any_case() {
    let rules =
        Rules::parse("[[rule]]\nid = \"R-x\"\nwhy = \"x\"\npaths = [\"Docs/Secret.TOML\"]\n")
            .unwrap();
    for p in ["Docs/Secret.TOML", "docs/secret.toml", "DOCS/SECRET.TOML"] {
        assert!(hit(&rules, p, "x", "R-x"), "{p}");
    }
}

#[test]
fn a_binary_without_a_patch_is_unseen() {
    let f = |path: &str, status: &str, changes: u64| FileView {
        path: path.into(),
        status: status.into(),
        has_patch: false,
        changes,
    };
    assert!(unseen_diff(&[f("a.png", "added", 0)], 1).is_some());
    assert!(unseen_diff(&[f("a.png", "modified", 0)], 1).is_some());
    assert!(unseen_diff(&[f("a.png", "removed", 0)], 1).is_none());
    assert!(unseen_diff(&[f("b.png", "renamed", 0)], 1).is_none());
}
