//! Risk rules and their evaluation (spec section 8). Pure: the command reads the rules from the
//! PR's base branch and passes the texts in. A rule only escalates; nothing de-escalates.

use std::collections::{BTreeMap, BTreeSet};

use regex::Regex;
use serde::{Deserialize, Serialize};

use crate::core::glob::{Glob, parse_all};
use crate::core::records::{Record, parse_record};

/// The built-in rules, used when the base branch has no `.bosun/risk.toml`.
pub const DEFAULT_RULES: &str = include_str!("risk_default.toml");

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawRule {
    id: String,
    why: String,
    #[serde(default)]
    paths: Vec<String>,
    #[serde(default)]
    content: Option<String>,
    #[serde(default)]
    new_dependency: bool,
    #[serde(default)]
    capability: Option<String>,
    #[serde(default)]
    approval: Option<String>,
}

/// Who may approve a high risk PR (spec section 8).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Approval {
    /// An owner from `governance/owners.toml`.
    Owner,
    /// Two independent review verdicts, or an owner.
    TwoVerdicts,
    /// A delegate key's signed approval with one clean independent verdict, or an owner.
    Delegate,
}

impl Approval {
    pub fn name(self) -> &'static str {
        match self {
            Approval::Owner => "owner",
            Approval::TwoVerdicts => "two-verdicts",
            Approval::Delegate => "delegate",
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawRules {
    #[serde(default)]
    rule: Vec<RawRule>,
}

/// One rule with its globs and regex compiled once.
#[derive(Debug, Clone)]
pub struct Rule {
    pub id: String,
    pub why: String,
    pub paths: Vec<Glob>,
    pub content: Option<Regex>,
    pub new_dependency: bool,
    pub capability: Option<String>,
    pub approval: Approval,
}

#[derive(Debug, Clone, Default)]
pub struct Rules {
    pub rules: Vec<Rule>,
}

impl Rules {
    /// Parse `[[rule]]` tables. Errors name the rule (bad regex, bad glob, no condition).
    pub fn parse(text: &str) -> Result<Rules, String> {
        let raw: RawRules = toml::from_str(text).map_err(|e| format!("risk rules: {e}"))?;
        let mut rules = Vec::new();
        for r in raw.rule {
            let id = r.id;
            let paths = parse_all(&r.paths).map_err(|e| format!("rule {id}: {e}"))?;
            let content = match r.content {
                Some(p) => Some(Regex::new(&p).map_err(|e| format!("rule {id}: bad regex: {e}"))?),
                None => None,
            };
            if paths.is_empty() && content.is_none() && !r.new_dependency && r.capability.is_none()
            {
                return Err(format!(
                    "rule {id} has no condition (paths, content, new_dependency or capability)"
                ));
            }
            let approval = match r.approval.as_deref() {
                None | Some("owner") => Approval::Owner,
                Some("two-verdicts") => Approval::TwoVerdicts,
                Some("delegate") => Approval::Delegate,
                Some(other) => {
                    return Err(format!(
                        "rule {id}: unknown approval `{other}` (use \"owner\", \"two-verdicts\" or \"delegate\")"
                    ));
                }
            };
            rules.push(Rule {
                id,
                why: r.why,
                paths,
                content,
                new_dependency: r.new_dependency,
                capability: r.capability,
                approval,
            });
        }
        Ok(Rules { rules })
    }

    pub fn defaults() -> Rules {
        Rules::parse(DEFAULT_RULES).expect("built-in risk rules parse")
    }
}

/// A changed file: its path now, its path before a rename, and the lines the PR adds.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ChangedFile {
    pub path: String,
    pub previous_path: Option<String>,
    pub added_lines: Vec<String>,
}

/// The added lines of a patch: the `+` lines of its hunks, not the file headers.
pub fn added_lines(patch: &str) -> Vec<String> {
    // File headers (`+++ b/x`) come before the first hunk; inside one, `++x` is the added `+x`.
    let mut in_hunk = false;
    let mut out = Vec::new();
    for l in patch.lines() {
        if l.starts_with("@@") {
            in_hunk = true;
        } else if in_hunk && let Some(rest) = l.strip_prefix('+') {
            out.push(rest.to_string());
        }
    }
    out
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Match {
    pub rule: String,
    pub why: String,
    pub approval: Approval,
    pub evidence: Vec<String>,
}

/// The rules that match. Content evidence names the file and line number but does not repeat the
/// line: it would copy a secret into the PR body.
pub fn evaluate(
    rules: &Rules,
    files: &[ChangedFile],
    new_deps: &[String],
    capabilities: &[String],
) -> Vec<Match> {
    let mut out = Vec::new();
    for rule in &rules.rules {
        let mut evidence = Vec::new();
        for f in files {
            let hit = std::iter::once(&f.path)
                .chain(f.previous_path.as_ref())
                .any(|p| rule.paths.iter().any(|g| matches_any_case(g, p)));
            if hit {
                evidence.push(format!("path {}", f.path));
            }
            // R-secrets: exempt code bindings, test files and marked lines, but never a
            // strong secret format, which matches even where `content` does not.
            let secrets = rule.id == "R-secrets";
            for (i, line) in f.added_lines.iter().enumerate() {
                let scrubbed = if secrets {
                    scrub_refs(line, code_file(&f.path))
                } else {
                    line.into()
                };
                let content = match &rule.content {
                    Some(re) if re.is_match(&scrubbed) => {
                        !(secrets
                            && secret_exempt(&f.path, line, |s| {
                                re.is_match(&scrub_refs(s, code_file(&f.path)))
                            }))
                    }
                    _ => false,
                };
                let prev = i.checked_sub(1).map(|p| f.added_lines[p].as_str());
                let hit = content
                    || (secrets
                        && (strong_secret(line)
                            || (!secret_test_dir(&f.path)
                                && (config_secret(&f.path, prev, &scrubbed)
                                    || arrow_secret(&scrubbed)
                                    || arrow_dollar_literal(&f.path, line)))));
                if hit {
                    evidence.push(format!("{}: added line {} matches", f.path, i + 1));
                }
            }
        }
        if rule.new_dependency {
            evidence.extend(new_deps.iter().map(|d| format!("new dependency {d}")));
            for f in files {
                if let Some(i) = (f.added_lines.iter()).position(|l| lock_source_moved(&f.path, l))
                {
                    evidence.push(format!(
                        "{}: added line {} names a new source",
                        f.path,
                        i + 1
                    ));
                }
            }
        }
        if let Some(c) = &rule.capability
            && capabilities.contains(c)
        {
            evidence.push(format!("capability {c}"));
        }
        if !evidence.is_empty() {
            out.push(Match {
                rule: rule.id.clone(),
                why: rule.why.clone(),
                approval: rule.approval,
                evidence,
            });
        }
    }
    out
}

/// Whether an added lockfile line (`source =`, `resolved:`) points at a host other than the
/// default registries (crates.io, npm, yarn): the lock then fetches from somewhere else.
fn lock_source_moved(path: &str, line: &str) -> bool {
    static RE: std::sync::LazyLock<Regex> = std::sync::LazyLock::new(|| {
        Regex::new(r#"(?i)^\s*"?(?:source|resolved)"?\s*[:=]?\s*"?([a-z+]+://[^"'\s]*)"#)
            .expect("static regex")
    });
    const LOCKS: [&str; 5] = [
        "cargo.lock",
        "package-lock.json",
        "npm-shrinkwrap.json",
        "pnpm-lock.yaml",
        "yarn.lock",
    ];
    const HOSTS: [&str; 3] = [
        "index.crates.io",
        "registry.npmjs.org",
        "registry.yarnpkg.com",
    ];
    let name = path.rsplit('/').next().unwrap_or(path).to_ascii_lowercase();
    if !LOCKS.contains(&name.as_str()) {
        return false;
    }
    let Some(c) = RE.captures(line) else {
        return false;
    };
    let url = c[1].to_ascii_lowercase();
    let host = url
        .split_once("://")
        .map_or("", |(_, r)| r.split(['/', '?', '#']).next().unwrap_or(""));
    // The crates.io git index is `github.com`: the whole value must be exactly it, so a path
    // or query that merely holds the text does not pass.
    let crates_io = url == "registry+https://github.com/rust-lang/crates.io-index";
    !(crates_io || HOSTS.contains(&host))
}

/// Secret formats that are never exempt (R-secrets): age, PEM and PGP private keys, GitHub,
/// Anthropic, OpenAI, Stripe, AWS and Slack tokens. They match even where the rule's own
/// `content` does not.
const STRONG_SECRETS: &str = concat!(
    r"AGE-SECRET-KEY-[1]|-----BEGIN (?:[A-Z0-9]+ )*PRIVATE KEY-----",
    r"|-----BEGIN PGP PRIVATE KEY BLOCK-----",
    r"|\bgh[pousr]_[A-Za-z0-9]{30,}|\bgithub_pat_[A-Za-z0-9_]{22,}",
    r"|\bsk-(?:ant-|proj-)?[A-Za-z0-9_-]{20,}|\b[sr]k_live_[A-Za-z0-9]{16,}",
    r"|\b(?:AKIA|ASIA)[0-9A-Z]{16}\b|\bxox[abprs]-[A-Za-z0-9-]{10,}",
);

fn strong_secret(line: &str) -> bool {
    static RE: std::sync::LazyLock<Regex> =
        std::sync::LazyLock::new(|| Regex::new(STRONG_SECRETS).expect("static regex"));
    RE.is_match(line)
}

/// A test file or fixture: a `test`, `tests`, `__tests__`, `fixtures`, `testdata`, `snapshots`
/// or `__snapshots__` directory, a `.snap` file, or a `*_test.*`, `*.test.*`, `*_spec.*`,
/// `*.spec.*` or `test_*` file name. Any case. An env or config file never is: a `.env*` or
/// `*.env` name, or a `config` or `deploy` directory, even under a test directory.
pub fn test_path(path: &str) -> bool {
    const DIRS: [&str; 7] = [
        "test",
        "tests",
        "__tests__",
        "fixtures",
        "testdata",
        "snapshots",
        "__snapshots__",
    ];
    let lower = path.to_ascii_lowercase();
    let mut parts: Vec<&str> = lower.split('/').collect();
    let name = parts.pop().unwrap_or_default();
    let config = name.starts_with(".env")
        || name.ends_with(".env")
        || parts.iter().any(|d| *d == "config" || *d == "deploy");
    if config {
        return false;
    }
    if parts.iter().any(|d| DIRS.contains(d)) || name.ends_with(".snap") {
        return true;
    }
    let stem = name.rsplit_once('.').map_or(name, |(s, _)| s);
    name.starts_with("test_")
        || [".test", "_test", ".spec", "_spec"]
            .iter()
            .any(|s| stem.ends_with(s))
}

/// A `test`, `tests`, `__tests__`, `fixtures`, `testdata`, `snapshots` or `__snapshots__`
/// directory at any depth, any case. A file name never exempts (an agent picks it): `test_*`,
/// `*.spec.*` and `.snap` are no exemption. An env or config file never is: a `.env*` or `*.env`
/// name, or a `config` or `deploy` directory, even under a test directory.
fn secret_test_dir(path: &str) -> bool {
    const DIRS: [&str; 7] = [
        "test",
        "tests",
        "__tests__",
        "fixtures",
        "testdata",
        "snapshots",
        "__snapshots__",
    ];
    let lower = path.to_ascii_lowercase();
    let mut parts: Vec<&str> = lower.split('/').collect();
    let name = parts.pop().unwrap_or_default();
    let config = name.starts_with(".env")
        || name.ends_with(".env")
        || parts.iter().any(|d| *d == "config" || *d == "deploy");
    !config && parts.iter().any(|d| DIRS.contains(d))
}

/// A line without references that are no secret: `${{ .. }}`, `${VAR}`, `process.env.X`,
/// `env::var(..)`, `$VAR`. They read a secret from elsewhere; a literal glued after one
/// (`${X}real`) stays, so the content regex sees `$real`.
///
/// A literal stays: the default in `${X:-lit}`, `${X-lit}`, `${X:=lit}`, `${X=lit}`, `${X:+lit}` and `${X+lit}`, a `$..` after a single quote, and
/// in a code file (`code`) one after a double quote too (`'$ecret1'`, `"$2b$10$.."`).
fn scrub_refs(line: &str, code: bool) -> std::borrow::Cow<'_, str> {
    static DEFAULT: std::sync::LazyLock<Regex> = std::sync::LazyLock::new(|| {
        Regex::new(r"^\$\{\w+:?[-=+]([^}$]+)\}$").expect("static regex")
    });
    static RE: std::sync::LazyLock<Regex> = std::sync::LazyLock::new(|| {
        Regex::new(concat!(
            r"\$\{\{.*?\}\}|\$\{[^}]*\}",
            // A template tag with no quoted default (`{{ .Values.x }}`).
            r#"|\{\{[^}"']*\}\}"#,
            // An env lookup with the name as the only argument: `getenv("X")`,
            // `os.getenv("X")`, `System.getenv("X")`, `os.environ["X"]`, `os.environ.get("X")`,
            // `ENV["X"]`, `ENV.fetch("X")`, `$_ENV["X"]`. A default (`, "d"`) stays.
            r#"|(?:\w+(?:\.|::))*getenv\(\s*(?:"[\w.-]*"|'[\w.-]*')\s*\)"#,
            r#"|(?:\w+\.)*environ\.get\(\s*(?:"[\w.-]*"|'[\w.-]*')\s*\)"#,
            r#"|(?:\w+\.)*environ\[\s*(?:"[\w.-]*"|'[\w.-]*')\s*\]"#,
            r#"|\$_(?:ENV|SERVER)\[\s*(?:"[\w.-]*"|'[\w.-]*')\s*\]"#,
            r#"|\bENV\[\s*(?:"[\w.-]*"|'[\w.-]*')\s*\]"#,
            r#"|\bENV\.fetch\(\s*(?:"[\w.-]*"|'[\w.-]*')\s*\)"#,
            r"|\$\w+|process\.env\.[\w.]*|(?:std::)?env::var\([^)]*\)",
            // A key that names or locates a secret, not the secret: `secret_name`,
            // `token_file`, `private_key_path`, `api_key_id`.
            r"|(?i:[\w-]*[_-](?:name|id|path|file))\b",
        ))
        .expect("static regex")
    });
    let mut out = String::new();
    let mut last = 0;
    for m in RE.find_iter(line) {
        // `$password = "x"` (php, perl) names the secret on the left: it stays.
        let assigned = m.as_str().starts_with('$')
            && !m.as_str().starts_with("${")
            && line[m.end()..].trim_start().starts_with('=');
        out.push_str(&line[last..m.start()]);
        let before = line[..m.start()].chars().next_back();
        let m_str = m.as_str();
        let quoted_lit = m_str.starts_with('$')
            && m_str.len() > 1
            && !m_str.starts_with("${")
            && (before == Some('\'') || (code && before == Some('"')));
        if let Some(d) = DEFAULT.captures(m_str) {
            out.push_str(&d[1]);
            last = m.end();
            continue;
        }
        if quoted_lit {
            out.push_str(m_str);
            last = m.end();
            continue;
        }
        let key = !m.as_str().starts_with('$') && is_name_key(m.as_str());
        // An env lookup is a reference only when nothing follows it but the end of the
        // expression: `getenv("X") if a else "lit"`, `+ "lit"` and a block default stay.
        let env = is_env_call(m.as_str());
        let rest = line[m.end()..].trim_start();
        let open = env
            && !(rest.is_empty()
                || rest.starts_with([';', ',', ')', '}', ']', '#'])
                || rest.starts_with("//"));
        out.push_str(if assigned || open {
            m.as_str()
        } else if key {
            "k"
        } else {
            "$"
        });
        last = m.end();
    }
    if last == 0 {
        return line.into();
    }
    out.push_str(&line[last..]);
    out.into()
}

/// A code binding whose value holds no literal, like `let <name> = cfg.k`. Any quote after the
/// `=` (a string, `String::from("...")`, `Some("...")`, an `||` fallback) makes it a literal.
fn code_binding(stmt: &str) -> bool {
    let t = stmt.trim_start();
    let Some(rest) = ["let ", "const ", "var ", "val ", "my "]
        .iter()
        .find_map(|k| t.strip_prefix(k))
    else {
        return false;
    };
    let Some((_, value)) = rest.split_once('=') else {
        return false;
    };
    let v = value.trim();
    !v.is_empty() && !v.contains(['"', '\'', '`']) && !v.starts_with(|c: char| c.is_ascii_digit())
}

/// Whether an added line that the R-secrets content matched is exempt. A strong secret never
/// is. Otherwise: any line under a `tests` or `fixtures` directory; a line where every
/// statement (split on `;`) the content matches is a code binding without a literal. No marker
/// comment exempts a line: the author of the line would write it.
pub fn secret_exempt(path: &str, line: &str, matches: impl Fn(&str) -> bool) -> bool {
    if strong_secret(line) {
        return false;
    }
    if secret_test_dir(path) {
        return true;
    }
    // One statement only: a second, unbound assignment on the line stays a match.
    let code = code_file(path) && line.split(';').filter(|s| !s.trim().is_empty()).count() == 1;
    line.split(';')
        .all(|stmt| !matches(stmt) || code_binding(stmt) || (code && assignment_binding(stmt)))
}

/// A scrub match that is an env lookup by name (not `${X}`, `$X`, `env::var` or a key).
fn is_env_call(m: &str) -> bool {
    m.contains("getenv(") || m.contains("environ") || m.starts_with("ENV") || m.starts_with("$_")
}

/// A config file where `key: value` holds a plain value: yaml, properties or env.
fn config_file(path: &str) -> bool {
    const EXT: [&str; 4] = ["yml", "yaml", "properties", "env"];
    let name = path.rsplit('/').next().unwrap_or(path).to_ascii_lowercase();
    // Lockfiles and pnpm's workspace list dependencies (`jsonwebtoken: 9.0.2`), not settings.
    let deps = name.ends_with(".lock")
        || ["-lock.json", "-lock.yaml", "-lock.yml"]
            .iter()
            .any(|s| name.ends_with(s))
        || name == "pnpm-workspace.yaml";
    !deps
        && (name.starts_with(".env")
            || name.rsplit_once('.').is_some_and(|(_, e)| EXT.contains(&e)))
}

/// A secret key name: one that ends in `api_key`, `token`, `password`, `passwd`, `private_key`,
/// or `secret` whole or as `secret_key`, `secret_access_key` or `secret_token`.
const SECRET_KEY: &str = concat!(
    r"(?:[\w.-]*(?:api[_-]?key|token|passw(?:or)?d|private[_-]?key)",
    r"|(?:[\w.-]*[_.-])?secret(?:[_-]?(?:access[_-]?)?key|[_-]?token)?)",
);

/// In a config file (R-secrets, built in): an unquoted yaml literal under a secret key
/// (`password: hunter2`, `- db_password: x`), or a Kubernetes env `value:` literal right after
/// a `name:` that names a secret (`- name: DB_PASSWORD` then `value: x`). `line` is scrubbed;
/// `prev` is the added line before it.
fn config_secret(path: &str, prev: Option<&str>, line: &str) -> bool {
    static PLAIN: std::sync::LazyLock<Regex> = std::sync::LazyLock::new(|| {
        Regex::new(&format!(
            r#"(?i)^\s*(?:-\s+)?["']?{SECRET_KEY}["']?\s*:\s+([^\s"'#].*?)\s*(?:\s#.*)?$"#
        ))
        .expect("static regex")
    });
    static NAME: std::sync::LazyLock<Regex> = std::sync::LazyLock::new(|| {
        Regex::new(&format!(
            r#"(?i)^\s*(?:-\s+)?name:\s*["']?{SECRET_KEY}["']?\s*(?:\s#.*)?$"#
        ))
        .expect("static regex")
    });
    static VALUE: std::sync::LazyLock<Regex> = std::sync::LazyLock::new(|| {
        Regex::new(r#"^\s*value:\s*(?:"([^"]*)"|'([^']*)'|([^\s"'#].*?))\s*(?:\s#.*)?$"#)
            .expect("static regex")
    });
    if !config_file(path) {
        return false;
    }
    if PLAIN.captures(line).is_some_and(|c| plain_literal(&c[1])) {
        return true;
    }
    let Some(c) = prev
        .filter(|p| NAME.is_match(p))
        .and_then(|_| VALUE.captures(line))
    else {
        return false;
    };
    match c.get(1).or(c.get(2)) {
        Some(q) => !q.as_str().is_empty() && !setting(q.as_str()),
        None => plain_literal(&c[3]),
    }
}

/// In any file (R-secrets, built in): a php or ruby array entry, a quoted secret key `=>` a
/// quoted one-word literal (`'password' => 'x'`), unless the value is a type, a cast or a
/// validation rule (`'password' => 'hashed'`, `'required|min:8'`). `line` is scrubbed.
fn arrow_secret(line: &str) -> bool {
    static ARROW: std::sync::LazyLock<Regex> = std::sync::LazyLock::new(|| {
        Regex::new(&format!(
            r#"(?i)["']{SECRET_KEY}["']\s*=>\s*(?:"([^"\s$|][^"\s|]*)"|'([^'\s$|][^'\s|]*)')"#
        ))
        .expect("static regex")
    });
    ARROW.captures_iter(line).any(|c| {
        let v = c.get(1).or(c.get(2)).map_or("", |m| m.as_str());
        !setting(v)
    })
}

/// In a PHP, Ruby or Perl file (R-secrets, built in): a quoted secret key `=>` a single-quoted
/// literal that starts with `$` (`'password' => '$ecret1'`). These languages never interpolate
/// in single quotes, so this is a literal, though the `$` scrub removed it; `"$x"`, `"{$x}"`,
/// `"#{x}"`, a bare `$x`, `'${X}'` and a lone `'$'` are skipped, and so is any other file (a
/// shell heredoc expands `'$X'`). `line` is the raw line.
fn arrow_dollar_literal(path: &str, line: &str) -> bool {
    static ARROW: std::sync::LazyLock<Regex> = std::sync::LazyLock::new(|| {
        Regex::new(&format!(
            r#"(?i)["']{SECRET_KEY}["']\s*=>\s*'\$[^'\s|{{][^'\s|]*'"#
        ))
        .expect("static regex")
    });
    // A password hash (bcrypt `$2y$10$`, argon2 `$argon2id$`, phpass `$P$`) is no secret.
    static HASH: std::sync::LazyLock<Regex> = std::sync::LazyLock::new(|| {
        Regex::new(r"=>\s*'\$(?:2[abxy]\$\d\d\$|argon2(?:id|i|d)\$|P\$)").expect("static regex")
    });
    let ext = path
        .rsplit('/')
        .next()
        .and_then(|n| n.rsplit_once('.'))
        .map(|(_, e)| e.to_ascii_lowercase());
    matches!(
        ext.as_deref(),
        Some("php" | "phtml" | "inc" | "rb" | "rake" | "pl" | "pm")
    ) && ARROW.is_match(line)
        && !HASH.is_match(line)
}

/// Values that are no literal secret: null, booleans, type names, permissions (`id-token:
/// write`), and Laravel casts and validation rules (`'password' => 'hashed'`).
const NOT_SECRET: [&str; 34] = [
    "null",
    "none",
    "nil",
    "true",
    "false",
    "yes",
    "no",
    "on",
    "off",
    "string",
    "str",
    "int",
    "integer",
    "number",
    "bool",
    "boolean",
    "object",
    "any",
    "read",
    "write",
    "array",
    "hashed",
    "encrypted",
    "required",
    "confirmed",
    "nullable",
    "sometimes",
    "present",
    "filled",
    "numeric",
    "json",
    "date",
    "datetime",
    "float",
];

/// A value under a secret key that is no secret: a word in `NOT_SECRET`, a duration (`1h`,
/// `720h`, `30s`) or a path (`/etc/x`, `./x`, `../x`, `~/x`).
fn setting(v: &str) -> bool {
    if NOT_SECRET.contains(&v.to_ascii_lowercase().as_str()) {
        return true;
    }
    // `strip_suffix`, not a byte slice: a value may end in a multi-byte character.
    let duration = v
        .strip_suffix(['s', 'm', 'h', 'd', 'w'])
        .is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()));
    // A pure version or range (`9.0.2`, `^9.0.2`, `~1.2`, `>=1.0`, `v1.2.3`): digits and dots.
    let ver = v.trim_start_matches(['^', '~', '=', '>', '<']);
    let ver = ver.strip_prefix('v').unwrap_or(ver);
    let version = ver.contains('.')
        && !ver.starts_with('.')
        && !ver.ends_with('.')
        && !ver.contains("..")
        && ver.bytes().all(|b| b.is_ascii_digit() || b == b'.');
    // A whole reference or interpolation once references are scrubbed (`$X`, `${..}` become
    // `$`; `#{..}`, `{$..}` become `#{$}`, `{$}`; a ruby `#{pw}`, a k8s `$(X)`); `${X}pw` and
    // `P4ss$w` are literals.
    let one = |o: &str, c: char| {
        v.strip_prefix(o)
            .and_then(|r| r.strip_suffix(c))
            .is_some_and(|r| !r.contains(['{', '}', '(', ')']))
    };
    let reference =
        v == "$" || one("#{", '}') || one("{$", '}') || one("${", '}') || one("$(", ')');
    duration || version || reference || ["/", "./", "../", "~/"].iter().any(|p| v.starts_with(p))
}

/// A plain yaml scalar that is a literal: one word (prose has spaces), not a reference, alias,
/// tag, block or flow (`$`, `*`, `&`, `!`, `|`, `>`, `{`, `[`, `%`, `@`, `<`, `~`), and no
/// `setting` (null, a boolean, a type name, `id-token: write`, a duration, a version, a path).
fn plain_literal(v: &str) -> bool {
    !v.is_empty()
        && !v.contains(char::is_whitespace)
        && !v.starts_with(['*', '&', '!', '|', '>', '{', '[', '%', '@', '<', '~', '`'])
        && !setting(v)
}

/// A `*_name`, `*-id`, `*_path` or `*_file` key (a scrub match that is no env call).
fn is_name_key(m: &str) -> bool {
    let l = m.to_ascii_lowercase();
    !l.contains(['(', '['])
        && [
            "_name", "-name", "_id", "-id", "_path", "-path", "_file", "-file",
        ]
        .iter()
        .any(|s| l.ends_with(s))
}

/// A source file, where `x = y` binds a variable: not an env, ini, yaml, toml or shell file.
fn code_file(path: &str) -> bool {
    const EXT: [&str; 31] = [
        "rs", "py", "js", "jsx", "mjs", "cjs", "ts", "tsx", "java", "kt", "kts", "scala", "go",
        "c", "h", "cc", "cpp", "hpp", "cs", "rb", "php", "swift", "m", "dart", "lua", "pl", "pm",
        "ex", "exs", "groovy", "tf",
    ];
    let name = path.rsplit('/').next().unwrap_or(path).to_ascii_lowercase();
    !name.starts_with(".env") && name.rsplit_once('.').is_some_and(|(_, e)| EXT.contains(&e))
}

/// In a code file: `name = value` where the value is a reference, not a literal: an identifier,
/// attribute, call, subscript, `None` or `null`. A string literal anywhere outside a subscript
/// or the one quoted argument of a reader call (`input("..")`, `read_file("..")`,
/// `cfg.get("k")`: one argument, no fallback) makes it a literal. A digit start is one too.
fn assignment_binding(stmt: &str) -> bool {
    static CALL: std::sync::LazyLock<Regex> = std::sync::LazyLock::new(|| {
        Regex::new(r#"([\w$]+)\(\s*(?:"[^"\\]*"|'[^'\\]*')\s*\)"#).expect("static regex")
    });
    static SUB: std::sync::LazyLock<Regex> = std::sync::LazyLock::new(|| {
        Regex::new(r#"\[\s*(?:"[^"\\]*"|'[^'\\]*')\s*\]"#).expect("static regex")
    });
    static READER: std::sync::LazyLock<Regex> = std::sync::LazyLock::new(|| {
        Regex::new(
            r"(?i)^(?:get\w*|read\w*|input|prompt\w*|load\w*|fetch\w*|lookup\w*|find\w*|ask\w*|open\w*|env|param\w*|resolve\w*|\w*value|\w*form|query\w*|cookie\w*|header\w*|arg\w*|flag\w*)$",
        )
        .expect("static regex")
    });
    static FALLBACK: std::sync::LazyLock<Regex> = std::sync::LazyLock::new(|| {
        Regex::new(r"(?:^|_)or(?:_|$)|[a-z]Or(?:[A-Z_]|$)|(?i:default|fallback|else)")
            .expect("static regex")
    });
    let t = stmt.trim_start();
    // A comment is no binding: it can hold anything.
    if ["#", "//", "/*", "* ", "--", "<!--"]
        .iter()
        .any(|c| t.starts_with(c))
    {
        return false;
    }
    let Some((_, value)) = stmt.split_once('=') else {
        return false;
    };
    let value = SUB.replace_all(value.trim(), "[]");
    let value = CALL.replace_all(&value, |c: &regex::Captures| {
        let f = &c[1];
        if READER.is_match(f) && !FALLBACK.is_match(f) && !secret_looking(&c[0]) {
            format!("{f}()")
        } else {
            c[0].to_string()
        }
    });
    // A trailing `\` continues the expression on the next line, out of sight.
    if value.trim_end().ends_with('\\') {
        return false;
    }
    let v = value.trim_start_matches(['&', '*']);
    v.starts_with(|c: char| c.is_ascii_alphabetic() || matches!(c, '_' | '@' | '$'))
        && !v.contains(['"', '\'', '`'])
}

/// The one string argument of a reader call looks like a secret: 8 or more characters, no path
/// separator, dot or space, at least one digit (a field name like `"password"` has none), and
/// not an env var name (`DB_PASSWORD`).
fn secret_looking(call: &str) -> bool {
    let arg = call
        .split_once('(')
        .map_or("", |(_, a)| a.trim_end_matches(')').trim())
        .trim_matches(['"', '\'']);
    arg.len() >= 8
        && arg.bytes().any(|b| b.is_ascii_digit())
        && !arg.contains(['/', '\\', '.', ' ', ':'])
        && !arg
            .bytes()
            .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || b == b'_')
}

/// A file as listed by GitHub, for [`unseen_diff`].
pub struct FileView {
    pub path: String,
    pub status: String,
    pub has_patch: bool,
    /// Added plus deleted lines; binary files and pure renames report 0.
    pub changes: u64,
    /// A renamed file whose blob is the previous path's blob: nothing but the name changed.
    /// GitHub lists a renamed binary that was also modified the same way (no patch, 0 changes).
    pub same_blob: bool,
}

/// `R-unseen` when bosun could not see the whole diff: GitHub listed fewer files than the PR
/// changed (it lists at most 3000), or a file with changed lines has no patch (large diffs).
/// GitHub reports 0 changed lines and no patch for a binary file: unseen too (the file name is
/// the author's choice, so it never exempts a file; a pure rename is exempt). Built in, not a rule file: a PR must not hide behind a missing diff.
pub fn unseen_diff(files: &[FileView], changed_files: u64) -> Option<Match> {
    let mut evidence = Vec::new();
    if (files.len() as u64) < changed_files {
        evidence.push(format!(
            "GitHub listed {} of {changed_files} changed files",
            files.len()
        ));
    }
    for f in files {
        // A deny-list on purpose: a status GitHub adds later counts as unseen, not as safe.
        // A binary file reports 0 changed lines, so it counts too; a pure rename (0 changes) does
        // not, its new path is judged by the rules.
        let renamed = f.status == "renamed" && f.changes == 0 && f.same_blob;
        if !matches!(f.status.as_str(), "removed" | "unchanged") && !f.has_patch && !renamed {
            evidence.push(format!("no patch for {}", f.path));
        }
    }
    (!evidence.is_empty()).then(|| Match {
        rule: "R-unseen".into(),
        why: "bosun could not see the whole diff".into(),
        approval: Approval::Owner,
        evidence,
    })
}

/// `R-bundle` when a PR that closes several tasks (`closes`) changes a path any rule gates
/// (D-016): gated work needs its own single-task PR. Re-checked on every push, since `bosun pr`
/// only sees the files once. Built in, like [`unseen_diff`].
pub fn bundle_gated(closes: usize, files: &[ChangedFile], rules: &Rules) -> Option<Match> {
    if closes < 2 {
        return None;
    }
    let evidence: Vec<String> = files
        .iter()
        .flat_map(|f| std::iter::once(&f.path).chain(f.previous_path.as_ref()))
        .filter(|p| {
            rules
                .rules
                .iter()
                .any(|r| r.paths.iter().any(|g| matches_any_case(g, p)))
        })
        .take(5)
        .map(|p| format!("path {p}"))
        .collect();
    (!evidence.is_empty()).then(|| Match {
        rule: "R-bundle".into(),
        why: "a PR that closes several tasks changes a gated path: gated work needs its own single-task PR (D-016)".into(),
        approval: Approval::Owner,
        evidence,
    })
}

/// `high` when any rule matched, else `normal`.
pub fn level(matches: &[Match]) -> &'static str {
    if matches.is_empty() { "normal" } else { "high" }
}

/// What a high risk PR needs: the owner when any match needs the owner (R-unseen always does),
/// else a delegate when any match is delegate-able, else two verdicts. No match at all is a
/// normal PR; the caller does not ask then.
pub fn requirement(matches: &[Match]) -> Approval {
    let any = |a| matches.iter().any(|m| m.approval == a);
    // A delegate never stands in for two verdicts: a mix of both classes needs the owner.
    if any(Approval::Owner)
        || matches.is_empty()
        || (any(Approval::Delegate) && any(Approval::TwoVerdicts))
    {
        Approval::Owner
    } else if any(Approval::Delegate) {
        Approval::Delegate
    } else {
        Approval::TwoVerdicts
    }
}

/// The governance paths: R-gov of the built-in rules, read from them, so `bosun pr --pr N`
/// and R-gov never disagree.
pub fn governance_globs() -> Vec<Glob> {
    let r = Rules::defaults();
    (r.rules.into_iter())
        .filter(|r| r.id == "R-gov")
        .flat_map(|r| r.paths)
        .collect()
}

/// Whether `path` matches one of `globs` as a risk rule matches it (any case).
pub fn matches_globs(globs: &[Glob], path: &str) -> bool {
    globs.iter().any(|g| matches_any_case(g, path))
}

/// Rules that are critical whatever their `approval` says: a delegate never approves them.
pub const CRITICAL_RULES: [&str; 3] = ["R-secrets", "R-prod-write", "R-unseen"];

/// Paths of the approval and risk system in every repo: a delegate never approves a change to
/// them (also when renamed from or to them). Every workflow is here: any workflow with
/// `statuses: write` can post a `risk-check` or `review-verdict` status for any PR; so can a
/// CircleCI config, which is why `.circleci/**` is here too. This only keeps a delegate from
/// approving such a change: a same-repo PR's own `on: pull_request` copy can post a forged
/// status on its head before anyone approves, so it is no owner gate (spec §8 residual; B2, a
/// check run from a dedicated GitHub App pinned in the ruleset, closes it).
pub const CRITICAL_PATHS: [&str; 19] = [
    ".bosun/**",
    "governance/**",
    "CODEOWNERS",
    ".github/CODEOWNERS",
    "docs/CODEOWNERS",
    // The required checks `hoist apply` puts in the ruleset: only the root file has `[github]`.
    "hoist.toml",
    ".github/workflows/**",
    ".github/actions/**",
    // Local composite actions (`uses: ./path`) run inside a workflow.
    "**/action.yml",
    "**/action.yaml",
    ".circleci/**",
    // Other CI providers: their config runs with the repo's secrets too.
    ".gitlab-ci.yml",
    ".gitlab/ci/**",
    ".buildkite/**",
    "**/jenkinsfile",
    "azure-pipelines.y*ml",
    "bitbucket-pipelines.yml",
    ".travis.yml",
    ".drone.yml",
];

/// bosun's own code, critical in a repo whose base has bosun's code (bosun itself and its
/// forks). All of `src/**`: the modules that judge approvals call most of the others, and any
/// of them can do I/O with the workflow's token once released. Also what builds or installs
/// the binary.
pub const APPROVAL_CODE_PATHS: [&str; 7] = [
    "src/**",
    "Cargo.toml",
    "Cargo.lock",
    "build.rs",
    "**/build.rs",
    ".cargo/**",
    "npm/**",
];

/// What a bootstrap-mode approval (D-012) never covers: the setting, governance, the risk rules,
/// the required checks and the jobs with an environment; in bosun also every
/// [`APPROVAL_CODE_PATHS`] entry (the code that checks it and verifies signatures). Apart from
/// [`CRITICAL_PATHS`], so the window is never widened or reopened from inside it.
pub const BOOTSTRAP_EXCLUDED: [&str; 13] = [
    // `.bosun/version` pins the gate release that runs the check.
    ".bosun/**",
    "governance/**",
    "CODEOWNERS",
    ".github/CODEOWNERS",
    "docs/CODEOWNERS",
    "**/hoist.toml",
    "channel/pieces/core/files/.bosun/**",
    ".github/workflows/release.yml",
    "**/workflows/splice.yml",
    // They run the approval check (root and channel copies).
    "**/workflows/risk.yml",
    "**/workflows/verdict.yml",
    "**/workflows/risk.yml.jinja",
    "**/workflows/verdict.yml.jinja",
];

/// Why a bootstrap-mode approval cannot count: the excluded paths among `(path, previous path,
/// patch)` (a rename counts on both sides), a workflow whose diff names `environment:` or is not
/// shown, and an R-secrets match. `bosun_code` adds [`APPROVAL_CODE_PATHS`]. Empty: it can count.
pub fn bootstrap_excluded(
    files: &[(&str, Option<&str>, Option<&str>)],
    matches: &[Match],
    bosun_code: bool,
) -> Vec<String> {
    let code: &[&str] = if bosun_code {
        &APPROVAL_CODE_PATHS
    } else {
        &[]
    };
    let globs: Vec<Glob> = (BOOTSTRAP_EXCLUDED.iter().chain(code))
        .map(|g| Glob::parse(g).expect("built-in bootstrap glob"))
        .collect();
    let environment = |patch: Option<&str>| {
        patch.is_none_or(|p| {
            p.lines().any(|l| {
                (l.starts_with('+') || l.starts_with('-'))
                    && l.to_ascii_lowercase().contains("environment:")
            })
        })
    };
    let mut why: Vec<String> = (files.iter())
        .filter(|(path, prev, patch)| {
            let mut both = std::iter::once(*path).chain(*prev);
            both.clone()
                .any(|p| globs.iter().any(|g| matches_any_case(g, p)))
                || (both.any(|p| p.to_ascii_lowercase().starts_with(".github/workflows/"))
                    && environment(*patch))
        })
        .map(|(path, ..)| path.to_string())
        .collect();
    if matches.iter().any(|m| m.rule == "R-secrets") {
        why.push("R-secrets".into());
    }
    why
}

/// Autonomous mode (D-017) covers only these paths by default; the owner widens or narrows the set
/// with `[approvals] autonomous_allow` on the base. The deny set below and the denied rules
/// always apply, whatever the allow set says.
pub const AUTONOMOUS_ALLOWED: [&str; 5] = [
    "src/**",
    "tests/**",
    "docs/**",
    "changelog.d/**",
    ".changeset/*.md",
];

/// Inside the allowed set, always the owner's: code that runs as root or at build time, templates
/// and the files the review and the plans trust.
pub const AUTONOMOUS_DENIED: [&str; 40] = [
    // CI and release files run with secrets; governance files decide who approves what. Kept
    // here as well as in the rules, so a repo whose rules lack them stays owner-only.
    ".github/**",
    "**/.github/**",
    ".circleci/**",
    ".bosun/**",
    "**/.bosun/**",
    ".claude/**",
    "**/.claude/**",
    // Who must approve a path: at any depth (lowercase globs match any case of the path).
    "codeowners",
    "**/codeowners",
    "cargo.toml",
    "**/cargo.toml",
    "deny.toml",
    "channel/**",
    ".cargo/**",
    "rust-toolchain*",
    "scripts/**",
    "profiles/**",
    "npm/**",
    "**/package.json",
    "*.yaml",
    "review.md",
    "**/agents.md",
    "**/claude.md",
    "**/claude.local.md",
    ".claude-plugin/**",
    ".splice/**",
    "governance/**",
    "**/hoist.toml",
    ".changeset/config.json",
    "docs/setup-script.sh",
    "docs/agents/**",
    "docs/lessons.toml",
    // The spec is the source of truth the gates and reviews are written from.
    "docs/spec/**",
    "docs/superpowers/specs/**",
    "**/plans/**",
    "**/*.jinja",
    "**/build.rs",
    // Module roots: a `mod` line there can swap a gated module for another file.
    "src/lib.rs",
    "src/main.rs",
    "src/**/mod.rs",
];

/// The rules whose match autonomous mode never covers: secrets, production writes, the gate code
/// (R-gate at base: what decides approvals, so the deny set follows the rule) and a weakened gate
/// (agents may propose relaxing criteria, never apply them), governance (who approves what),
/// dependencies, CI and the shipped CI of other repos.
pub const AUTONOMOUS_DENIED_RULES: [&str; 9] = [
    "R-secrets",
    "R-prod-write",
    "R-gate",
    "R-weaken",
    "R-gov",
    "R-deps",
    "R-ci",
    "R-ci-review",
    // A one-off task is small by definition: over the cap the owner decides.
    "R-oneoff-size",
];

/// The `R-oneoff-size` match: a one-off task changes at most [`ONE_OFF_MAX`](super::shape::ONE_OFF_MAX)
/// lines. Shared by risk-check and the autonomous signer so both see the same cap.
pub fn oneoff_size_match(one_off: bool, lines: u64) -> Option<Match> {
    use super::shape::ONE_OFF_MAX;
    (one_off && lines > ONE_OFF_MAX).then(|| Match {
        rule: "R-oneoff-size".into(),
        why: format!(
            "a one-off task is at most {ONE_OFF_MAX} changed lines; this PR changes {lines}"
        ),
        approval: Approval::Owner,
        evidence: vec![format!("{lines} changed lines")],
    })
}

/// The Rust files among `(path, patch)` whose diff is not shown or changes a line that may set a
/// `#[path]` (also inside `cfg_attr`, over several lines): `path` then `=` and a quote, spaces
/// dropped. A false hit (`let path = "x"`) only sends the PR to the owner.
pub fn path_attributes(files: &[(&str, Option<&str>)]) -> Vec<String> {
    let attr = |l: &str| {
        let t: String = l.chars().filter(|c| !c.is_whitespace()).collect();
        (l.starts_with('+') || l.starts_with('-'))
            && (t.contains("path=\"") || t.contains("path=r"))
    };
    (files.iter())
        .filter(|(p, patch)| {
            p.to_ascii_lowercase().ends_with(".rs") && patch.is_none_or(|d| d.lines().any(attr))
        })
        .map(|(p, _)| p.to_string())
        .collect()
}

/// The autonomous allow set: `[approvals] autonomous_allow` (`None`: [`AUTONOMOUS_ALLOWED`]); a
/// glob that does not parse allows nothing.
pub fn autonomous_allow_globs(allow: Option<&[String]>) -> Vec<Glob> {
    match allow {
        None => (AUTONOMOUS_ALLOWED.iter())
            .map(|g| Glob::parse(g).expect("built-in autonomous glob"))
            .collect(),
        Some(l) => l.iter().filter_map(|g| Glob::parse(g).ok()).collect(),
    }
}

/// What an autonomous-mode approval never covers among `(path, previous path)`: every path
/// outside [`AUTONOMOUS_ALLOWED`] or inside [`AUTONOMOUS_DENIED`] (a rename counts on both sides,
/// any case; a `..` segment too), and a match of [`AUTONOMOUS_DENIED_RULES`]. Empty: it can count.
/// `allow` is `[approvals] autonomous_allow` (`None`: [`AUTONOMOUS_ALLOWED`]); a glob that does not
/// parse allows nothing.
pub fn autonomous_excluded(
    files: &[(&str, Option<&str>)],
    matches: &[Match],
    allow: Option<&[String]>,
) -> Vec<String> {
    let globs = |l: &[&str]| -> Vec<Glob> {
        (l.iter())
            .map(|g| Glob::parse(g).expect("built-in autonomous glob"))
            .collect()
    };
    let deny = globs(&AUTONOMOUS_DENIED);
    let allow = autonomous_allow_globs(allow);
    let any = |gs: &[Glob], p: &str| gs.iter().any(|g| matches_any_case(g, p));
    let mut why: Vec<String> = (files.iter())
        .filter(|(path, prev)| {
            (std::iter::once(path).chain(prev))
                .any(|p| p.split('/').any(|s| s == "..") || !any(&allow, p) || any(&deny, p))
        })
        .map(|(path, _)| path.to_string())
        .collect();
    for rule in AUTONOMOUS_DENIED_RULES {
        if matches.iter().any(|m| m.rule == rule) {
            why.push(rule.into());
        }
    }
    why
}

/// The sessions whose clean verdict on `head` lets the orchestrator approve on its own (D-017):
/// `(author, body, unedited)` comments oldest first. As [`clean_verdicts`], but an edited comment
/// never counts, and any verdict by [`VERDICT_AUTHOR`] on `head` that would not count (edited too),
/// from any session and in any order, leaves nothing.
pub fn autonomous_verdicts<'a>(
    comments: impl IntoIterator<Item = (&'a str, &'a str, bool)>,
    head: &str,
    excluded: &[String],
    models: Option<&[String]>,
) -> Vec<String> {
    let all: Vec<_> = comments.into_iter().collect();
    // Any verdict on the head that would not count blocks, edited too (an edit can only make it
    // count less): a rejection, majors, a low or missing confidence. Only all-clean passes.
    let refused = all.iter().any(|(a, b, _)| {
        a.eq_ignore_ascii_case(VERDICT_AUTHOR)
            && matches!(apply_record(b), Some(Record::Verdict { head_sha, approved, confidence, major, .. })
                if head_sha == head
                    && !(approved && major == Some(0)
                        && confidence.is_some_and(|c| c >= VERDICT_MIN_CONFIDENCE)))
    });
    if refused {
        return Vec::new();
    }
    let kept = (all.into_iter()).filter_map(|(a, b, unedited)| unedited.then_some((a, b)));
    clean_verdicts(kept, head, excluded, models)
}

/// The changed paths among `(path, previous path)` pairs that are CI workflow files
/// ([`WORKFLOW_PATHS`](super::approve::WORKFLOW_PATHS)); a rename counts on both sides.
pub fn workflow_files(files: &[(&str, Option<&str>)]) -> Vec<String> {
    let globs: Vec<Glob> = (super::approve::WORKFLOW_PATHS.iter())
        .map(|g| Glob::parse(g).expect("built-in workflow glob"))
        .collect();
    (files.iter())
        .filter(|(path, prev)| {
            (std::iter::once(path).chain(prev.iter()))
                .any(|p| globs.iter().any(|g| matches_any_case(g, p)))
        })
        .map(|(path, _)| path.to_string())
        .collect()
}

/// Why a PR is critical: the critical rules it matches and the changed critical paths (a
/// rename counts on both sides). `bosun_code` adds [`APPROVAL_CODE_PATHS`]. Empty: not critical.
pub fn critical(matches: &[Match], files: &[ChangedFile], bosun_code: bool) -> Vec<String> {
    let mut why: Vec<String> = matches
        .iter()
        .filter(|m| CRITICAL_RULES.contains(&m.rule.as_str()))
        .map(|m| m.rule.clone())
        .collect();
    let code: &[&str] = if bosun_code {
        &APPROVAL_CODE_PATHS
    } else {
        &[]
    };
    let globs: Vec<Glob> = CRITICAL_PATHS
        .iter()
        .chain(code)
        .map(|g| Glob::parse(g).expect("built-in critical glob"))
        .collect();
    for f in files {
        let hit = std::iter::once(&f.path)
            .chain(f.previous_path.as_ref())
            .any(|p| globs.iter().any(|g| matches_any_case(g, p)));
        if hit {
            why.push(f.path.clone());
        }
    }
    why
}

/// A risk path also matches its lowercased form: on a case-insensitive filesystem (macOS,
/// Windows) `Secrets.toml` or `.CircleCI/` is the same file, and a wider match only asks for
/// more review. Patterns are written in lowercase where case can vary.
fn matches_any_case(g: &Glob, path: &str) -> bool {
    let lower = path.to_lowercase();
    g.matches(path)
        || g.matches(&lower)
        || Glob::parse(&g.as_str().to_lowercase()).is_ok_and(|l| l.matches(&lower))
}

/// The floor: on a critical PR every delegate match needs the owner.
pub fn apply_floor(matches: &mut [Match], critical: &[String]) {
    if !critical.is_empty() {
        for m in matches
            .iter_mut()
            .filter(|m| m.approval == Approval::Delegate)
        {
            m.approval = Approval::Owner;
        }
    }
}

/// A verdict counts only from this author: the verdict workflow writes the record comment with
/// its own token. A record an agent session posts directly is not a verdict (spec section 8).
pub const VERDICT_AUTHOR: &str = "github-actions[bot]";
/// The confidence a verdict needs to count toward two verdicts.
pub const VERDICT_MIN_CONFIDENCE: f64 = 0.9;
/// The session prefix `verdict apply` records a `[approvals] verdict_bots` verdict under.
pub const BOT_SESSION: &str = "bot:";

/// A bot's verdict sets `review-verdict` only: it never counts toward any approval (it may be the
/// author's own session writing through the app, so it is never independent).
pub fn is_bot_session(session: &str) -> bool {
    session.starts_with(BOT_SESSION)
}

/// How many verdicts of distinct sessions the two-verdicts class needs.
pub const VERDICTS_NEEDED: usize = 2;

/// The verdict record of a comment with exactly the shape `bosun verdict apply` writes: the
/// first line starts with `Review verdict: `, the record is the last non-empty line, and no
/// other line is a record. Anything else (a comment that merely contains a record) is `None`.
pub fn apply_record(body: &str) -> Option<Record> {
    let lines: Vec<&str> = body
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .collect();
    let (last, rest) = lines.split_last()?;
    let first_ok = rest
        .first()
        .is_some_and(|l| l.starts_with("Review verdict: "));
    let one_record = !rest.iter().any(|l| l.starts_with(r#"{"bosun":"#));
    if !(first_ok && one_record) {
        return None;
    }
    match serde_json::from_str::<Record>(last) {
        Ok(r @ Record::Verdict { .. }) => Some(r),
        _ => None,
    }
}

/// Sessions that hold or held a lease on a task: they did the work, so their verdict is not
/// independent. Over-counting here is safe (it only excludes), so any commenter's record counts.
pub fn lease_sessions<'a>(bodies: impl IntoIterator<Item = &'a str>) -> Vec<String> {
    bodies
        .into_iter()
        .filter_map(|b| match parse_record(b) {
            Some(Record::Lease { session, .. } | Record::LeaseEnd { session, .. }) => Some(session),
            _ => None,
        })
        .collect()
}

/// The sessions with an approving verdict on `head`, sorted. `comments` are `(author, body)`
/// oldest first. Only comments by [`VERDICT_AUTHOR`] in the shape of [`apply_record`] count; a
/// session's newest record for `head` decides, so a later disapproval (or a record without a
/// confidence) withdraws its approval. The sessions `unknown` and empty, and every session in
/// `excluded` (the task's lease holders), never count. With `models` (`[approvals] review_models`)
/// a record counts only if its self-declared model is in the list; no model, no count.
pub fn two_verdicts<'a>(
    comments: impl IntoIterator<Item = (&'a str, &'a str)>,
    head: &str,
    excluded: &[String],
    models: Option<&[String]>,
) -> Vec<String> {
    verdict_sessions(comments, head, excluded, models, |_| true)
}

/// The sessions whose verdict on `head` lets a delegate approve: as [`two_verdicts`], and the
/// record counts its major findings and has none (a blocker never approves). Older records
/// without the count never count.
pub fn clean_verdicts<'a>(
    comments: impl IntoIterator<Item = (&'a str, &'a str)>,
    head: &str,
    excluded: &[String],
    models: Option<&[String]>,
) -> Vec<String> {
    verdict_sessions(comments, head, excluded, models, |major| major == Some(0))
}

fn verdict_sessions<'a>(
    comments: impl IntoIterator<Item = (&'a str, &'a str)>,
    head: &str,
    excluded: &[String],
    models: Option<&[String]>,
    clean: impl Fn(Option<u32>) -> bool,
) -> Vec<String> {
    let mut newest: std::collections::BTreeMap<String, bool> = Default::default();
    for (author, body) in comments {
        if !author.eq_ignore_ascii_case(VERDICT_AUTHOR) {
            continue;
        }
        if let Some(Record::Verdict {
            session,
            head_sha,
            approved,
            confidence,
            major,
            model,
        }) = apply_record(body)
            && head_sha == head
            && !session.is_empty()
            && session != "unknown"
            && !is_bot_session(&session)
            && !excluded.contains(&session)
        {
            let enrolled = models.is_none_or(|m| model.is_some_and(|x| m.contains(&x)));
            let counts = approved
                && enrolled
                && confidence.is_some_and(|c| c >= VERDICT_MIN_CONFIDENCE)
                && clean(major);
            newest.insert(session, counts);
        }
    }
    newest
        .into_iter()
        .filter_map(|(s, ok)| ok.then_some(s))
        .collect()
}

const CARGO_TABLES: [&str; 3] = ["dependencies", "dev-dependencies", "build-dependencies"];
const NPM_TABLES: [&str; 4] = [
    "dependencies",
    "devDependencies",
    "optionalDependencies",
    "peerDependencies",
];

/// Fields of a Cargo dependency that say where its code comes from. `version` is left out: a
/// version bump of a crate already in the tree is not new code from a new place.
const CARGO_SOURCE_KEYS: [&str; 7] = [
    "git",
    "rev",
    "tag",
    "branch",
    "registry",
    "registry-index",
    "package",
];

/// The source of one Cargo dependency entry (empty: the default registry), or `None` when it
/// adds no code from outside the repo: `x.workspace = true` inherits a root pin (the pin counts
/// in the root manifest), and a `path` entry without `git` is code in this repo, in the diff.
fn cargo_source(v: &toml::Value) -> Option<String> {
    let Some(t) = v.as_table() else {
        return Some(String::new());
    };
    let inherits = t.get("workspace").and_then(toml::Value::as_bool) == Some(true);
    if inherits || (t.contains_key("path") && !t.contains_key("git")) {
        return None;
    }
    let parts: Vec<String> = CARGO_SOURCE_KEYS
        .iter()
        .filter_map(|k| t.get(*k).map(|x| format!("{k}={x}")))
        .collect();
    Some(parts.join(" "))
}

/// The source class of an npm version spec: a semver range or tag is the registry (empty);
/// anything else (git, a URL, `npm:` alias, `file:`, `link:`, `user/repo`) is itself.
fn npm_source(spec: &str) -> String {
    let s = spec.trim();
    let range = s
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || " .*^~<>=|-+x".contains(c));
    if range { String::new() } else { s.to_string() }
}

/// A YAML key with its quotes and spaces removed, and the rest of the line after its colon.
/// `None` when the line is not `key: rest`.
fn yaml_pair(body: &str) -> Option<(String, &str)> {
    let (key, rest) = match body.chars().next()? {
        q @ ('"' | '\'') => {
            let end = body[1..].find(q)? + 1;
            let rest = body[end + 1..].trim_start().strip_prefix(':')?;
            (body[1..end].to_string(), rest)
        }
        _ => {
            let i = body.find(':')?;
            (body[..i].trim().to_string(), &body[i + 1..])
        }
    };
    let rest = rest.trim();
    // A key: value pair needs a space after the colon, or nothing (`a:b` is a plain scalar).
    if !rest.is_empty() && !body.contains(": ") && !body.contains(":\t") {
        return None;
    }
    Some((key, rest))
}

/// Top-level `pnpm-workspace.yaml` blocks whose entries are dependencies or redirect them.
/// `catalogs` holds named catalogs one level deeper.
const PNPM_BLOCKS: [&str; 4] = ["catalog", "catalogs", "overrides", "patchedDependencies"];

/// The entries of the dependency blocks of a `pnpm-workspace.yaml` (`catalog`, `catalogs`,
/// `overrides`, `patchedDependencies`): a small line reader for block-style `name: spec` lines
/// (bosun has no YAML parser). Fails closed: any line in or opening such a block that it cannot
/// read (flow style, an alias or anchor with content, a merge key, odd nesting) counts as a
/// dependency named by the line, so a change to it is never missed.
fn pnpm_workspace(text: &str, specs: &mut BTreeMap<String, String>) {
    let unread = |specs: &mut BTreeMap<String, String>, line: &str| {
        specs.insert(
            format!("pnpm-workspace.yaml: {}", line.trim()),
            String::new(),
        );
    };
    // The open block, and for `catalogs` the named catalog.
    let mut block: Option<String> = None;
    let mut named: Option<String> = None;
    let mut name_indent: Option<usize> = None;
    let mut entry_indent: Option<usize> = None;
    for raw in text.lines() {
        let line = raw.find(" #").map_or(raw, |i| &raw[..i]).trim_end();
        let body = line.trim_start();
        if body.is_empty() || body.starts_with('#') || body == "---" {
            continue;
        }
        let indent = line.len() - body.len();
        if indent == 0 {
            (block, named, name_indent, entry_indent) = (None, None, None, None);
            let Some((key, rest)) = yaml_pair(body) else {
                continue;
            };
            if !PNPM_BLOCKS.contains(&key.as_str()) && !key.starts_with("catalog") {
                continue;
            }
            // An anchor alone (`catalog: &c`) still opens the block; any other value is unread.
            let opens = rest.is_empty() || (rest.starts_with('&') && !rest.contains(' '));
            if opens && PNPM_BLOCKS.contains(&key.as_str()) {
                block = Some(key);
            } else {
                unread(specs, body);
            }
            continue;
        }
        let Some(b) = block.as_deref() else { continue };
        let Some((key, rest)) = yaml_pair(body) else {
            unread(specs, body);
            continue;
        };
        let simple = !key.starts_with("<<") && !rest.starts_with(['&', '*', '{', '[', '|', '>']);
        if b == "catalogs" && *name_indent.get_or_insert(indent) == indent {
            // A named catalog: `name:` (an anchor may follow); its entries sit deeper.
            let opens = rest.is_empty() || (rest.starts_with('&') && !rest.contains(' '));
            if opens && !key.starts_with("<<") {
                (named, entry_indent) = (Some(key), None);
            } else {
                unread(specs, body);
            }
            continue;
        }
        let depth_ok = *entry_indent.get_or_insert(indent) == indent;
        let in_catalog = b != "catalogs" || named.is_some();
        if simple && !rest.is_empty() && depth_ok && in_catalog {
            let value = rest.trim_matches(['\'', '"']);
            let (name, source) = match b {
                "patchedDependencies" => (format!("{b}/{key}"), value.to_string()),
                "overrides" => (format!("{b}/{key}"), npm_source(value)),
                _ => (key, npm_source(value)),
            };
            let spec = specs.entry(name).or_default();
            if !spec.split('\n').any(|s| s == source) {
                spec.push_str(&source);
                spec.push('\n');
            }
        } else {
            unread(specs, body);
        }
    }
}

/// Dependencies a `Cargo.toml`, `package.json` or `pnpm-workspace.yaml` text declares (by the
/// file name of `path`), each with its source: where its code comes from, not its version.
/// Included: the dependency tables (and their `target` tables), `[workspace.dependencies]`,
/// `[patch.*]` (as `patch.<registry>/<name>`) and `[replace]` (as `replace/<spec>`) in Cargo;
/// the dependency maps, `overrides`, `resolutions`, `pnpm.overrides` and
/// `pnpm.patchedDependencies` in npm; the catalogs, `overrides` and `patchedDependencies` of a
/// pnpm workspace. Edges that add no outside code are left out: Cargo `workspace = true` and
/// in-repo `path` entries, and npm `catalog:` and `workspace:` references (the root pins and the
/// catalogs count where they are declared). Other files and unparsable texts have none.
pub fn dependency_specs(path: &str, text: &str) -> BTreeMap<String, String> {
    let mut specs = BTreeMap::new();
    match path.rsplit('/').next().unwrap_or(path) {
        "Cargo.toml" => {
            let Ok(doc) = toml::from_str::<toml::Table>(text) else {
                return specs;
            };
            fn table(v: Option<&toml::Value>) -> Option<&toml::Table> {
                v.and_then(toml::Value::as_table)
            }
            let mut tables: Vec<(String, &toml::Table)> = Vec::new();
            let mut scopes = vec![&doc];
            scopes.extend(
                table(doc.get("target"))
                    .into_iter()
                    .flat_map(|t| t.values())
                    .filter_map(toml::Value::as_table),
            );
            for scope in scopes {
                for key in CARGO_TABLES {
                    tables.extend(table(scope.get(key)).map(|t| (String::new(), t)));
                }
            }
            let ws = table(doc.get("workspace"));
            tables
                .extend(table(ws.and_then(|w| w.get("dependencies"))).map(|t| (String::new(), t)));
            for (registry, t) in table(doc.get("patch")).into_iter().flatten() {
                tables.extend(t.as_table().map(|t| (format!("patch.{registry}/"), t)));
            }
            tables.extend(table(doc.get("replace")).map(|t| ("replace/".to_string(), t)));
            for (prefix, t) in tables {
                for (name, v) in t {
                    if let Some(source) = cargo_source(v) {
                        specs.insert(format!("{prefix}{name}"), source);
                    }
                }
            }
        }
        "package.json" => {
            let Ok(doc) = serde_json::from_str::<serde_json::Value>(text) else {
                return specs;
            };
            let pnpm = doc.get("pnpm");
            let maps = NPM_TABLES.iter().map(|k| ("", doc.get(*k))).chain([
                ("overrides/", doc.get("overrides")),
                ("resolutions/", doc.get("resolutions")),
                ("overrides/", pnpm.and_then(|p| p.get("overrides"))),
                (
                    "patchedDependencies/",
                    pnpm.and_then(|p| p.get("patchedDependencies")),
                ),
            ]);
            for (prefix, map) in maps {
                for (name, v) in map.and_then(|m| m.as_object()).into_iter().flatten() {
                    // npm `overrides` may nest objects: the whole value is the source.
                    let spec = v.as_str().map_or_else(|| v.to_string(), str::to_string);
                    if spec.starts_with("catalog:") || spec.starts_with("workspace:") {
                        continue;
                    }
                    let source = match prefix {
                        "patchedDependencies/" => spec,
                        _ => npm_source(&spec),
                    };
                    specs.insert(format!("{prefix}{name}"), source);
                }
            }
        }
        "pnpm-workspace.yaml" => pnpm_workspace(text, &mut specs),
        _ => {}
    }
    specs
}

/// Dependencies that add outside code, over `(path, base text, head text)` triples: a name
/// declared at head but not at base (`x`), or one whose source changed (`x (source changed)`:
/// registry to git, a `package =` rename, an `npm:` alias...). A version bump is neither. A
/// manifest missing at base is all new.
pub fn new_dependencies(manifests: &[(String, Option<String>, Option<String>)]) -> Vec<String> {
    let mut out = BTreeSet::new();
    for (path, base, head) in manifests {
        let old = base
            .as_deref()
            .map_or_else(BTreeMap::new, |t| dependency_specs(path, t));
        let new = head
            .as_deref()
            .map_or_else(BTreeMap::new, |t| dependency_specs(path, t));
        for (name, source) in new {
            match old.get(&name) {
                None => out.insert(name),
                Some(o) if *o != source => out.insert(format!("{name} (source changed)")),
                Some(_) => false,
            };
        }
    }
    out.into_iter().collect()
}

/// A repo owner from `governance/owners.toml`: exactly one of a user (`github`) or a team
/// (`team = "<org>/<slug>"`, spec §16). A team counts no member until expansion is built (fail
/// closed), so `owner_logins` holds users only.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Owner {
    #[serde(default)]
    pub github: Option<String>,
    #[serde(default)]
    pub team: Option<String>,
    #[serde(default)]
    pub slack: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawOwners {
    #[serde(default)]
    owner: Vec<Owner>,
}

pub fn parse_owners(text: &str) -> Result<Vec<Owner>, String> {
    let owners = toml::from_str::<RawOwners>(text)
        .map(|o| o.owner)
        .map_err(|e| format!("governance/owners.toml: {e}"))?;
    for (i, o) in owners.iter().enumerate() {
        let team_ok = o.team.as_deref().is_some_and(|t| {
            t.split_once('/')
                .is_some_and(|(a, b)| !a.is_empty() && !b.is_empty() && !b.contains('/'))
        });
        match (&o.github, &o.team) {
            (Some(_), None) => {}
            (None, Some(_)) if team_ok && o.slack.is_none() => {}
            _ => {
                return Err(format!(
                    "governance/owners.toml: [[owner]] {} needs exactly one of `github` or `team = \"<org>/<slug>\"` (`slack` only with `github`)",
                    i + 1
                ));
            }
        }
    }
    Ok(owners)
}

/// Why `entries` cannot be the owners of `repo_owner`'s repo: each is a login or a
/// `<repo_owner>/<slug>` team, no duplicates, at least one login (a team is unreadable to
/// sessions, spec §16).
pub fn owner_entries_error(entries: &[String], repo_owner: &str) -> Option<String> {
    let name = |s: &str, extra: &[char]| {
        !s.is_empty()
            && s.len() <= 39
            && s.chars()
                .all(|c| c.is_ascii_alphanumeric() || extra.contains(&c))
    };
    let mut seen = std::collections::BTreeSet::new();
    for e in entries {
        let ok = match e.split_once('/') {
            Some((org, slug)) => org.eq_ignore_ascii_case(repo_owner) && name(slug, &['-', '_']),
            None => name(e, &['-']),
        };
        if !ok {
            return Some(format!(
                "`{e}` is neither a GitHub login nor a team `{repo_owner}/<slug>`"
            ));
        }
        if !seen.insert(e.to_ascii_lowercase()) {
            return Some(format!("`{e}` is listed twice"));
        }
    }
    (!entries.iter().any(|e| !e.contains('/'))).then(|| {
        "no login: a team alone may be unreadable, so list at least one owner by login".into()
    })
}

/// [`parse_owners`] for the repo of `org`: a team of another org is a parse error too (spec §16).
pub fn parse_owners_in(text: &str, org: &str) -> Result<Vec<Owner>, String> {
    let owners = parse_owners(text)?;
    let other = |t: &&str| {
        !t.split('/')
            .next()
            .is_some_and(|o| o.eq_ignore_ascii_case(org))
    };
    match owners.iter().filter_map(|o| o.team.as_deref()).find(other) {
        Some(t) => Err(format!(
            "governance/owners.toml: team `{t}` is not of the repo's org {org}"
        )),
        None => Ok(owners),
    }
}

/// `governance/owners.toml` for `entries` (logins, or `<org>/<slug>` teams), keeping the Slack ID
/// a login has in `old` (compared case-insensitively).
pub fn render_owners(entries: &[String], old: &[Owner]) -> String {
    let mut s = String::from("# Repo owners (spec §16), written by `bosun owners set`.\n");
    for e in entries {
        if e.contains('/') {
            s.push_str(&format!("\n[[owner]]\nteam = \"{e}\"\n"));
            continue;
        }
        s.push_str(&format!("\n[[owner]]\ngithub = \"{e}\"\n"));
        let slack = old.iter().find(|o| {
            o.github
                .as_deref()
                .is_some_and(|g| g.eq_ignore_ascii_case(e))
        });
        if let Some(id) = slack.and_then(|o| o.slack.as_deref()) {
            s.push_str(&format!("slack = \"{id}\"\n"));
        }
    }
    s
}

const NOTE_OPEN: &str = "<!-- bosun:risk -->";
const NOTE_CLOSE: &str = "<!-- /bosun:risk -->";

/// `s` with backticks, angle brackets and anything but printable ASCII as `?`, for file names in a PR body.
pub fn plain(s: &str) -> String {
    // Printable ASCII only: no bidi override, line separator or zero-width character survives.
    let bad = |c: char| matches!(c, '`' | '<' | '>') || !(c == ' ' || c.is_ascii_graphic());
    s.chars().map(|c| if bad(c) { '?' } else { c }).collect()
}

/// The Risk note: the rules that matched, why, the evidence, and what the owner must check.
pub fn risk_note(matches: &[Match], head_sha: &str) -> String {
    let mut s = format!("{NOTE_OPEN}\n## Risk: high\n\n");
    for m in matches {
        s.push_str(&format!("- **{}**: {}\n", m.rule, m.why));
        for e in &m.evidence {
            s.push_str(&format!("  - `{}`\n", plain(e))); // evidence names files
        }
    }
    let ask = match requirement(matches) {
        Approval::Owner => format!(
            "Owner: review the head SHA {head_sha} and approve. An approval on an older commit does not count."
        ),
        Approval::TwoVerdicts => format!(
            "Needed: two independent review verdicts (different sessions, approve with confidence >= {VERDICT_MIN_CONFIDENCE}) on the head SHA {head_sha}, or an owner approval. Verdicts on an older commit do not count."
        ),
        Approval::Delegate => format!(
            "Needed: an owner approval of the head SHA {head_sha}, or a delegate's signed approval with one independent review verdict (approve, confidence >= {VERDICT_MIN_CONFIDENCE}, no major finding) on it."
        ),
    };
    s.push_str(&format!("\n{ask}\n{NOTE_CLOSE}"));
    s
}

/// The files the Risk note of a PR body names as evidence, in order, without repeats.
pub fn note_paths(body: &str) -> Vec<String> {
    let (Some(a), Some(b)) = (body.find(NOTE_OPEN), body.find(NOTE_CLOSE)) else {
        return Vec::new();
    };
    let mut out: Vec<String> = Vec::new();
    for line in body.get(a..b).unwrap_or_default().lines() {
        if let Some(path) = line
            .strip_prefix("  - `")
            .and_then(|r| r.strip_suffix('`'))
            .filter(|p| !p.is_empty())
            && !out.iter().any(|o| o == path)
        {
            out.push(path.to_string());
        }
    }
    out
}

/// `body` with the Risk note replaced by `note` (or appended); `None` removes it.
pub fn replace_note(body: &str, note: Option<&str>) -> String {
    let (before, after) = match (body.find(NOTE_OPEN), body.find(NOTE_CLOSE)) {
        (Some(a), Some(b)) if a < b => (&body[..a], &body[b + NOTE_CLOSE.len()..]),
        _ => (body, ""),
    };
    let rest = format!("{}{}", before.trim_end(), after);
    match note {
        Some(n) if rest.trim().is_empty() => n.to_string(),
        Some(n) => format!("{}\n\n{n}", rest.trim_end()),
        None => rest,
    }
}

/// Capabilities named by `bosun:capability/<c>` labels.
pub fn capability_labels<S: AsRef<str>>(labels: &[S]) -> Vec<String> {
    labels
        .iter()
        .filter_map(|l| l.as_ref().strip_prefix("bosun:capability/"))
        .map(str::to_string)
        .collect()
}

/// Whether a PR comment approves `head`: its trimmed body is exactly `/bosun approve <head>`
/// with the full 40-character SHA.
pub fn comment_approves(body: &str, head: &str) -> bool {
    head.len() == 40 && body.trim() == format!("/bosun approve {head}")
}

/// The distinct N of the `<!-- bosun:revert N -->` lines of a PR body (digits only), in order.
pub fn revert_targets(body: &str) -> Vec<u64> {
    let mut out: Vec<u64> = Vec::new();
    for part in body.split("<!-- bosun:revert ").skip(1) {
        let n = part.split_once(" -->").map(|(n, _)| n);
        let n = n
            .filter(|n| n.bytes().all(|b| b.is_ascii_digit()))
            .and_then(|n| n.parse().ok());
        out.extend(n.filter(|n| !out.contains(n)));
    }
    out
}

/// One file of a PR as GitHub lists it, for `inverse_problems`.
pub struct PatchFile<'a> {
    pub path: &'a str,
    pub previous_path: Option<&'a str>,
    pub status: &'a str,
    pub patch: Option<&'a str>,
    pub changes: u64,
}

/// A patch as tokens: `@@` with the hunk's start lines, a `\ No newline` line glued to the line
/// before it, `+`/`-` swapped when `flip`, each run of changed lines sorted `-` before `+`.
fn tokens(patch: &str, flip: bool) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut run: Vec<String> = Vec::new();
    for l in patch.lines() {
        let t = match l.chars().next() {
            Some('\\') => {
                if let Some(t) = run.last_mut().or(out.last_mut()) {
                    t.push('\u{0}');
                }
                continue;
            }
            Some(c @ ('+' | '-')) => {
                let c = match (flip, c) {
                    (true, '+') => '-',
                    (true, _) => '+',
                    _ => c,
                };
                run.push(format!("{c}{}", &l[1..]));
                continue;
            }
            // The hunk's start lines: #N's new side is the revert's old side.
            _ if l.starts_with("@@") => {
                let at = |i: usize| l.split(' ').nth(i).and_then(|w| w[1..].split(',').next());
                let (a, b) = (at(1).unwrap_or("?"), at(2).unwrap_or("?"));
                if flip {
                    format!("@@ {b} {a}")
                } else {
                    format!("@@ {a} {b}")
                }
            }
            _ => l.to_string(),
        };
        run.sort_by_key(|t| t.starts_with('+'));
        out.append(&mut run);
        out.push(t);
    }
    run.sort_by_key(|t| t.starts_with('+'));
    out.append(&mut run);
    out
}

/// Why `revert` is not the exact inverse of `merged` (empty: it is), at most 5. Fails closed:
/// unpaired files, other status pairs, a missing patch (also a rename), a differing patch.
pub fn inverse_problems(merged: &[PatchFile], revert: &[PatchFile]) -> Vec<String> {
    let inverse = [
        "added removed",
        "removed added",
        "modified modified",
        "renamed renamed",
    ];
    let (mut out, mut used) = (Vec::new(), vec![false; merged.len()]);
    for r in revert {
        let pairs = |m: &PatchFile| {
            m.path == r.path
                || (r.status == "renamed"
                    && Some(m.path) == r.previous_path
                    && m.previous_path == Some(r.path))
        };
        let hits: Vec<usize> = (0..merged.len()).filter(|&i| pairs(&merged[i])).collect();
        let why: String = match hits[..] {
            [i] if !std::mem::replace(&mut used[i], true) => {
                let (m, st) = (&merged[i], format!("{} {}", merged[i].status, r.status));
                match (m.patch, r.patch) {
                    _ if !inverse.contains(&st.as_str()) => format!("status {st} is not inverse"),
                    (Some(a), Some(b)) if tokens(a, true) == tokens(b, false) => continue,
                    (Some(_), Some(_)) => "the patch is not the inverse".into(),
                    _ => format!("no patch for {}", r.path),
                }
            }
            _ => "no single counterpart in the reverted PR".into(),
        };
        out.push(format!("{}: {why}", r.path));
    }
    let left = merged.iter().zip(&used).filter(|(_, u)| !**u);
    out.extend(
        left.map(|(m, _)| format!("{}: changed by the reverted PR, not the revert", m.path)),
    );
    out.truncate(5);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bootstrap_excludes_every_approval_code_path_in_bosun_only() {
        for g in APPROVAL_CODE_PATHS {
            let path = g.replace("**", "x/y");
            let f = [(path.as_str(), None, Some("@@\n+x"))];
            assert_eq!(
                bootstrap_excluded(&f, &[], true),
                std::slice::from_ref(&path),
                "{g}"
            );
            assert!(
                bootstrap_excluded(&f, &[], false).is_empty(),
                "{g} in a consumer repo"
            );
        }
    }

    #[test]
    fn autonomous_covers_only_the_allowlist_minus_the_denied() {
        let ok = [
            "src/cli/claim.rs",
            "tests/cli_x.rs",
            "docs/guide.md",
            "changelog.d/x.md",
            ".changeset/x.md",
            ".changeset/README.md",
        ];
        let no = [
            // Outside the narrow default allow set.
            "Cargo.toml",
            "crates/x/Cargo.toml",
            "deny.toml",
            "channel/pieces/core/files/src/a.rs",
            "scripts/x.sh",
            "profiles/rust.toml",
            "npm/x/index.js",
            "package.json",
            "web/package.json",
            "ci.yaml",
            "REVIEW.md",
            "AGENTS.md",
            "docs/AGENTS.md",
            "x/CLAUDE.md",
            "rust-toolchain.toml",
            ".cargo/config.toml",
            "sub/.github/workflows/x.yml",
            "sub/.bosun/x.toml",
            "sub/.claude/x.json",
            "other/x.txt",
            ".github/workflows/ci.yml",
            ".github/actions/x/action.yml",
            ".circleci/config.yml",
            ".bosun/risk.toml",
            ".claude/settings.json",
            "governance/owners.toml",
            "hoist.toml",
            ".changeset/config.json",
            "build.rs",
            "src/x/build.rs",
            "src/a/b.rs.jinja",
            "docs/agents/org-decisions.md",
            "docs/lessons.toml",
            "docs/setup-script.sh",
            "docs/superpowers/plans/p.md",
            "docs/spec/08-risk.md",
            "docs/superpowers/specs/s.md",
            ".claude-plugin/plugin.json",
            "x/Claude.Local.md",
            "SRC/Build.RS",
            "Docs/Agents/X.md",
            // Module roots can swap a gated module for another file.
            "src/lib.rs",
            "src/main.rs",
            "src/cli/mod.rs",
            "CODEOWNERS",
            "docs/CODEOWNERS",
            "src/CODEOWNERS",
            "docs/x/codeowners",
        ];
        for p in ok {
            assert!(
                autonomous_excluded(&[(p, None)], &[], None).is_empty(),
                "{p}"
            );
        }
        for p in no {
            assert_eq!(autonomous_excluded(&[(p, None)], &[], None), [p], "{p}");
        }
        // `[approvals] autonomous_allow` narrows the default; the deny set still wins.
        // The owner widens the default through the base config; the deny set still wins.
        let wide: Vec<String> = ["**".into()].into();
        assert!(autonomous_excluded(&[("other/x.txt", None)], &[], Some(&wide)).is_empty());
        assert_eq!(
            autonomous_excluded(&[("Cargo.toml", None)], &[], Some(&wide)),
            ["Cargo.toml"]
        );
        let narrow: Vec<String> = ["src/**".into(), "plans/**".into(), "[".into()].into();
        let ex = |p: &str| autonomous_excluded(&[(p, None)], &[], Some(&narrow));
        assert!(ex("src/a.rs").is_empty());
        assert_eq!(ex("Cargo.toml"), ["Cargo.toml"]);
        assert_eq!(ex("plans/x.md"), ["plans/x.md"]);
        assert_eq!(ex("src/lib.rs"), ["src/lib.rs"]);
        let none: Vec<String> = Vec::new();
        assert_eq!(
            autonomous_excluded(&[("src/a.rs", None)], &[], Some(&none)).len(),
            1
        );
        // Gate code goes to the owner: every path of the default R-gate rule is a match that
        // autonomous mode refuses, so the deny set follows R-gate.
        let gate = Rules::defaults()
            .rules
            .into_iter()
            .find(|r| r.id == "R-gate");
        let gate = gate.expect("R-gate");
        assert!(!gate.paths.is_empty());
        for g in &gate.paths {
            let p = g.as_str().replace("**", "x.rs");
            let m = eval(&[file(&p, &[])]);
            assert!(
                autonomous_excluded(&[(&p, None)], &m, None).contains(&"R-gate".to_string()),
                "{p}"
            );
        }
        // A rename counts on both sides.
        let f = [("src/a.rs", Some(".github/workflows/ci.yml"))];
        let narrow: Vec<String> = ["src/**".into(), "docs/**".into()].into();
        assert_eq!(autonomous_excluded(&f, &[], Some(&narrow)), ["src/a.rs"]);
        let f = [(".bosun/x.toml", Some("src/a.rs"))];
        assert_eq!(autonomous_excluded(&f, &[], Some(&narrow)).len(), 1);
        // A rename out of a denied path is denied.
        let f = [("src/a.rs", Some("docs/agents/x.md"))];
        assert_eq!(autonomous_excluded(&f, &[], None), ["src/a.rs"]);
        let m = |rule: &str| Match {
            rule: rule.into(),
            why: String::new(),
            approval: Approval::Owner,
            evidence: Vec::new(),
        };
        let f = [("src/a.rs", None)];
        assert_eq!(
            autonomous_excluded(&f, &[m("R-secrets")], None),
            ["R-secrets"]
        );
        assert_eq!(
            autonomous_excluded(&f, &[m("R-prod-write")], None),
            ["R-prod-write"]
        );
        assert_eq!(
            autonomous_excluded(&f, &[m("R-weaken")], None),
            ["R-weaken"]
        );
        assert_eq!(autonomous_excluded(&f, &[m("R-gov")], None), ["R-gov"]);
        assert_eq!(
            autonomous_excluded(&f, &[m("R-oneoff-size")], None),
            ["R-oneoff-size"]
        );
        for r in ["R-deps", "R-ci", "R-ci-review"] {
            assert_eq!(autonomous_excluded(&f, &[m(r)], None), [r]);
        }
        // A `#[path]` attribute in a changed Rust line, or an unseen Rust diff, goes to the owner.
        let some = Some("@@\n+fn a() {}");
        let attr = Some("@@\n+#[cfg_attr(x,\n+   path = \"other.rs\")]\n+mod risk;");
        assert!(path_attributes(&[("src/a.rs", some)]).is_empty());
        assert!(path_attributes(&[("docs/a.md", None)]).is_empty());
        assert_eq!(path_attributes(&[("src/a.rs", attr)]), ["src/a.rs"]);
        assert_eq!(path_attributes(&[("src/a.RS", None)]), ["src/a.RS"]);
        let attr = Some("@@\n-#[path = \"x.rs\"]");
        assert_eq!(path_attributes(&[("src/a.rs", attr)]).len(), 1);
    }

    #[test]
    fn autonomous_verdicts_skip_edits_and_stop_at_any_request_for_changes() {
        let h = "a".repeat(40);
        let v = |s: &str, head: &str, approved: bool| {
            let r = serde_json::json!({"bosun": "verdict", "session": s, "head_sha": head,
                "approved": approved, "confidence": 0.95, "major": 0});
            format!("Review verdict: approve 0.95, 0 blocker.\n\n{r}")
        };
        let (a, b) = (v("a", &h, true), v("b", &h, false));
        let (other, bad) = (v("c", &"b".repeat(40), false), v("d", &h, true));
        let (a, b, other, bad) = (a.as_str(), b.as_str(), other.as_str(), bad.as_str());
        let run = |c: &[(&str, &str, bool)], ex: &[String]| {
            autonomous_verdicts(c.iter().copied(), &h, ex, None)
        };
        let bot = VERDICT_AUTHOR;
        assert_eq!(run(&[(bot, a, true)], &[]), ["a"]);
        assert!(run(&[(bot, a, false)], &[]).is_empty());
        // A rejection from any session, before or after, or from an excluded one.
        assert!(run(&[(bot, b, true), (bot, a, true)], &[]).is_empty());
        assert!(run(&[(bot, a, true), (bot, b, true)], &[]).is_empty());
        assert!(run(&[(bot, a, true), (bot, b, true)], &["b".into()]).is_empty());
        // An edited rejection still blocks (fail closed); one on another head or by someone
        // else does not.
        assert!(run(&[(bot, a, true), (bot, b, false)], &[]).is_empty());
        assert_eq!(run(&[(bot, a, true), (bot, other, true)], &[]), ["a"]);
        assert_eq!(run(&[("mallory", b, true), (bot, a, true)], &[]), ["a"]);
        assert_eq!(
            run(&[(bot, a, true), (bot, bad, true)], &["d".into()]),
            ["a"]
        );
        // An approval that would not count blocks too: majors or low confidence.
        let weak = |c: f64, m: u32| {
            let r = serde_json::json!({"bosun": "verdict", "session": "e", "head_sha": &h,
                "approved": true, "confidence": c, "major": m});
            format!("Review verdict: approve.\n\n{r}")
        };
        for w in [weak(0.95, 1), weak(0.1, 0)] {
            assert!(
                run(&[(bot, a, true), (bot, &w, true)], &[]).is_empty(),
                "{w}"
            );
        }
    }

    #[test]
    fn the_approval_path_and_the_hooks_are_gate_code() {
        for p in [
            "src/core/carry.rs",
            "src/core/lockmerge.rs",
            "src/core/touch.rs",
            "src/core/text.rs",
            "src/core/shape.rs",
            "src/core/plan.rs",
            "src/core/guard.rs",
            "src/core/repeat.rs",
            "src/cli/carry_over.rs",
            "src/cli/pr_bundle.rs",
            "src/cli/plan.rs",
            "src/hooks/pre_tool_use.rs",
            // The gate stamps, their records and the CI-minutes gate decide what passes too.
            "src/core/ci_minutes.rs",
            "src/cli/ci_minutes.rs",
            "src/core/stamps.rs",
            "src/cli/stamps.rs",
            "src/core/gate_record.rs",
        ] {
            assert!(ids(&eval(&[file(p, &[])])).contains(&"R-gate"), "{p}");
        }
    }

    fn file(path: &str, added: &[&str]) -> ChangedFile {
        ChangedFile {
            path: path.into(),
            previous_path: None,
            added_lines: added.iter().map(|s| s.to_string()).collect(),
        }
    }

    fn ids(m: &[Match]) -> Vec<&str> {
        m.iter().map(|m| m.rule.as_str()).collect()
    }

    fn eval(files: &[ChangedFile]) -> Vec<Match> {
        evaluate(&Rules::defaults(), files, &[], &[])
    }

    #[test]
    fn defaults_have_the_spec_rules() {
        let r = Rules::defaults();
        let got: Vec<&str> = r.rules.iter().map(|r| r.id.as_str()).collect();
        assert_eq!(
            got,
            [
                "R-gov",
                "R-ci",
                "R-gate",
                "R-ci-review",
                "R-secrets",
                "R-deps",
                "R-prod-write",
                "R-weaken"
            ]
        );
        assert_eq!(r.rules[0].why, "Governance files decide who approves what.");
    }

    #[test]
    fn path_rules_match_new_or_old_path() {
        let m = eval(&[file(".github/workflows/ci.yml", &[]), file("src/a.rs", &[])]);
        assert_eq!(ids(&m), ["R-ci"]);
        let mut renamed = file("docs/owners.toml", &[]);
        renamed.previous_path = Some("governance/owners.toml".into());
        assert_eq!(ids(&eval(&[renamed])), ["R-gov"]);
        assert_eq!(ids(&eval(&[file(".github/CODEOWNERS", &[])])), ["R-gov"]);
        // A local composite action (`uses: ./path`) runs inside a workflow.
        for p in ["tools/x/action.yml", "action.yaml", "ci/Setup/Action.YML"] {
            assert_eq!(ids(&eval(&[file(p, &[])])), ["R-ci"], "{p}");
        }
        assert!(eval(&[file("src/governance.rs", &[])]).is_empty());
    }

    #[test]
    fn content_rule_reads_added_lines_only() {
        let m = eval(&[file("a.env", &["API_KEY = abc"])]);
        assert_eq!(ids(&m), ["R-secrets"]);
        assert!(!m[0].evidence[0].contains("abc"));
        assert!(eval(&[file("a.env", &["let tokens = 3;"])]).is_empty());
        let added = added_lines("@@ -1,2 +1,1 @@\n-secret = 1\n keep\n+++ header\n+ok");
        assert_eq!(added, ["++ header", "ok"]);
        let refs: Vec<&str> = added.iter().map(String::as_str).collect();
        assert!(eval(&[file("a", &refs)]).is_empty());
    }

    #[test]
    fn deps_and_capabilities() {
        let m = evaluate(
            &Rules::defaults(),
            &[],
            &["serde".into()],
            &["prod-write".into()],
        );
        assert_eq!(ids(&m), ["R-deps", "R-prod-write"]);
        assert_eq!(m[0].evidence, ["new dependency serde"]);
        assert_eq!(level(&m), "high");
        assert_eq!(level(&[]), "normal");
    }

    #[test]
    fn bad_rules_name_the_rule() {
        let e = Rules::parse("[[rule]]\nid = \"R-x\"\nwhy = \"w\"\ncontent = \"(\"").unwrap_err();
        assert!(e.contains("R-x") && e.contains("regex"), "{e}");
        let e = Rules::parse("[[rule]]\nid = \"R-y\"\nwhy = \"w\"").unwrap_err();
        assert!(e.contains("R-y"), "{e}");
        assert!(Rules::parse("[[rule]]\nid=\"a\"\nwhy=\"w\"\nnope=1").is_err());
    }

    #[test]
    fn cargo_dependency_tables() {
        let text = r#"
[package]
name = "x"
[dependencies]
serde = "1"
[dev-dependencies]
tempfile = "3"
[build-dependencies]
cc = "1"
[workspace.dependencies]
toml = "1"
[target.'cfg(unix)'.dependencies]
libc = "0"
"#;
        let n = dependency_specs("crates/a/Cargo.toml", text);
        let got: Vec<&str> = n.keys().map(String::as_str).collect();
        assert_eq!(got, ["cc", "libc", "serde", "tempfile", "toml"]);
        assert!(dependency_specs("README.md", text).is_empty());
    }

    #[test]
    fn new_dependencies_are_head_minus_base() {
        let base = r#"{"dependencies":{"a":"1"}}"#;
        let head = r#"{"dependencies":{"a":"2","b":"1"},"devDependencies":{"c":"1"}}"#;
        let m = vec![(
            "package.json".to_string(),
            Some(base.into()),
            Some(head.into()),
        )];
        assert_eq!(new_dependencies(&m), ["b", "c"]);
        let created = vec![(
            "Cargo.toml".to_string(),
            None,
            Some("[dependencies]\nz=\"1\"".into()),
        )];
        assert_eq!(new_dependencies(&created), ["z"]);
    }

    fn deps(path: &str, base: Option<&str>, head: Option<&str>) -> Vec<String> {
        new_dependencies(&[(path.into(), base.map(Into::into), head.map(Into::into))])
    }

    #[test]
    fn workspace_edges_to_existing_pins_are_not_new() {
        let base = "[package]\nname = \"a\"\n[dependencies]\nserde = \"1\"\n";
        let head = "[package]\nname = \"a\"\n[dependencies]\nserde = \"1\"\n\
tokio.workspace = true\nanyhow = { workspace = true, features = [\"std\"] }\n\
core = { path = \"../core\" }\n[dev-dependencies]\ninsta = { workspace = true }\n";
        assert!(deps("crates/a/Cargo.toml", Some(base), Some(head)).is_empty());
        // A new crate whose manifest only inherits pins adds nothing either.
        let only_edges = "[package]\nname = \"b\"\n[dependencies]\ntokio.workspace = true\n";
        assert!(deps("crates/b/Cargo.toml", None, Some(only_edges)).is_empty());
    }

    #[test]
    fn new_root_pins_count_but_not_version_bumps() {
        let base = "[workspace.dependencies]\nserde = \"1.0.1\"\ntokio = { version = \"1\" }\n";
        let head = "[workspace.dependencies]\nserde = \"1.0.2\"\n\
tokio = { version = \"1\", features = [\"rt\"] }\nreqwest = \"0.12\"\n";
        assert_eq!(deps("Cargo.toml", Some(base), Some(head)), ["reqwest"]);
        // A path edge that becomes a registry dependency is new code.
        let b = "[dependencies]\nx = { path = \"../x\" }\n";
        let h = "[dependencies]\nx = { path = \"../x\", git = \"https://e.test/x\" }\n";
        assert_eq!(deps("a/Cargo.toml", Some(b), Some(h)), ["x"]);
    }

    #[test]
    fn pnpm_catalog_and_workspace_refs_are_not_new() {
        let base = r#"{"dependencies":{"a":"1"}}"#;
        let head =
            r#"{"dependencies":{"a":"1","b":"catalog:","c":"catalog:react18","d":"workspace:*"}}"#;
        assert!(deps("web/package.json", Some(base), Some(head)).is_empty());
        let head = r#"{"dependencies":{"a":"2","e":"^1"}}"#;
        assert_eq!(deps("package.json", Some(base), Some(head)), ["e"]);
    }

    #[test]
    fn pnpm_workspace_catalogs_count() {
        let base = "packages:\n  - 'apps/*'\ncatalog:\n  react: ^18.2.0\n";
        let head = "packages:\n  - 'apps/*'\ncatalog:\n  react: ^18.3.0\n  'zod': 3.23.0\n\
catalogs:\n  legacy:\n    \"@scope/x\": 1.0.0 # pinned\n";
        assert_eq!(
            deps("pnpm-workspace.yaml", Some(base), Some(head)),
            ["@scope/x", "zod"]
        );
        assert!(deps("pnpm-workspace.yaml", Some(head), Some(head)).is_empty());
    }

    #[test]
    fn owners_parse() {
        let o =
            parse_owners("[[owner]]\ngithub = \"ada\"\nslack = \"U1\"\n[[owner]]\ngithub = \"bo\"")
                .unwrap();
        assert_eq!(o.len(), 2);
        assert_eq!(o[0].slack.as_deref(), Some("U1"));
        assert!(parse_owners("[[owner]]\nname = \"x\"").is_err());
        let team = parse_owners("[[owner]]\nteam = \"acme/core\"").unwrap();
        assert_eq!(team[0].team.as_deref(), Some("acme/core"));
        for bad in [
            "[[owner]]\ngithub = \"a\"\nteam = \"acme/core\"",
            "[[owner]]\nslack = \"U1\"",
            "[[owner]]\nteam = \"acme\"",
            "[[owner]]\nteam = \"acme/a/b\"",
            "[[owner]]\nteam = \"acme/core\"\nslack = \"U1\"",
        ] {
            assert!(parse_owners(bad).is_err(), "{bad}");
        }
        let other = "[[owner]]\ngithub = \"a\"\n[[owner]]\nteam = \"else/core\"";
        assert!(parse_owners_in(other, "acme").is_err());
        assert!(parse_owners_in(&other.replace("else", "ACME"), "acme").is_ok());
    }

    #[test]
    fn note_names_are_plain_ascii_and_quoted() {
        // Bidi overrides, line separators and other non-ASCII cannot reorder or break the note.
        assert_eq!(plain("a\u{202e}b\u{2028}c\u{200b}`<>é"), "a?b?c?????");
        let note = risk_note(
            &eval(&[file(".github/workflows/x\u{202e}.yml", &[])]),
            "abc",
        );
        assert!(
            note.contains("  - `path .github/workflows/x?.yml`\n"),
            "{note}"
        );
    }

    #[test]
    fn note_is_replaced_in_place() {
        let m = eval(&[file(".github/workflows/x.yml", &[])]);
        let note = risk_note(&m, "abc");
        assert!(note.contains("R-ci") && note.contains("review the head SHA abc and approve"));
        let once = replace_note("Closes #1", Some(&note));
        let twice = replace_note(&once, Some(&risk_note(&m, "def")));
        assert_eq!(twice.matches(NOTE_OPEN).count(), 1);
        assert!(twice.starts_with("Closes #1") && twice.contains("SHA def"));
        assert_eq!(replace_note(&twice, None), "Closes #1");
    }

    /// Rules with both classes; the repo's own rules do not use two verdicts yet.
    const CLASSES: &str = r#"
[[rule]]
id = "R-gate"
why = "g"
paths = ["src/gates/**", ".github/workflows/ci.yml"]

[[rule]]
id = "R-ci-review"
why = "r"
approval = "two-verdicts"
paths = ["channel/**", ".github/workflows/**", "deny.toml"]
"#;

    fn classed(files: &[&str]) -> Vec<Match> {
        let files: Vec<ChangedFile> = files.iter().map(|f| file(f, &[])).collect();
        evaluate(&Rules::parse(CLASSES).unwrap(), &files, &[], &[])
    }

    fn approvals(files: &[&str]) -> Approval {
        requirement(&classed(files))
    }

    #[test]
    fn owner_gated_and_two_verdict_paths() {
        for p in ["src/gates/runner.rs", ".github/workflows/ci.yml"] {
            assert_eq!(approvals(&[p]), Approval::Owner, "{p}");
        }
        for p in ["channel/x.toml", "deny.toml", ".github/workflows/other.yml"] {
            assert_eq!(approvals(&[p]), Approval::TwoVerdicts, "{p}");
        }
        // A mix needs the owner: any owner match wins.
        assert_eq!(approvals(&["deny.toml", "src/gates/x.rs"]), Approval::Owner);
        // A file matching both classes needs the owner.
        assert_eq!(approvals(&[".github/workflows/ci.yml"]), Approval::Owner);
    }

    #[test]
    fn requirement_owner_wins_over_other_matches_and_unseen() {
        let review = classed(&["deny.toml"]);
        assert_eq!(requirement(&review), Approval::TwoVerdicts);
        let mut mixed = review.clone();
        mixed.extend(unseen_diff(&[], 5));
        assert_eq!(requirement(&mixed), Approval::Owner);
        let deps = evaluate(&Rules::defaults(), &[], &["x".into()], &[]);
        assert_eq!(requirement(&deps), Approval::Owner);
    }

    #[test]
    fn approval_field_parses_and_rejects_unknown_values() {
        let ok = |a: &str| {
            Rules::parse(&format!(
                "[[rule]]\nid=\"R-a\"\nwhy=\"w\"\npaths=[\"a\"]\n{a}"
            ))
        };
        assert_eq!(ok("").unwrap().rules[0].approval, Approval::Owner);
        assert_eq!(
            ok("approval = \"owner\"").unwrap().rules[0].approval,
            Approval::Owner
        );
        assert_eq!(
            ok("approval = \"two-verdicts\"").unwrap().rules[0].approval,
            Approval::TwoVerdicts
        );
        let e = ok("approval = \"anyone\"").unwrap_err();
        assert!(e.contains("R-a") && e.contains("anyone"), "{e}");
    }

    #[test]
    fn the_note_names_what_each_class_needs() {
        let owner = risk_note(&classed(&["src/gates/a.rs"]), "abc");
        assert!(owner.contains("Owner: review the head SHA abc and approve"));
        let two = risk_note(&classed(&["deny.toml"]), "abc");
        assert!(two.contains("two independent review verdicts"), "{two}");
        assert!(two.contains("head SHA abc") && two.contains("owner approval"));
    }

    fn rec_line(session: &str, head: &str, approved: bool, conf: Option<f64>) -> String {
        Record::Verdict {
            session: session.into(),
            head_sha: head.into(),
            approved,
            confidence: conf,
            major: None,
            model: None,
        }
        .to_line()
    }

    /// A comment as `bosun verdict apply` writes it.
    fn rec(session: &str, head: &str, approved: bool, conf: Option<f64>) -> String {
        format!(
            "Review verdict: approve 0.95.\n\n{}",
            rec_line(session, head, approved, conf)
        )
    }

    fn count(items: &[(&str, String)], head: &str) -> Vec<String> {
        two_verdicts(items.iter().map(|(a, b)| (*a, b.as_str())), head, &[], None)
    }

    #[test]
    fn two_verdicts_counts_distinct_bot_sessions_on_the_head() {
        let bot = VERDICT_AUTHOR;
        let ok = |s: &str| (bot, rec(s, "h", true, Some(0.9)));
        assert_eq!(count(&[ok("b"), ok("a")], "h"), ["a", "b"]);
        assert_eq!(count(&[ok("a"), ok("a")], "h"), ["a"]);
        assert!(count(&[ok("a")], "other").is_empty());
        // Below the bar, without a confidence, or not an approval.
        for r in [
            rec("c", "h", true, Some(0.89)),
            rec("c", "h", true, None),
            rec("c", "h", false, Some(0.99)),
        ] {
            assert!(count(&[(bot, r)], "h").is_empty());
        }
        // Not written by the verdict workflow.
        assert!(count(&[("some-agent", rec("c", "h", true, Some(1.0)))], "h").is_empty());
    }

    #[test]
    fn a_bot_session_never_counts_toward_an_approval() {
        let clean = |session: &str| {
            let r = Record::Verdict {
                session: session.into(),
                head_sha: "h".into(),
                approved: true,
                confidence: Some(0.99),
                major: Some(0),
                model: None,
            };
            format!("Review verdict: approve 0.99.\n\n{}", r.to_line())
        };
        let items = [clean("bot:claude[bot]"), clean("a")];
        let it = || items.iter().map(|b| (VERDICT_AUTHOR, b.as_str()));
        assert_eq!(two_verdicts(it(), "h", &[], None), ["a"]);
        assert_eq!(clean_verdicts(it(), "h", &[], None), ["a"]);
        let it3 = || items.iter().map(|b| (VERDICT_AUTHOR, b.as_str(), true));
        assert_eq!(autonomous_verdicts(it3(), "h", &[], None), ["a"]);
        let only_bot = [clean("bot:claude[bot]")];
        let ob = only_bot.iter().map(|b| (VERDICT_AUTHOR, b.as_str(), true));
        assert!(autonomous_verdicts(ob, "h", &[], None).is_empty());
        assert!(is_bot_session("bot:x") && !is_bot_session("session_bot"));
    }

    #[test]
    fn review_models_limit_which_verdicts_count() {
        let line = |s: &str, model: Option<&str>| {
            let r = Record::Verdict {
                session: s.into(),
                head_sha: "h".into(),
                approved: true,
                confidence: Some(0.95),
                major: Some(0),
                model: model.map(String::from),
            };
            format!("Review verdict: approve.\n\n{}", r.to_line())
        };
        let all = [
            (VERDICT_AUTHOR, line("a", Some("m1"))),
            (VERDICT_AUTHOR, line("b", Some("m2"))),
            (VERDICT_AUTHOR, line("c", None)),
        ];
        let it = || all.iter().map(|(a, b)| (*a, b.as_str()));
        assert_eq!(two_verdicts(it(), "h", &[], None), ["a", "b", "c"]);
        let m1 = ["m1".to_string()];
        assert_eq!(two_verdicts(it(), "h", &[], Some(&m1)), ["a"]);
        assert_eq!(clean_verdicts(it(), "h", &[], Some(&m1)), ["a"]);
        assert!(two_verdicts(it(), "h", &[], Some(&[])).is_empty());
        // Exact match only.
        let prefix = ["m".to_string()];
        assert!(clean_verdicts(it(), "h", &[], Some(&prefix)).is_empty());
        let auto = || all.iter().map(|(a, b)| (*a, b.as_str(), true));
        assert_eq!(autonomous_verdicts(auto(), "h", &[], Some(&m1)), ["a"]);
    }

    #[test]
    fn unknown_empty_and_excluded_sessions_never_count() {
        let bot = VERDICT_AUTHOR;
        let all = [
            (bot, rec("unknown", "h", true, Some(1.0))),
            (bot, rec("", "h", true, Some(1.0))),
            (bot, rec("worker", "h", true, Some(1.0))),
            (bot, rec("rev", "h", true, Some(1.0))),
        ];
        let got = two_verdicts(
            all.iter().map(|(a, b)| (*a, b.as_str())),
            "h",
            &["worker".to_string()],
            None,
        );
        assert_eq!(got, ["rev"]);
    }

    #[test]
    fn lease_sessions_lists_past_and_live_leases() {
        let lease = r#"{"bosun":"lease","session":"a","origin":"local","expires":"2026-10-22T10:30:00Z","touch":["x"]}"#;
        let end = r#"{"bosun":"lease-end","session":"b","reason":"released"}"#;
        let got = lease_sessions([
            lease,
            end,
            "plain text",
            r#"{"bosun":"leave","session":"c","reason":"x"}"#,
        ]);
        assert_eq!(got, ["a", "b"]);
    }

    #[test]
    fn only_the_shape_apply_writes_counts() {
        let line = rec_line("s", "h", true, Some(0.95));
        let good = format!("Review verdict: approve 0.95.\n\n{line}");
        assert!(apply_record(&good).is_some());
        assert!(apply_record(&format!("{good}\n")).is_some());
        for bad in [
            line.clone(),
            format!("hello\n\n{line}"),
            format!("Review verdict: x.\n\n{line}\n\ntrailing text"),
            format!("Review verdict: x.\n{line}\n{line}"),
            format!(
                "Review verdict: x.\n{}\n\n{line}",
                rec_line("o", "h", true, Some(1.0))
            ),
            "Review verdict: x.".to_string(),
            format!(
                "Review verdict: x.\n\n{}",
                r#"{"bosun":"leave","session":"s","reason":"x"}"#
            ),
        ] {
            assert!(apply_record(&bad).is_none(), "{bad}");
        }
    }

    #[test]
    fn the_newest_record_of_a_session_decides() {
        let bot = VERDICT_AUTHOR;
        let withdrawn = [
            (bot, rec("a", "h", true, Some(0.95))),
            (bot, rec("a", "h", false, Some(0.95))),
        ];
        assert!(count(&withdrawn, "h").is_empty());
        let restored = [
            (bot, rec("a", "h", false, Some(0.95))),
            (bot, rec("a", "h", true, Some(0.95))),
        ];
        assert_eq!(count(&restored, "h"), ["a"]);
        // A record on another head does not touch the head's verdict.
        let other = [
            (bot, rec("a", "h", true, Some(0.95))),
            (bot, rec("a", "old", false, Some(0.95))),
        ];
        assert_eq!(count(&other, "h"), ["a"]);
    }

    #[test]
    fn capability_labels_and_unseen_diffs() {
        let f = |path: &str, status: &str, patch: bool, changes: u64| FileView {
            path: path.into(),
            status: status.into(),
            has_patch: patch,
            changes,
            same_blob: status == "renamed" && path.starts_with("moved"),
        };
        let ok = [
            f("a.rs", "modified", true, 3),
            f("moved.png", "renamed", false, 0),
        ];
        assert!(unseen_diff(&ok, 2).is_none());
        assert!(unseen_diff(&ok, 0).is_none());
        assert!(unseen_diff(&ok, 3).is_some());
        let m = unseen_diff(&[f("big.rs", "modified", false, 900)], 1).unwrap();
        assert_eq!(m.rule, "R-unseen");
        assert!(m.evidence[0].contains("big.rs"));
        // GitHub reports 0 added and 0 deleted lines and no patch for a binary file: unseen.
        assert!(unseen_diff(&[f("img.PNG", "added", false, 0)], 1).is_some());
        assert!(unseen_diff(&[f("gone.rs", "removed", false, 5)], 1).is_none());
        // The extension is the author's choice: a file with changed lines and no patch is unseen.
        let m = unseen_diff(&[f("payload.bin", "added", false, 5000)], 1).unwrap();
        assert!(m.evidence[0].contains("payload.bin"));
        assert!(unseen_diff(&[f("lib.so", "modified", false, 12)], 1).is_some());
        assert!(unseen_diff(&[f("copy.rs", "copied", false, 40)], 1).is_some());
        // A renamed binary that also changed has no patch and 0 changes, like a pure rename.
        assert!(unseen_diff(&[f("edited.png", "renamed", false, 0)], 1).is_some());
        assert_eq!(
            capability_labels(&["bug", "bosun:capability/prod-write"]),
            ["prod-write"]
        );
    }

    #[test]
    fn a_comment_approves_only_the_full_head_sha() {
        let head = "0123456789abcdef0123456789abcdef01234567";
        assert!(comment_approves(&format!(" /bosun approve {head}\n"), head));
        assert!(!comment_approves(
            &format!("/bosun approve {}", &head[..7]),
            head
        ));
        assert!(!comment_approves("/bosun approve 0123456", "0123456"));
        assert!(!comment_approves(
            &format!("/bosun approve {head} ok"),
            head
        ));
        assert!(!comment_approves(&format!("/bosun  approve {head}"), head));
    }

    /// A delegate never approves a trust file change: the critical floor is what stops it.
    #[test]
    fn every_trust_file_is_critical() {
        for t in crate::core::approve::TRUST_FILES {
            let f = ChangedFile {
                path: t.into(),
                previous_path: None,
                added_lines: Vec::new(),
            };
            assert!(!critical(&[], &[f], false).is_empty(), "{t}");
        }
    }
}
