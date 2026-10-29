//! Risk rules and their evaluation (spec section 8). Pure: the command reads the rules from the
//! PR's base branch and passes the texts in. A rule only escalates; nothing de-escalates.

use std::collections::BTreeSet;

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
                approval: rule.approval,
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

/// `R-unseen` when bosun could not see the whole diff: GitHub listed fewer files than the PR
/// changed (it lists at most 3000), or a file with changed lines has no patch (large diffs).
/// GitHub reports 0 changed lines for a binary file; the file name is the author's choice, so
/// it never exempts a file. Built in, not a rule file: a PR must not hide behind a missing diff.
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
        if !matches!(f.status.as_str(), "removed" | "unchanged") && !f.has_patch && f.changes > 0 {
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

/// Rules that are critical whatever their `approval` says: a delegate never approves them.
pub const CRITICAL_RULES: [&str; 3] = ["R-secrets", "R-prod-write", "R-unseen"];

/// Paths of the approval and risk system in every repo: a delegate never approves a change to
/// them (also when renamed from or to them). Every workflow is here: any workflow with
/// `statuses: write` can post a `risk-check` or `review-verdict` status for any PR; so can a
/// CircleCI config, which is why `.circleci/**` is here too. This only keeps a delegate from
/// approving such a change: a same-repo PR's own `on: pull_request` copy can post a forged
/// status on its head before anyone approves, so it is no owner gate (spec §8 residual; B2, a
/// check run from a dedicated GitHub App pinned in the ruleset, closes it).
pub const CRITICAL_PATHS: [&str; 17] = [
    ".bosun/**",
    "governance/**",
    "CODEOWNERS",
    ".github/CODEOWNERS",
    "docs/CODEOWNERS",
    // The required checks `hoist apply` puts in the ruleset: only the root file has `[github]`.
    "hoist.toml",
    ".github/workflows/**",
    ".github/actions/**",
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

/// What a bootstrap-mode approval (D-012) never covers: the setting, the code that checks it and
/// verifies signatures, governance, the risk rules, the required checks and the jobs with an
/// environment. Apart from [`CRITICAL_PATHS`], so the window is never widened or reopened from
/// inside it.
pub const BOOTSTRAP_EXCLUDED: [&str; 22] = [
    // `.bosun/version` pins the gate release that runs the check.
    ".bosun/**",
    "governance/**",
    "CODEOWNERS",
    ".github/CODEOWNERS",
    "docs/CODEOWNERS",
    "hoist.toml",
    "src/config.rs",
    "src/time.rs",
    "src/cli/risk_check.rs",
    "src/cli/approve.rs",
    "src/core/approve.rs",
    "src/core/risk.rs",
    "src/core/risk_default.toml",
    "src/core/glob.rs",
    "src/github/b64.rs",
    "Cargo.toml",
    "Cargo.lock",
    "channel/pieces/core/files/.bosun/**",
    ".github/workflows/release.yml",
    "**/workflows/splice.yml",
    // They run the approval check (root and channel copies).
    "**/workflows/risk.yml",
    "**/workflows/verdict.yml",
];

/// Why a bootstrap-mode approval cannot count: the excluded paths among `(path, previous path,
/// patch)` (a rename counts on both sides), a workflow whose diff names `environment:` or is not
/// shown, and an R-secrets match. Empty: it can count.
pub fn bootstrap_excluded(
    files: &[(&str, Option<&str>, Option<&str>)],
    matches: &[Match],
) -> Vec<String> {
    let globs: Vec<Glob> = (BOOTSTRAP_EXCLUDED.iter())
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
    g.matches(path) || g.matches(&path.to_lowercase())
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
/// `excluded` (the task's lease holders), never count.
pub fn two_verdicts<'a>(
    comments: impl IntoIterator<Item = (&'a str, &'a str)>,
    head: &str,
    excluded: &[String],
) -> Vec<String> {
    verdict_sessions(comments, head, excluded, |_| true)
}

/// The sessions whose verdict on `head` lets a delegate approve: as [`two_verdicts`], and the
/// record counts its major findings and has none (a blocker never approves). Older records
/// without the count never count.
pub fn clean_verdicts<'a>(
    comments: impl IntoIterator<Item = (&'a str, &'a str)>,
    head: &str,
    excluded: &[String],
) -> Vec<String> {
    verdict_sessions(comments, head, excluded, |major| major == Some(0))
}

fn verdict_sessions<'a>(
    comments: impl IntoIterator<Item = (&'a str, &'a str)>,
    head: &str,
    excluded: &[String],
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
        }) = apply_record(body)
            && head_sha == head
            && !session.is_empty()
            && session != "unknown"
            && !excluded.contains(&session)
        {
            let counts =
                approved && confidence.is_some_and(|c| c >= VERDICT_MIN_CONFIDENCE) && clean(major);
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
                "R-prod-write"
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
        two_verdicts(items.iter().map(|(a, b)| (*a, b.as_str())), head, &[])
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
        // GitHub reports 0 added and 0 deleted lines for a binary file.
        assert!(unseen_diff(&[f("img.PNG", "added", false, 0)], 1).is_none());
        assert!(unseen_diff(&[f("gone.rs", "removed", false, 5)], 1).is_none());
        // The extension is the author's choice: a file with changed lines and no patch is unseen.
        let m = unseen_diff(&[f("payload.bin", "added", false, 5000)], 1).unwrap();
        assert!(m.evidence[0].contains("payload.bin"));
        assert!(unseen_diff(&[f("lib.so", "modified", false, 12)], 1).is_some());
        assert!(unseen_diff(&[f("copy.rs", "copied", false, 40)], 1).is_some());
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
