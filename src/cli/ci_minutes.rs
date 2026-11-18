//! `bosun gate ci-minutes`: the CI-minute rules (`core::ci_minutes`) over the repo's
//! `.github/workflows/*.yml|yaml`. Built in, runs no repo code, needs no GitHub. `bosun doctor`
//! reports the same findings.

use std::path::Path;

use serde_json::{Value, json};

use crate::cli::gate::repo_root;
use crate::config::{CiProvider, Config};
use crate::core::ci_minutes::{self, Finding};
use crate::output::{BosunError, CmdResult, Outcome};

/// The workflow files read and every finding, sorted by file name.
pub fn scan(root: &Path) -> (Vec<String>, Vec<Finding>) {
    let Ok(dir) = std::fs::read_dir(root.join(".github/workflows")) else {
        return (Vec::new(), Vec::new());
    };
    let mut files: Vec<(String, String)> = dir
        .flatten()
        .filter(|e| {
            e.path()
                .extension()
                .is_some_and(|x| x == "yml" || x == "yaml")
        })
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            Some((
                format!(".github/workflows/{name}"),
                std::fs::read_to_string(e.path()).ok()?,
            ))
        })
        .collect();
    files.sort();
    let findings = files
        .iter()
        .flat_map(|(f, t)| ci_minutes::check(f, t))
        .collect();
    (files.into_iter().map(|(f, _)| f).collect(), findings)
}

pub fn findings_json(findings: &[Finding]) -> Vec<Value> {
    findings
        .iter()
        .map(|f| json!({"file": f.file, "job": f.job, "rule": f.rule, "fix": f.fix}))
        .collect()
}

/// The CI provider at the merge-base with the PR base (the gates' base, `resolve_base_in`:
/// `origin/$GITHUB_BASE_REF`, else what `origin/HEAD` points to, else `origin/main`), never the working tree: a PR that sets `none` cannot skip the gate on itself.
/// Any doubt (no base, no file, unparsable) is the default provider, so the gate runs.
pub fn base_provider(root: &Path) -> CiProvider {
    let base_ref = crate::cli::gate::resolve_base_in(
        root,
        None,
        std::env::var("GITHUB_BASE_REF").ok().as_deref(),
    );
    let Some(base) = crate::hooks::git_out(root, &["merge-base", "HEAD", &base_ref]) else {
        return CiProvider::default();
    };
    let spec = format!("{base}:.bosun/bosun.toml");
    crate::hooks::git_out(root, &["show", &spec])
        .and_then(|t| Config::parse(&t, ".bosun/bosun.toml").ok())
        .map_or_else(CiProvider::default, |c| c.ci_provider())
}

/// Whether the gate is skipped: the base config says no CI provider and the repo has no workflow
/// file. Workflow files that exist are always scanned.
pub fn skipped(root: &Path) -> bool {
    base_provider(root) == CiProvider::None && scan(root).0.is_empty()
}

pub fn check(dir: Option<&Path>) -> CmdResult {
    let dir = dir.map(Path::to_path_buf).unwrap_or_else(|| ".".into());
    let root = repo_root(&dir)?;
    // A config that does not parse is an error, whatever the provider.
    Config::load(&root)?;
    if skipped(&root) {
        return Ok(Outcome::data(
            json!({"gate": "ci-minutes", "skipped": true, "reason": "ci provider none", "files": [], "findings": []}),
        ));
    }
    let (files, findings) = scan(&root);
    let data = json!({"gate": "ci-minutes", "files": files, "findings": findings_json(&findings)});
    if findings.is_empty() {
        return Ok(Outcome::data(data));
    }
    let first = &findings[0];
    Err(BosunError::error(
        format!(
            "{} CI-minute finding(s); first: {} {}{}",
            findings.len(),
            first.file,
            first.rule,
            first
                .job
                .as_ref()
                .map_or(String::new(), |j| format!(" (job {j})"))
        ),
        format!(
            "{} (all findings are in data.findings; docs/ci-minutes.md)",
            first.fix
        ),
    )
    .with_data(data))
}
