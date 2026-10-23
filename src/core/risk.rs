//! Risk rules and their evaluation (spec section 8). Pure: the command reads the rules from the
//! PR's base branch and passes the texts in. A rule only escalates; nothing de-escalates.

use std::collections::BTreeSet;

use regex::Regex;
use serde::{Deserialize, Serialize};

use crate::core::glob::{Glob, parse_all};

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
            rules.push(Rule {
                id,
                why: r.why,
                paths,
                content,
                new_dependency: r.new_dependency,
                capability: r.capability,
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

/// The added lines of a patch: `+` lines, not the `+++` header.
pub fn added_lines(patch: &str) -> Vec<String> {
    patch
        .lines()
        .filter(|l| l.starts_with('+') && !l.starts_with("+++"))
        .map(|l| l[1..].to_string())
        .collect()
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Match {
    pub rule: String,
    pub why: String,
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
                .any(|p| rule.paths.iter().any(|g| g.matches(p)));
            if hit {
                evidence.push(format!("path {}", f.path));
            }
            if let Some(re) = &rule.content {
                for (i, line) in f.added_lines.iter().enumerate() {
                    if re.is_match(line) {
                        evidence.push(format!("{}: added line {} matches", f.path, i + 1));
                    }
                }
            }
        }
        if rule.new_dependency {
            evidence.extend(new_deps.iter().map(|d| format!("new dependency {d}")));
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
                evidence,
            });
        }
    }
    out
}

/// A file as listed by GitHub, for [`unseen_diff`].
pub struct FileView {
    pub path: String,
    pub status: String,
    pub has_patch: bool,
    /// Added plus deleted lines; binary files and pure renames report 0.
    pub changes: u64,
}

const BINARY_EXTENSIONS: [&str; 16] = [
    "png", "jpg", "jpeg", "gif", "ico", "webp", "pdf", "zip", "gz", "tar", "woff", "woff2", "ttf",
    "wasm", "so", "bin",
];

fn obviously_binary(f: &FileView) -> bool {
    f.changes == 0
        || f.path
            .rsplit_once('.')
            .is_some_and(|(_, e)| BINARY_EXTENSIONS.contains(&e.to_ascii_lowercase().as_str()))
}

/// `R-unseen` when bosun could not see the whole diff: GitHub listed fewer files than the PR
/// changed (it lists at most 3000), or a text file it lists has no patch (large diffs).
/// Built in, not a rule file: a PR must not be able to hide behind a missing diff.
pub fn unseen_diff(files: &[FileView], changed_files: u64) -> Option<Match> {
    let mut evidence = Vec::new();
    if (files.len() as u64) < changed_files {
        evidence.push(format!(
            "GitHub listed {} of {changed_files} changed files",
            files.len()
        ));
    }
    for f in files {
        if matches!(
            f.status.as_str(),
            "added" | "modified" | "renamed" | "changed"
        ) && !f.has_patch
            && !obviously_binary(f)
        {
            evidence.push(format!("no patch for {}", f.path));
        }
    }
    (!evidence.is_empty()).then(|| Match {
        rule: "R-unseen".into(),
        why: "bosun could not see the whole diff".into(),
        evidence,
    })
}

/// `high` when any rule matched, else `normal`.
pub fn level(matches: &[Match]) -> &'static str {
    if matches.is_empty() { "normal" } else { "high" }
}

const CARGO_TABLES: [&str; 3] = ["dependencies", "dev-dependencies", "build-dependencies"];
const NPM_TABLES: [&str; 4] = [
    "dependencies",
    "devDependencies",
    "optionalDependencies",
    "peerDependencies",
];

/// Dependency names declared in a `Cargo.toml` or `package.json` text (by the file name of
/// `path`). Other files and unparsable texts have none.
pub fn dependency_names(path: &str, text: &str) -> BTreeSet<String> {
    let mut names = BTreeSet::new();
    match path.rsplit('/').next().unwrap_or(path) {
        "Cargo.toml" => {
            let Ok(doc) = toml::from_str::<toml::Table>(text) else {
                return names;
            };
            let mut tables: Vec<&toml::Table> = Vec::new();
            let mut scopes = vec![&doc];
            if let Some(t) = doc.get("target").and_then(|t| t.as_table()) {
                scopes.extend(t.values().filter_map(|v| v.as_table()));
            }
            for scope in scopes {
                for key in CARGO_TABLES {
                    tables.extend(scope.get(key).and_then(|v| v.as_table()));
                }
            }
            tables.extend(
                doc.get("workspace")
                    .and_then(|w| w.get("dependencies"))
                    .and_then(|v| v.as_table()),
            );
            for t in tables {
                names.extend(t.keys().cloned());
            }
        }
        "package.json" => {
            let Ok(doc) = serde_json::from_str::<serde_json::Value>(text) else {
                return names;
            };
            for key in NPM_TABLES {
                if let Some(o) = doc.get(key).and_then(|v| v.as_object()) {
                    names.extend(o.keys().cloned());
                }
            }
        }
        _ => {}
    }
    names
}

/// Names declared at head but not at base, over `(path, base text, head text)` triples.
/// A manifest missing at base is all new.
pub fn new_dependencies(manifests: &[(String, Option<String>, Option<String>)]) -> Vec<String> {
    let mut out = BTreeSet::new();
    for (path, base, head) in manifests {
        let old = base
            .as_deref()
            .map_or_else(BTreeSet::new, |t| dependency_names(path, t));
        let new = head
            .as_deref()
            .map_or_else(BTreeSet::new, |t| dependency_names(path, t));
        out.extend(new.difference(&old).cloned());
    }
    out.into_iter().collect()
}

/// A repo owner from `governance/owners.toml`.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Owner {
    pub github: String,
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
    toml::from_str::<RawOwners>(text)
        .map(|o| o.owner)
        .map_err(|e| format!("governance/owners.toml: {e}"))
}

const NOTE_OPEN: &str = "<!-- bosun:risk -->";
const NOTE_CLOSE: &str = "<!-- /bosun:risk -->";

/// The Risk note: the rules that matched, why, the evidence, and what the owner must check.
pub fn risk_note(matches: &[Match], head_sha: &str) -> String {
    let mut s = format!("{NOTE_OPEN}\n## Risk: high\n\n");
    for m in matches {
        s.push_str(&format!("- **{}**: {}\n", m.rule, m.why));
        for e in &m.evidence {
            s.push_str(&format!("  - {e}\n"));
        }
    }
    s.push_str(&format!(
        "\nOwner: review the head SHA {head_sha} and approve. An approval on an older commit does not count.\n{NOTE_CLOSE}"
    ));
    s
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

#[cfg(test)]
mod tests {
    use super::*;

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
    fn defaults_have_the_five_spec_rules() {
        let r = Rules::defaults();
        let got: Vec<&str> = r.rules.iter().map(|r| r.id.as_str()).collect();
        assert_eq!(
            got,
            ["R-gov", "R-ci", "R-secrets", "R-deps", "R-prod-write"]
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
        assert!(eval(&[file("src/governance.rs", &[])]).is_empty());
    }

    #[test]
    fn content_rule_reads_added_lines_only() {
        let m = eval(&[file("a.env", &["API_KEY = abc"])]);
        assert_eq!(ids(&m), ["R-secrets"]);
        assert!(!m[0].evidence[0].contains("abc"));
        assert!(eval(&[file("a.env", &["let tokens = 3;"])]).is_empty());
        let added = added_lines("@@ -1,2 +1,1 @@\n-secret = 1\n keep\n+++ header\n+ok");
        assert_eq!(added, ["ok"]);
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
        let n = dependency_names("crates/a/Cargo.toml", text);
        let got: Vec<&str> = n.iter().map(String::as_str).collect();
        assert_eq!(got, ["cc", "libc", "serde", "tempfile", "toml"]);
        assert!(dependency_names("README.md", text).is_empty());
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

    #[test]
    fn owners_parse() {
        let o =
            parse_owners("[[owner]]\ngithub = \"ada\"\nslack = \"U1\"\n[[owner]]\ngithub = \"bo\"")
                .unwrap();
        assert_eq!(o.len(), 2);
        assert_eq!(o[0].slack.as_deref(), Some("U1"));
        assert!(parse_owners("[[owner]]\nname = \"x\"").is_err());
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

    #[test]
    fn capability_labels_and_unseen_diffs() {
        let f = |path: &str, status: &str, patch: bool, changes: u64| FileView {
            path: path.into(),
            status: status.into(),
            has_patch: patch,
            changes,
        };
        let ok = [
            f("a.rs", "modified", true, 3),
            f("logo.png", "added", false, 0),
        ];
        assert!(unseen_diff(&ok, 2).is_none());
        assert!(unseen_diff(&ok, 0).is_none());
        assert!(unseen_diff(&ok, 3).is_some());
        let m = unseen_diff(&[f("big.rs", "modified", false, 900)], 1).unwrap();
        assert_eq!(m.rule, "R-unseen");
        assert!(m.evidence[0].contains("big.rs"));
        assert!(unseen_diff(&[f("img.PNG", "added", false, 5)], 1).is_none());
        assert!(unseen_diff(&[f("gone.rs", "removed", false, 5)], 1).is_none());
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
}
