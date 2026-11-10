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

pub fn check(dir: Option<&Path>) -> CmdResult {
    let dir = dir.map(Path::to_path_buf).unwrap_or_else(|| ".".into());
    let root = repo_root(&dir)?;
    if Config::load(&root)?.ci_provider() == CiProvider::None {
        return Ok(Outcome::data(
            json!({"gate": "ci-minutes", "skipped": "ci provider none", "files": [], "findings": []}),
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
