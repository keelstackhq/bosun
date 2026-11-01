//! `bosun test-integrity [--base REF] [--pr N]` and `bosun gate test-integrity`. Works offline,
//! except that `--pr` reads the PR body and the linked task's `allow` from GitHub.

use std::path::Path;

use serde_json::json;

use crate::app::App;
use crate::cli::IntegrityArgs;
use crate::cli::gate::{repo_root, resolve_ci};
use crate::config::Config;
use crate::core::meta::{ALLOW_KINDS, linked_issues, parse_task_meta};
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
    match args.pr {
        Some(n) => from_pr(dir, args.base.as_deref(), n),
        None => check(dir, args.base.as_deref()),
    }
}

/// The PR's body and the `allow` of its first linked issue with task metadata (known kinds only),
/// in place of `BOSUN_PR_BODY` and `BOSUN_INTEGRITY_ALLOW`. `data.task` and `data.allow` say
/// which were used; a PR that links no task allows nothing.
fn from_pr(dir: Option<&Path>, base: Option<&str>, n: u64) -> CmdResult {
    let app = App::load(dir)?;
    let pull = app.gh.get_pull(n)?.ok_or_else(|| {
        BosunError::error(
            format!("PR #{n} not found"),
            "pass the number of an open PR",
        )
    })?;
    let mut task = None;
    let mut allow = Vec::new();
    for i in linked_issues(&pull.body) {
        let meta = app
            .gh
            .get_issue(i)?
            .and_then(|x| parse_task_meta(x.body_text()));
        if let Some(m) = meta {
            task = Some(i);
            allow = m.allow;
            allow.retain(|k| ALLOW_KINDS.contains(&k.as_str()));
            break;
        }
    }
    let add = |data: &mut serde_json::Value| {
        data["task"] = json!(task);
        data["allow"] = json!(allow);
    };
    match check_with(dir, base, &pull.body, &allow) {
        Ok(mut out) => {
            add(&mut out.data);
            Ok(out)
        }
        Err(mut e) => {
            if let Some(data) = e.data.as_mut() {
                add(data);
            }
            Err(e)
        }
    }
}

/// Shared by the command and the gate.
pub fn check(dir: Option<&Path>, base: Option<&str>) -> CmdResult {
    let body = std::env::var("BOSUN_PR_BODY").unwrap_or_default();
    let allow = parse_allow(&std::env::var("BOSUN_INTEGRITY_ALLOW").unwrap_or_default());
    check_with(dir, base, &body, &allow)
}

fn check_with(dir: Option<&Path>, base: Option<&str>, body: &str, allow: &[String]) -> CmdResult {
    let dir = dir.map(Path::to_path_buf).unwrap_or_else(|| ".".into());
    let root = repo_root(&dir)?;
    let (base, _) = resolve_ci(&root, base)?;
    let config = Config::load(&root)?;
    integrity::run(&Request {
        root: &root,
        stacks: &config.stack,
        base: &base,
        body,
        allow,
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
