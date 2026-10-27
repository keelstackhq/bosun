//! `bosun test-integrity [--base REF]` and `bosun gate test-integrity`. Works offline.

use std::path::Path;

use crate::cli::IntegrityArgs;
use crate::cli::gate::{repo_root, resolve_ci};
use crate::config::Config;
use crate::gates::integrity::{self, Request};
use crate::output::{BosunError, CmdResult};

/// Split `BOSUN_INTEGRITY_ALLOW` into kinds.
fn parse_allow(text: &str) -> Vec<String> {
    text.split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect()
}

pub fn run(dir: Option<&Path>, args: &IntegrityArgs) -> CmdResult {
    if args.pr.is_some() {
        return Err(BosunError::usage(
            "--pr is not available yet",
            "set BOSUN_PR_BODY and BOSUN_INTEGRITY_ALLOW instead of --pr",
        ));
    }
    check(dir, args.base.as_deref())
}

/// Shared by the command and the gate.
pub fn check(dir: Option<&Path>, base: Option<&str>) -> CmdResult {
    let dir = dir.map(Path::to_path_buf).unwrap_or_else(|| ".".into());
    let root = repo_root(&dir)?;
    let (base, _) = resolve_ci(&root, base)?;
    // Task 13+ reads them from the PR and the task metadata.
    let body = std::env::var("BOSUN_PR_BODY").unwrap_or_default();
    let allow = parse_allow(&std::env::var("BOSUN_INTEGRITY_ALLOW").unwrap_or_default());
    let config = Config::load(&root)?;
    integrity::run(&Request {
        root: &root,
        stacks: &config.stack,
        base: &base,
        body: &body,
        allow: &allow,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allow_list_splits_on_commas() {
        assert_eq!(
            parse_allow(" skip, delete-test ,,"),
            vec!["skip", "delete-test"]
        );
        assert!(parse_allow("").is_empty());
    }
}
