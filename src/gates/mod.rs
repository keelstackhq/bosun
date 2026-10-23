//! The gate runner (spec §7): `bosun gate <name>` maps a gate to the stack profile's command, narrows
//! it to the affected packages, runs it with its output on stderr, and reports one JSON object.

pub mod adapters;
pub mod integrity;
pub mod packages;
pub mod profile;

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Instant;

use serde_json::{Value, json};

use crate::config::Stack;
use crate::output::{BosunError, CmdResult, Outcome};
use packages::Affected;
use profile::{Gate, Profile, WhenNoPackage};

pub struct Request<'a> {
    pub root: &'a Path,
    pub stacks: &'a [Stack],
    pub gate: &'a str,
    pub base: &'a str,
    pub all: bool,
}

/// Run `git <args>` in `root`; stdout on success.
pub(crate) fn git(root: &Path, args: &[&str]) -> Result<String, BosunError> {
    let out = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .output()
        .map_err(|e| {
            BosunError::error(
                format!("cannot run git: {e}"),
                "install git and make sure it is on PATH",
            )
        })?;
    if !out.status.success() {
        return Err(BosunError::error(
            format!(
                "git {} failed: {}",
                args.join(" "),
                String::from_utf8_lossy(&out.stderr).trim()
            ),
            "fetch the base branch (`git fetch origin main`) or pass an existing ref with --base",
        ));
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// Whether `git cat-file -e <spec>` finds the object; any failure counts as "no".
fn git_has(root: &Path, spec: &str) -> bool {
    Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["cat-file", "-e", spec])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

/// The profile's manifest when the stack has none in the working tree and none at the base
/// either, also with `--all`: a stack that never had one has nothing to check yet.
fn missing_manifest<'a>(req: &Request, stack: &Stack, profile: &'a Profile) -> Option<&'a str> {
    let manifest = profile.manifest.as_deref()?;
    let rel = stack_rel(&stack.path);
    if req.root.join(rel).join(manifest).exists() {
        return None;
    }
    // Also with --all: a merge-queue run after a PR deleted the manifest must still fail.
    let in_base = {
        let path = Path::new(rel).join(manifest);
        git_has(req.root, &format!("{}:{}", req.base, path.display()))
    };
    (!in_base).then_some(manifest)
}

/// A stack path as a clean relative path; `.` and empty mean the repo root.
fn stack_rel(path: &str) -> &str {
    let p = path.trim_start_matches("./").trim_end_matches('/');
    if p == "." { "" } else { p }
}

/// The changed files that live under the stack, relative to the stack directory.
fn under_stack(path: &str, changed: &[String]) -> Vec<String> {
    let rel = stack_rel(path);
    changed
        .iter()
        .filter_map(|f| {
            if rel.is_empty() {
                Some(f.clone())
            } else {
                Path::new(f)
                    .strip_prefix(rel)
                    .ok()
                    .map(|p| p.to_string_lossy().into_owned())
            }
        })
        .collect()
}

pub fn run(req: &Request) -> CmdResult {
    let range = format!("{}...HEAD", req.base);
    let changed: Vec<String> = git(req.root, &["diff", "--name-only", &range])?
        .lines()
        .map(String::from)
        .collect();

    let mut plan: Vec<(&Stack, Profile, Vec<String>)> = Vec::new();
    for stack in req.stacks {
        let files = under_stack(&stack.path, &changed);
        if req.all || !files.is_empty() {
            let profile = Profile::load(req.root, &stack.profile)?;
            profile.gate(req.gate)?;
            plan.push((stack, profile, files));
        }
    }
    if plan.is_empty() {
        return Ok(Outcome::data(json!({
            "gate": req.gate, "stack": null, "command": [], "packages": [],
            "exit_status": 0, "secs": 0.0, "over_budget": false, "skipped": true,
        })));
    }

    let mut runs: Vec<Value> = Vec::new();
    let mut failed: Option<i64> = None;
    for (stack, profile, files) in &plan {
        let gate = profile.gate(req.gate)?;
        let run = match missing_manifest(req, stack, profile) {
            Some(manifest) => json!({
                "gate": req.gate, "stack": stack.path, "command": [], "packages": [],
                "exit_status": 0, "secs": 0.0, "over_budget": false, "skipped": true,
                "reason": format!("no {manifest} in {}", stack.path),
            }),
            None => execute(req, stack, gate, files)?,
        };
        let status = run["exit_status"].as_i64().unwrap_or(0);
        runs.push(run);
        if status != 0 {
            failed = Some(status);
            break;
        }
    }
    let mut data = runs.last().cloned().unwrap_or_else(|| json!({}));
    if runs.len() > 1 {
        data["runs"] = Value::Array(runs);
    }
    match failed {
        None => Ok(Outcome::data(data)),
        Some(code) => Err(BosunError::error(
            format!("gate {} failed (exit {code})", req.gate),
            format!(
                "fix the failures above, then rerun `bosun gate {}`",
                req.gate
            ),
        )
        .with_data(data)),
    }
}

fn execute(
    req: &Request,
    stack: &Stack,
    gate: &Gate,
    files: &[String],
) -> Result<Value, BosunError> {
    let dir: PathBuf = req.root.join(stack_rel(&stack.path));
    let (names, extra): (Vec<String>, Vec<String>) = if gate.is_per_package() {
        let pkgs = packages::load(&dir)?;
        let affected = if req.all {
            Affected::All
        } else {
            packages::affected(&dir, files, &pkgs)
        };
        match affected {
            Affected::None if gate.when_no_package == WhenNoPackage::Skip => {
                return Ok(json!({
                    "gate": req.gate, "stack": stack.path, "command": [], "packages": [],
                    "exit_status": 0, "secs": 0.0, "over_budget": false, "skipped": true,
                }));
            }
            Affected::Some(names) => {
                let args = gate.package_args(&names);
                (names, args)
            }
            Affected::All | Affected::None => (
                pkgs.iter().map(|p| p.name.clone()).collect(),
                gate.all.clone(),
            ),
        }
    } else {
        (Vec::new(), Vec::new())
    };

    let diff = if gate.needs_diff() {
        let path = std::env::temp_dir().join(format!("bosun-gate-{}.diff", std::process::id()));
        let range = format!("{}...HEAD", req.base);
        let text = git(req.root, &["diff", &range])?;
        std::fs::write(&path, text).map_err(|e| {
            BosunError::error(
                format!("cannot write {}: {e}", path.display()),
                "make the temp directory writable (TMPDIR)",
            )
        })?;
        Some(path)
    } else {
        None
    };
    let diff_str = diff.as_ref().map(|d| d.to_string_lossy().into_owned());
    let mut argv = gate.expand_cmd(files, diff_str.as_deref());
    argv.extend(extra);

    let result = spawn(&argv, &dir);
    if let Some(d) = &diff {
        let _ = std::fs::remove_file(d);
    }
    let (status, secs) = result?;
    Ok(json!({
        "gate": req.gate,
        "stack": stack.path,
        "command": argv,
        "packages": names,
        "exit_status": status,
        "secs": secs,
        "over_budget": secs > gate.budget() as f64,
        "skipped": false,
    }))
}

/// Run `argv` in `dir` with stdout and stderr both on bosun's stderr; (exit status, seconds).
fn spawn(argv: &[String], dir: &Path) -> Result<(i64, f64), BosunError> {
    let program = argv.first().ok_or_else(|| {
        BosunError::error(
            "the gate command is empty",
            "set `cmd` in the profile gate to a program and its arguments",
        )
    })?;
    let start = Instant::now();
    let status = Command::new(program)
        .args(&argv[1..])
        .current_dir(dir)
        .stdin(Stdio::null())
        .stdout(Stdio::from(std::io::stderr()))
        .stderr(Stdio::inherit())
        .status()
        .map_err(|e| {
            BosunError::error(
                format!("cannot run `{program}`: {e}"),
                format!("install `{program}` and make sure it is on PATH"),
            )
        })?;
    let secs = (start.elapsed().as_secs_f64() * 1000.0).round() / 1000.0;
    Ok((status.code().map_or(-1, i64::from), secs))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stack_paths_are_normalised() {
        assert_eq!(stack_rel("."), "");
        assert_eq!(stack_rel("./web/"), "web");
        let changed = vec!["web/a.ts".to_string(), "src/b.rs".to_string()];
        assert_eq!(under_stack("web/", &changed), vec!["a.ts"]);
        assert_eq!(under_stack(".", &changed).len(), 2);
    }
}
