//! R-secrets: no false positives on code and test fixtures, real secrets still caught (spec §8).
//! Lines are written with placeholders (`@TOK@`, `@SEC@`, `@AKEY@`) so this file does not match
//! the rule that judges its own PR.
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
    assert!(!secret_match("src/a.rs", "let @TOK@ = a; let @SEC@ = b;"));
}

#[test]
fn types_fields_and_references_are_not_secrets() {
    for (path, line) in [
        ("src/a.rs", "    pub @TOK@: String,"),
        ("src/a.rs", "fn f(password: &str) {"),
        ("src/a.rs", "    Self { @TOK@: None }"),
        ("src/a.rs", "        @TOK@: cfg.@TOK@,"),
        ("src/a.py", "def f(@TOK@: str):"),
        ("src/a.ts", "  @TOK@: process.env.T,"),
        ("src/a.ts", "const @TOK@ = process.env.T;"),
        (
            "src/a.rs",
            "let t = std::env::var(\"X\"); self.@TOK@ = env::var(\"X\").ok();",
        ),
        ("ci/x.yml", "        GH_@TOK@: ${{ github.@TOK@ }}"),
        ("ci/x.yml", "    @SEC@s: inherit"),
        ("ci/x.yml", "    @AKEY@=${{ secrets.K }}"),
        ("ci.sh", "@TOK@=$GH_@TOK@"),
        ("docs/a.md", "The @TOK@: pass it in the header."),
    ] {
        assert!(!secret_match(path, line), "{path}: {line}");
    }
}

#[test]
fn quoted_and_assigned_values_still_match() {
    for (path, line) in [
        ("config/a.json", "  \"api@TOK@\": \"x\""),
        ("config/a.json", "  \"@AKEY@\": \"x\""),
        ("config/a.yml", "@TOK@: 's3cr3t'"),
        ("ci.sh", "SECRET_KEY=abc"),
        ("ci.sh", "PASSWORD=abc"),
        ("ci.sh", "private_key=abc"),
        ("ci.sh", "url=https://u:p@host/x"),
    ] {
        assert!(secret_match(path, line), "{path}: {line}");
    }
}

#[test]
fn literals_in_code_still_match() {
    for line in [
        "    let @TOK@ = \"s3cr3t-value\";",
        "    let @TOK@ = r\"s3cr3t-value\";",
        "    let @TOK@ = String::from(\"s3cr3t-value\");",
        "    let @TOK@ = Some(\"s3cr3t-value\");",
        "    let @TOK@ = &\"s3cr3t-value\";",
        "const @TOK@ = process.env.T || 's3cr3t-value';",
        "let a = v; let @TOK@ = \"s3cr3t-value\";",
        "let a = v; @AKEY@ = s3cr3t",
    ] {
        assert!(secret_match("src/auth.rs", line), "{line}");
    }
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
        "crates/x/tests/fixtures/app.toml",
        "client_@SEC@=s3cr3t-value-xyz"
    ));
    // A file name is the author's choice: only the directory exempts.
    assert!(secret_match("src/login_test.go", "@TOK@ = \"fake\""));
    assert!(secret_match(
        "web/src/login.spec.ts",
        "const t = '@TOK@ = fake';"
    ));
    assert!(!secret_match("src/snapshots/x__y.snap", "@TOK@ = fake"));
    assert!(!secret_match("web/__snapshots__/a.snap", "@TOK@ = fake"));
    assert!(secret_match("src/x__y.snap", "@TOK@ = fake"));
}

#[test]
fn env_and_config_files_are_never_test_files() {
    for path in [
        "tests/fixtures/app.env",
        "tests/fixtures/.env.local",
        "tests/config/app.toml",
        "test/deploy/values.yaml",
        "spec/app.toml",
    ] {
        assert!(secret_match(path, "@TOK@ = s3cr3t"), "{path}");
    }
}

#[test]
fn the_allow_marker_exempts_nothing() {
    for line in [
        "@TOK@ = \"s3cr3t-public-demo\" # bosun:allow-secret public demo key from the vendor docs",
        "let @TOK@ = \"s3cr3t-demo\"; // bosun:allow-secret documented sandbox value",
        "@TOK@ = \"s3cr3t\" # bosun:allow-secret",
    ] {
        assert!(secret_match("config/app.toml", line), "{line}");
    }
    assert!(secret_match(
        "src/a.rs",
        "@TOK@ = \"s3cr3t\" # bosun:allow-secret  -->"
    ));
    assert!(secret_match(
        "src/a.rs",
        "/* bosun:allow-secret demo */ @TOK@ = \"s3cr3t\""
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
        "-----BEGIN PGP PRIVATE KEY BLOCK-----".to_string(),
        format!("    let t = \"{gh}\";"),
        format!("x = {gh} # bosun:allow-secret it is fake, honest"),
        format!("key: AKIA{}", "ABCDEFGHIJKLMNOP"),
        format!("anthropic: sk-ant-{}", "api03-abcdefghijklmnopqrstuv"),
        format!("stripe: sk_live_{}", "abcdefghijklmnop1234"),
        format!("stripe: rk_live_{}", "abcdefghijklmnop1234"),
        format!("SLACK=xoxb-{}", "1234567890-abcdef"),
    ];
    for line in &lines {
        assert!(secret_match("tests/fixtures/keys.txt", line), "{line}");
        assert!(secret_match("src/main.rs", line), "{line}");
    }
}

#[test]
fn a_keyword_inside_a_string_literal_is_not_a_key() {
    for (path, line) in [
        ("src/a.py", "print(\"Password: \" + x)"),
        ("src/a.java", "log.info(\"@TOK@: %s\", v)"),
        ("tools/a.sh", "read -p \"Token: \" T"),
        ("tools/a.sh", "PASSWORD=\"$DB_PASSWORD\""),
        ("tools/a.sh", "@TOK@=\"$GH_@TOK@\""),
    ] {
        assert!(!secret_match(path, line), "{path}: {line}");
    }
}

#[test]
fn typed_and_walrus_literals_still_match() {
    for (path, line) in [
        ("src/a.go", "password := \"s3cr3t\""),
        ("src/a.rs", "const TOKEN: &str = \"s3cr3t\";"),
        ("src/a.rs", "static @TOK@: &'static str = \"s3cr3t\";"),
        ("src/a.ts", "const password: string = \"s3cr3t\";"),
        ("src/a.py", "password: str = \"s3cr3t\""),
        ("infra/a.tf", "password = \"s3cr3t\""),
        ("infra/a.tf", "{ @TOK@: \"s3cr3t\" }"),
    ] {
        assert!(secret_match(path, line), "{path}: {line}");
    }
    // A reference is no literal, in a .tf file too.
    assert!(!secret_match("infra/a.tf", "password = var.db_password"));
    assert!(!secret_match("src/a.go", "password := cfg.Password"));
}

#[test]
fn shell_defaults_with_a_literal_match() {
    for line in [
        "API_@TOK@^=${API_@TOK@^:-abcd1234efgh5678}",
        "@TOK@^=\"${@TOK@^:-abcd1234efgh5678}\"",
        "export SECRET=${SECRET-abcd1234efgh5678}",
    ] {
        let line = line.replace('^', "").replace("@TOK@", "TOKEN");
        assert!(secret_match("src/a.sh", &line), "{line}");
    }
    // A default that is itself a reference, or empty, is no literal.
    assert!(!secret_match("src/a.sh", "API_@TOK@=${API_@TOK@:-$OTHER}"));
    assert!(!secret_match("src/a.sh", "API_@TOK@=${API_@TOK@:-}"));
    assert!(!secret_match("src/a.sh", "API_@TOK@=${API_@TOK@}"));
}

#[test]
fn quoted_literals_that_start_with_a_dollar_match() {
    for (path, line) in [
        ("src/a.py", "@TOK@ = '$ecret1234'"),
        ("src/a.js", "const @SEC@ = '$up3rS3cret!';"),
        ("src/a.py", "@TOK@ = \"$2b$10$abcdefghijk\""),
        ("src/a.sh", "@TOK@='$ecret1234'"),
    ] {
        assert!(secret_match(path, line), "{path}: {line}");
    }
    // A double-quoted `$VAR` in shell or yaml reads a variable.
    assert!(!secret_match("src/a.sh", "@TOK@=\"$GH_@TOK@\""));
    assert!(!secret_match("ci/x.yml", "@TOK@: \"$GH_@TOK@\""));
}

#[test]
fn secret_names_that_are_no_secret_do_not_match() {
    for (path, line) in [
        ("src/a.py", "@SEC@ary = \"Bob\""),
        ("src/a.py", "@SEC@s = [\"a\", \"b\"]"),
        ("src/a.py", "@SEC@s_dir = \"/run/@SEC@s\""),
        ("src/a.py", "@SEC@_version = \"latest\""),
        ("deploy/k8s.yml", "  @SEC@Name: \"tls-cert\""),
        ("deploy/k8s.yml", "  @TOK@@SEC@: \"my-@TOK@\""),
        ("src/a.yml", "    @SEC@Ref: \"db\""),
        ("src/a.yml", "- @SEC@KeyRef: \"x\""),
        ("src/a.yml", "  @SEC@_name: \"x\""),
        ("src/a.yml", "  @SEC@_id: \"x\""),
    ] {
        assert!(!secret_match(path, line), "{path}: {line}");
    }
    for (path, line) in [
        ("src/a.py", "@SEC@ = \"x\""),
        ("src/a.py", "client_@SEC@ = \"x\""),
        ("src/a.go", "@SEC@Key := \"x\""),
        ("src/a.py", "SECRET_KEY = \"x\""),
        ("src/a.py", "aws_@SEC@_access_key = \"x\""),
        ("src/a.py", "@SEC@_@TOK@ = \"x\""),
        ("src/a.yml", "  client_@SEC@: \"x\""),
    ] {
        assert!(secret_match(path, line), "{path}: {line}");
    }
}

#[test]
fn everyday_references_are_not_secrets() {
    for (path, line) in [
        (
            "ci/clone.yml",
            "  - git clone https://oauth2:${CI_JOB_@TOK@}@gitlab.com/a/b",
        ),
        (
            "ci/p.sh",
            "git push https://$GITHUB_ACTOR:$GITHUB_@TOK@@github.com/a/b",
        ),
        (
            "ci/p.sh",
            "git remote set-url origin \"https://x-access-@TOK@:${GH_@TOK@}@github.com/$REPO\"",
        ),
        ("src/a.js", "`https://x:${@TOK@}@github.com/a/b`"),
        ("config/s.php", "    '@TOK@' => env('@TOK@'),"),
        ("config/s.php", "    'password' => Hash::make($pw),"),
        ("src/h.go", "password := r.FormValue(\"password\")"),
        ("src/h.go", "@TOK@ := c.Query(\"@TOK@\")"),
        ("deploy/v.yml", "  password: \"{{ .Values.db.password }}\""),
    ] {
        assert!(!secret_match(path, &fill(line)), "{path}: {line}");
    }
    // Literals still match.
    for (path, line) in [
        ("ci/p.sh", "git clone https://bob:hunter22@gitlab.com/a/b"),
        (
            "deploy/v.yml",
            "  password: \"{{ .Values.x | default \"abc\" }}\"",
        ),
        ("deploy/v.yml", "  password: \"{{ .Values.x }}hunter2\""),
    ] {
        assert!(secret_match(path, line), "{path}: {line}");
    }
}

/// Risk-check of PR 1 that adds `lines` (in order) to `path`: true when R-secrets matches.
fn secret_match_lines(path: &str, lines: &[&str]) -> bool {
    let fake = FakeGithub::start();
    fake.pull(1, "change", "feature", "");
    fake.content(
        "governance/owners.toml",
        BASE,
        "[[owner]]\ngithub = \"ada\"\n",
    );
    let body: String = lines.iter().map(|l| format!("\n+{}", fill(l))).collect();
    let patch = format!("@@ -0,0 +1,{} @@{body}", lines.len());
    fake.pull_file(1, path, "modified", lines.len() as u64, 0, Some(&patch));
    let (code, v) = run(&fake, "session_test", NOW, &["risk-check", "1"]);
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
fn unquoted_yaml_literals_match_in_config_files() {
    for (path, line) in [
        ("deploy/db.yml", "password: s3cr3t"),
        ("deploy/db.yaml", "    db_password: s3cr3t # prod"),
        ("deploy/db.yml", "  - password: s3cr3t"),
        ("deploy/app.yml", "  client_@SEC@: s3cr3t-xyz"),
        ("deploy/app.yml", "  @AKEY@: abcd1234s3cr3t"),
        ("conf/app.properties", "db.password: s3cr3t"),
    ] {
        assert!(secret_match(path, line), "{path}: {line}");
    }
}

#[test]
fn unquoted_yaml_types_and_references_are_not_secrets() {
    for (path, line) in [
        ("deploy/a.yml", "  password: string"),
        ("deploy/a.yml", "  password: ${{ secrets.X }}"),
        ("deploy/a.yml", "  @TOK@: ${@TOK@}"),
        ("deploy/a.yml", "  password: !secret db"),
        ("deploy/a.yml", "  password:"),
        ("deploy/a.yml", "  password: null"),
        ("deploy/a.yml", "  password: ~"),
        ("deploy/a.yml", "  password: \"\""),
        ("deploy/a.yml", "  password: true"),
        ("deploy/a.yml", "  password: |"),
        ("deploy/a.yml", "  password: *db_password"),
        ("deploy/a.yml", "  password: \"{{ .Values.p }}\""),
        ("deploy/a.yml", "  password: Enter your password"),
        ("ci/release.yml", "      id-@TOK@: write"),
        ("etc/nsswitch.conf", "passwd:         files"),
        // Durations and paths under a secret key are settings, not secrets.
        ("deploy/a.yml", "  access_@TOK@: 1h"),
        ("deploy/a.yml", "  refresh_@TOK@: 720h"),
        ("deploy/a.yml", "  @TOK@_ttl_@TOK@: 30s"),
        ("deploy/a.yml", "  private_key: /etc/ssl/private/key.pem"),
        (
            "deploy/a.yml",
            "  @TOK@: /var/run/secrets/kubernetes.io/serviceaccount/@TOK@",
        ),
        ("deploy/a.yml", "  password: ./secrets/db"),
        ("deploy/a.yml", "  password: ../secrets/db"),
        ("deploy/a.yml", "  password: ~/secrets/db"),
        ("tests/fixtures/a.yml", "  password: s3cr3t"),
        // Code files keep today's rules: a type, a field or a parameter.
        ("src/a.rs", "    pub password: String,"),
        ("src/a.rs", "    @TOK@: &str,"),
        ("src/a.rs", "    password: Option<String>,"),
        ("src/a.rs", "fn f(password: &str) {"),
        ("src/a.ts", "  password: string;"),
        ("src/a.py", "    password: str = field(default=None)"),
    ] {
        assert!(!secret_match(path, line), "{path}: {line}");
    }
}

#[test]
fn php_array_literals_match() {
    for line in [
        "    'password' => 's3cr3t',",
        "    \"@AKEY@\" => \"abc\",",
        "    'db_password' => 's3cr3t'",
    ] {
        assert!(secret_match("config/database.php", line), "{line}");
    }
    for line in [
        "    'password' => $pw,",
        "    'password' => env('X'),",
        "    '@TOK@' => '',",
        // Laravel casts and validation rules name a type or a rule, not a value.
        "    protected $casts = ['password' => 'hashed'];",
        "    'password' => 'required',",
        "    '@TOK@' => 'string',",
        "    'password' => 'encrypted',",
        "    'password' => 'confirmed',",
        "    'api_@TOK@' => 'nullable',",
        "    '@TOK@' => 'sometimes',",
        "    'password' => 'required|min:8',",
    ] {
        assert!(!secret_match("config/database.php", line), "{line}");
    }
}

#[test]
fn kubernetes_env_values_under_a_secret_name_match() {
    for lines in [
        ["        - name: DB_PASSWORD", "          value: \"s3cr3t\""],
        ["        - name: DB_PASSWORD", "          value: s3cr3t"],
        ["        - name: API_@TOK@", "          value: 's3cr3t'"],
        [
            "        - name: DB_PASSWORD # prod db",
            "          value: s3cr3t",
        ],
        [
            "        - name: \"STRIPE_@SEC@_KEY\"",
            "          value: s3cr3t",
        ],
    ] {
        assert!(secret_match_lines("deploy/app.yaml", &lines), "{lines:?}");
    }
    for lines in [
        ["        - name: DB_PASSWORD", "          valueFrom:"],
        [
            "        - name: DB_PASSWORD",
            "          value: \"$(DB_PASS)\"",
        ],
        ["        - name: DB_PASSWORD", "          value: \"\""],
        ["        - name: LOG_LEVEL", "          value: debug"],
        [
            "        - name: DB_PASSWORD",
            "          value: /run/secrets/db",
        ],
        [
            "        - name: DB_PASSWORD_FILE",
            "          value: /run/s/db",
        ],
        [
            "        - name: DB_PASSWORD",
            "          value: ${{ secrets.X }}",
        ],
    ] {
        assert!(!secret_match_lines("deploy/app.yaml", &lines), "{lines:?}");
    }
}

/// A value that ends in a multi-byte character never panics risk-check (one JSON object out).
#[test]
fn non_ascii_values_do_not_panic() {
    for (path, line) in [
        ("config/locales/ja.yml", "パスワード: パスワード"),
        ("config/locales/ja.yml", "  password: パスワード"),
        ("deploy/a.yml", "  token: café"),
        ("lang/ja/validation.php", "    'password' => 'パスワード',"),
    ] {
        secret_match_lines(path, &[line]);
    }
}

/// Lockfile entries, versions and interpolated values are no literal secrets.
#[test]
fn versions_lockfiles_and_interpolations_are_not_secrets() {
    for (path, line) in [
        ("pnpm-lock.yaml", "      g@TOK@: 7.1.0"),
        ("pnpm-lock.yaml", "      jsonweb@TOK@: 9.0.2"),
        ("pnpm-workspace.yaml", "  jsonweb@TOK@: ^9.0.2"),
        ("deploy/values.yml", "  jsonweb@TOK@: ~9.0.2"),
        ("config/db.rb", "  'password' => \"#{ENV['DB_PASS']}\","),
        ("config/db.php", "    'password' => \"{$pass}\","),
        ("config/db.php", "    'password' => \"$pass\","),
        ("config/db.rb", "  'password' => \"#{pw}\","),
        ("config/db.php", "    'password' => $pass,"),
        // A single-quoted reference is shell text outside PHP, Ruby and Perl; `'${X}'` and a
        // lone `'$'` are references or nothing in any file.
        ("config/db.php", "    'password' => '${DB_PASSWORD}',"),
        ("config/db.php", "    'password' => '$',"),
        ("docker/entrypoint.sh", "  'password' => '$DB_PASSWORD',"),
        ("docker/entrypoint.sh", "  'password' => '${DB_PASSWORD}',"),
        ("scripts/setup.sh", "echo \"'password' => '$DB_PASSWORD'\" > c.php"),
    ] {
        assert!(!secret_match_lines(path, &[line]), "{path}: {line}");
    }
    // A real literal in a yaml that is no lockfile still matches.
    assert!(secret_match_lines(
        "deploy/values.yml",
        &["  db_password: s3cr3t"]
    ));
}

/// Skip-list holes: a version-looking, `$`-holding or lock-named value or file still hides no secret.
#[test]
fn version_dollar_and_lock_lookalikes_still_match() {
    for (path, line) in [
        ("deploy/a.yml", "  password: 9.Xk3mP2qLz"),
        ("deploy/a.yml", "  password: 1.0-Xk3mP2qLzR8"),
        ("deploy/a.yml", "  password: v1.2.3hunter"),
        ("deploy/a.yml", "  password: P4ss$word9"),
        ("config/db.php", "    'password' => 'v1.2a',"),
        ("config/db.php", "    'password' => 'P4ss$w',"),
        // Single quotes never interpolate in PHP or Ruby: `'$ecret1'` is a literal.
        ("config/db.php", "    'password' => '$ecret1',"),
        ("config/db.rb", "  'password' => '$ecret1',"),
        ("config/db.php", "    '@TOK@' => '$3cr3t',"),
        ("deploy/deadlock.yaml", "  password: s3cr3tX"),
        ("deploy/block.yml", "  password: s3cr3tX"),
    ] {
        assert!(secret_match_lines(path, &[line]), "{path}: {line}");
    }
    assert!(secret_match_lines(
        "k8s/deploy.yaml",
        &[
            "        - name: DB_PASSWORD",
            "          value: \"v1.2.3x\""
        ]
    ));
    // Real lockfiles and pure versions stay out.
    for (path, line) in [
        ("yarn.lock", "  jsonweb@TOK@: 9.0.2"),
        ("app/poetry.lock", "  jsonweb@TOK@: 9.0.2"),
        ("deploy/a.yml", "  jsonweb@TOK@: v9.0.2"),
    ] {
        assert!(!secret_match_lines(path, &[line]), "{path}: {line}");
    }
}

/// A reference glued in front of a literal does not hide it; a whole interpolation does.
#[test]
fn glued_references_do_not_hide_literals() {
    assert!(secret_match_lines(
        "deploy/a.yml",
        &["  password: ${UNUSED}hunter2"]
    ));
    assert!(secret_match_lines(
        "k8s/deploy.yaml",
        &[
            "        - name: DB_PASSWORD",
            "          value: \"${X}hunter2\""
        ]
    ));
    assert!(!secret_match_lines(
        "config/db.rb",
        &["  'password' => \"#{pw}\","]
    ));
    assert!(!secret_match_lines(
        "deploy/a.yml",
        &["  password: ${DB_PASSWORD}"]
    ));
}
