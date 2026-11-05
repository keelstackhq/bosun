//! `bosun gate <name> [--base REF] [--all]`. Works offline: it needs only git and the config, not
//! a GitHub repo id.

use std::path::{Path, PathBuf};
use std::process::Command;

use crate::cli::{GateArgs, stamps};
use crate::config::Config;
use crate::gates::ci_env::{self, BaseRule, CiContext, Provider};
use crate::gates::{self, Request};
use crate::output::{BosunError, CmdResult, Outcome};

/// `--base`, else `GITHUB_BASE_REF` as `origin/<ref>`, else `origin/main`.
pub fn resolve_base(flag: Option<&str>, github_base_ref: Option<&str>) -> String {
    let nonempty = |s: &&str| !s.trim().is_empty();
    match (flag.filter(nonempty), github_base_ref.filter(nonempty)) {
        (Some(b), _) => b.to_string(),
        (None, Some(r)) => format!("origin/{}", r.trim()),
        (None, None) => "origin/main".to_string(),
    }
}

/// Like `resolve_base`, but without a flag or `GITHUB_BASE_REF` the base is what
/// `origin/HEAD` points to (offline), else `origin/main`.
pub fn resolve_base_in(root: &Path, flag: Option<&str>, github_base_ref: Option<&str>) -> String {
    let nonempty = |s: &&str| !s.trim().is_empty();
    if flag.filter(nonempty).is_some() || github_base_ref.filter(nonempty).is_some() {
        return resolve_base(flag, github_base_ref);
    }
    Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["symbolic-ref", "--short", "refs/remotes/origin/HEAD"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .filter(|b| b.starts_with("origin/") && b.len() > "origin/".len())
        .unwrap_or_else(|| resolve_base(None, None))
}

/// Base and CI context: `--base` wins; on CircleCI `ci_env` decides; elsewhere `resolve_base_in`.
/// A merge-queue name is trusted only for the default branch and a sha on it, else it is a PR.
pub(crate) fn resolve_ci(
    root: &Path,
    flag: Option<&str>,
) -> Result<(String, CiContext), BosunError> {
    let env = |k: &str| std::env::var(k).ok();
    let default_ref = resolve_base_in(root, None, None);
    let default_branch = default_ref.strip_prefix("origin/").unwrap_or("main");
    let mut ci = ci_env::detect(&env, default_branch);
    let explicit = flag.is_some_and(|f| !f.trim().is_empty());
    // A queue name only sets the event (and `all`): the sha never sets the diff base, because an
    // old ancestor would hide what main gained since. It must still name a sha on the default branch.
    let queue = match &ci.base {
        BaseRule::Queue { sha, base } if !explicit => Some((sha.clone(), base.clone())),
        _ => None,
    };
    if let Some((sha, base)) = queue {
        let on_default = gates::git(root, &["merge-base", "--is-ancestor", &sha, &default_ref]);
        if base != default_branch || on_default.is_err() {
            eprintln!("bosun: the merge-queue branch name is not trusted; treating it as a PR");
            (ci.event, ci.all) = (ci_env::Event::Pr, false);
        }
        ci.base = BaseRule::MergeBase(default_ref.clone());
    }
    let base = match (&ci.base, explicit) {
        (BaseRule::Ref(r), false) => r.clone(),
        (BaseRule::MergeBase(r), false) => gates::git(root, &["merge-base", "HEAD", r])?
            .trim()
            .to_string(),
        _ => resolve_base_in(root, flag, env("GITHUB_BASE_REF").as_deref()),
    };
    Ok((base, ci))
}

pub(crate) fn repo_root(dir: &Path) -> Result<PathBuf, BosunError> {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(["rev-parse", "--show-toplevel"])
        .output()
        .map_err(|e| {
            BosunError::error(
                format!("cannot run git: {e}"),
                "install git and make sure it is on PATH",
            )
        })?;
    if !out.status.success() {
        return Err(BosunError::error(
            format!("{} is not inside a git repository", dir.display()),
            "run bosun inside the repo, or pass -C <repo>",
        ));
    }
    Ok(PathBuf::from(String::from_utf8_lossy(&out.stdout).trim()))
}

pub fn run(dir: Option<&Path>, args: &GateArgs) -> CmdResult {
    // Built in, not in the profile: the same code as `bosun test-integrity`.
    if args.name == "test-integrity" {
        let out = crate::cli::test_integrity::check(dir, args.base.as_deref())?;
        return Ok(stamped(
            dir.unwrap_or(Path::new(".")),
            &args.name,
            args.base.is_some(),
            out,
        ));
    }
    // Built in: the CI-minute rules over .github/workflows (docs/ci-minutes.md).
    if args.name == "ci-minutes" {
        return crate::cli::ci_minutes::check(dir);
    }
    // Not a profile gate: it reads GitHub, not the checkout.
    if args.name == "main-open" {
        return gates::main_open::run(dir, args.pr);
    }
    let dir = dir.map(Path::to_path_buf).unwrap_or_else(|| ".".into());
    let root = repo_root(&dir)?;
    let config = Config::load(&root)?;
    let (base, ci) = resolve_ci(&root, args.base.as_deref())?;
    let mut out = gates::run(&Request {
        root: &root,
        stacks: &config.stack,
        gate: &args.name,
        base: &base,
        all: args.all || ci.all,
    })?;
    if ci.provider == Provider::CircleCi {
        let (p, e) = (ci.provider.name(), ci.event.name());
        out.data["ci"] = serde_json::json!({"provider": p, "event": e, "base": base});
    }
    Ok(stamped(&root, &args.name, args.base.is_some(), out))
}

/// Whether the gate did nothing: every run it made was a skip (empty diff, missing manifest).
fn all_skipped(data: &serde_json::Value) -> bool {
    let skipped = |v: &serde_json::Value| v["skipped"] == true;
    match data["runs"].as_array() {
        Some(runs) => runs.iter().all(skipped),
        None => skipped(data),
    }
}

/// Outside CI, leave the stamp `bosun pr --ready` looks for and report it as `data.stamp`.
/// Only a real run on the default base stamps: a skip proves nothing about the tree, and an
/// explicit `--base` checks a different range than the PR's (`data.stamp` is null then).
/// A failure to stamp goes to stderr and never fails the gate.
fn stamped(dir: &Path, gate: &str, explicit_base: bool, mut out: Outcome) -> Outcome {
    let env = |k: &str| std::env::var(k).ok();
    if ci_env::detect(&env, "main").provider != Provider::Local {
        return out;
    }
    out.data["stamp"] = serde_json::Value::Null;
    if explicit_base || all_skipped(&out.data) {
        return out;
    }
    match stamps::write(dir, gate) {
        Ok(path) => out.data["stamp"] = serde_json::json!(path.display().to_string()),
        Err(e) => eprintln!("bosun: gate {gate} passed but could not be stamped: {e}"),
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_gate_is_skipped_only_when_every_run_is() {
        use serde_json::json;
        assert!(all_skipped(&json!({"skipped": true})));
        assert!(!all_skipped(&json!({"exit_status": 0})));
        let mixed = json!({"skipped": true, "runs": [{"skipped": false}, {"skipped": true}]});
        assert!(!all_skipped(&mixed));
    }

    #[test]
    fn base_resolution_order() {
        assert_eq!(resolve_base(Some("v1"), Some("dev")), "v1");
        assert_eq!(resolve_base(None, Some("dev")), "origin/dev");
        assert_eq!(resolve_base(None, Some("")), "origin/main");
        assert_eq!(resolve_base(None, None), "origin/main");
    }
}
