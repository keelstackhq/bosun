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
fn literal_values_behind_a_reference_or_a_dollar_quote_still_match() {
    let rules = Rules::defaults();
    let cases = [
        "API_KEY=$'abcd1234'",
        "TOKEN=$\"abc\"",
        "TOKEN=${UNUSED}realsecretvalue",
        "let token = env::var(\"T\").unwrap_or(\"hardcoded\".into());",
        "let token = env::var(\"T\").unwrap_or_else(|| \"hardcoded\".into());",
        "password: \"${{ x }}hunter2\"",
        "let token = env::var(\"T\").unwrap_or_else(|_| \"x\".into());",
        "let token = env::var(\"T\").ok().unwrap_or(\"x\".into());",
        "$password = \"hunter2\";",
        "my $password = \"hunter2\";",
        "$token='abc123'",
    ];
    for line in cases {
        assert!(hit(&rules, "src/conf.txt", line, "R-secrets"), "{line}");
    }
    // A plain reference is still no secret.
    for line in [
        "TOKEN=$OTHER",
        "TOKEN=${OTHER}",
        "password: \"${{ secrets.X }}\"",
        "let token = env::var(\"T\").unwrap();",
        "let token = std::env::var(\"T\")?;",
    ] {
        assert!(!hit(&rules, "src/conf.txt", line, "R-secrets"), "{line}");
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
        "src/conf_test.txt",
        "web/a.spec.json",
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
            "source = \"git+https://github.com/evil/x?q=registry+https://github.com/rust-lang/crates.io-index\"",
        ),
        (
            "Cargo.lock",
            "source = \"registry+https://github.com/rust-lang/crates.io-index-evil\"",
        ),
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
        same_blob: path.starts_with('b'),
    };
    assert!(unseen_diff(&[f("a.png", "added", 0)], 1).is_some());
    assert!(unseen_diff(&[f("a.png", "modified", 0)], 1).is_some());
    assert!(unseen_diff(&[f("a.png", "removed", 0)], 1).is_none());
    assert!(unseen_diff(&[f("b.png", "renamed", 0)], 1).is_none());
    // Renamed and changed (a different blob): unseen.
    assert!(unseen_diff(&[f("c.png", "renamed", 0)], 1).is_some());
}

#[test]
fn ordinary_code_is_no_secret() {
    let rules = Rules::defaults();
    let cases = [
        ("app/main.py", "password = getenv(\"DB_PASSWORD\")"),
        ("app/main.py", "password = os.getenv(\"DB_PASSWORD\")"),
        ("app/main.py", "password = os.environ[\"DB_PASSWORD\"]"),
        ("app/main.py", "password = os.environ.get(\"DB_PASSWORD\")"),
        (
            "src/X.java",
            "String password = System.getenv(\"DB_PASSWORD\");",
        ),
        ("src/x.c", "char *password = getenv(\"DB_PASSWORD\");"),
        ("src/x.php", "$password = getenv(\"DB_PASSWORD\");"),
        ("src/x.rb", "password = ENV.fetch(\"DB_PASSWORD\")"),
        ("src/x.rb", "password = ENV[\"DB_PASSWORD\"]"),
        ("app/main.py", "self.password = password"),
        ("app/main.py", "connect(user, password=password,"),
        ("app/main.py", "password=password,"),
        ("app/main.py", "password = config[\"password\"]"),
        ("app/main.py", "password = None"),
        ("app/main.py", "password = input(\"Password: \")"),
        ("app/main.py", "passwd = getpass()"),
        ("app/main.py", "private_key = read_file(\"k.pem\")"),
        ("src/x.rs", "secret_name = \"prod-db\""),
        ("src/x.rs", "let secret_id = \"prod-db\";"),
        ("cfg/app.toml", "secret_name = \"prod-db\""),
        ("cfg/app.toml", "private_key_path: \"/etc/k.pem\""),
        ("cfg/app.toml", "token_file = \"/run/t\""),
    ];
    for (p, line) in cases {
        assert!(!hit(&rules, p, line, "R-secrets"), "{p}: {line}");
    }
}

#[test]
fn literals_behind_calls_and_bindings_still_match() {
    let rules = Rules::defaults();
    let cases = [
        ("app/main.py", "password = os.getenv(\"X\", \"hunter2\")"),
        ("app/main.py", "password = os.getenv(\"X\") or \"hunter2\""),
        (
            "app/main.py",
            "password = os.environ.get(\"X\") or 'hunter2'",
        ),
        ("app/main.py", "password = cfg.get(\"k\", \"hunter2\")"),
        ("app/main.py", "password = Secret(\"hunter2\")"),
        ("app/main.py", "password = config[\"k\"] or \"hunter2\""),
        ("app/main.py", "password = getOrElse(\"hunter2\")"),
        ("app/main.py", "self.password = \"hunter2\""),
        ("app/main.py", "password = 12345"),
        ("src/x.rs", "password = String::from(\"x\")"),
        ("src/x.rb", "password = ENV.fetch(\"X\", \"hunter2\")"),
        ("src/x.rb", "password = ENV[\"X\"] || \"hunter2\""),
        (
            "src/x.kt",
            "val password = System.getenv(\"X\") ?: \"hunter2\"",
        ),
        ("app/main.py", "if a == b: password = \"hunter2\""),
        ("app/main.py", "password = x; password = \"hunter2\""),
        ("app/main.py", "# password = Pr0dP4ss"),
        ("src/x.js", "// token = s3cr3tT0ken"),
        (
            "app/main.py",
            "password = os.getenv(\"X\") if os.getenv(\"X\") else \"hunter2\"",
        ),
        ("app/main.py", "password = os.getenv(\"X\") + \"hunter2\""),
        (
            "app/main.py",
            "password = os.getenv(\"X\") or (\"hunter2\")",
        ),
        ("src/x.rb", "password = ENV.fetch(\"X\") { \"hunter2\" }"),
        ("app/main.py", "password = os.getenv(\"X\")\\"),
        ("app/main.py", "password = getOr(\"hunter2\")"),
        ("app/main.py", "a = 1; password = hunter2"),
        ("conf/app.ini", "password = hunter2"),
        ("conf/app.env", "password = prod_name"),
        ("conf/app.ini", "client_secret_key = hunter2"),
        ("conf/app.ini", "password = \"hunter2_id\""),
    ];
    for (p, line) in cases {
        assert!(hit(&rules, p, line, "R-secrets"), "{p}: {line}");
    }
}

#[test]
fn shell_default_forms_with_a_literal_match() {
    let rules = Rules::defaults();
    for line in [
        "SECRET=${S:=hunter2}",
        "SECRET=${S:+hunter2}",
        "SECRET=${S=hunter2}",
        "SECRET=${S+hunter2}",
        "SECRET=${S:-hunter2}",
    ] {
        assert!(hit(&rules, "x.sh", line, "R-secrets"), "{line}");
    }
    assert!(!hit(&rules, "x.sh", "SECRET=${S}", "R-secrets"));
}

#[test]
fn comparison_with_a_literal_matches() {
    let rules = Rules::defaults();
    for (p, line) in [
        ("src/a.py", r#"if token == "s3cr3tvalue":"#),
        ("src/a.py", r#"if api_key == "abcdef123":"#),
        ("src/a.rs", r#"if token == "abc" {"#),
        ("src/a.js", r#"if (secret === "abcdef") {"#),
    ] {
        assert!(hit(&rules, p, line, "R-secrets"), "{line}");
    }
    for (p, line) in [
        ("src/a.py", "if secret == expected:"),
        ("src/a.py", r#"if token == "":"#),
    ] {
        assert!(!hit(&rules, p, line, "R-secrets"), "{line}");
    }
}

#[test]
fn reader_call_with_a_secret_looking_literal_matches() {
    let rules = Rules::defaults();
    assert!(hit(
        &rules,
        "src/a.py",
        r#"token = get_token("abc12345xyz")"#,
        "R-secrets"
    ));
    for line in [
        r#"password = getenv("DB_PASSWORD")"#,
        r#"password = get_password("DB_PASSWORD")"#,
        r#"key = read_file("k.pem")"#,
        r#"token = read_file("/run/secrets/token")"#,
        r#"token = input("Enter token:")"#,
        r#"token = get_token("abc")"#,
    ] {
        assert!(!hit(&rules, "src/a.py", line, "R-secrets"), "{line}");
    }
}
