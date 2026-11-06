//! `bosun uninstall`: take back what `bosun adopt` wrote, as `.bosun/adopt.toml` records it.

use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use super::UninstallArgs;
use super::adopt::{known_paths, read_opt};
use crate::core::adopt::{MANIFEST_PATH, Manifest, fnv1a_hex};
use crate::ctx::{git_out, parse_remote_url, parse_repo};
use crate::github::{Github, IssuePatch, IssueState, LiveGithub, PatchState};
use crate::output::{BosunError, CmdResult, Outcome};

/// Why `rel` must not be touched: not a path adopt writes, or it (or a parent) is a symlink.
fn unsafe_reason(root: &Path, rel: &str, known: &[&str]) -> Option<String> {
    if !known.contains(&rel) {
        return Some(format!("{rel} is not a path bosun adopt writes"));
    }
    let mut p = root.to_path_buf();
    for part in rel.split('/') {
        p.push(part);
        if std::fs::symlink_metadata(&p).is_ok_and(|m| m.file_type().is_symlink()) {
            return Some(format!("{} is a symlink", p.display()));
        }
    }
    None
}

fn io_err(what: &str, path: &Path, e: std::io::Error) -> BosunError {
    BosunError::error(
        format!("cannot {what} {}: {e}", path.display()),
        "make the path accessible, then rerun `bosun uninstall`",
    )
}

pub fn run(dir: Option<&Path>, a: &UninstallArgs) -> CmdResult {
    let start = match dir {
        Some(d) => d.to_path_buf(),
        None => std::env::current_dir().map_err(|e| {
            BosunError::error(
                format!("cannot read the current directory: {e}"),
                "run bosun from inside the repo or pass -C <dir>",
            )
        })?,
    };
    let root: PathBuf = git_out(&start, &["rev-parse", "--show-toplevel"])
        .map_or_else(|| start.clone(), PathBuf::from);
    let Some(bytes) = read_opt(&root.join(MANIFEST_PATH))? else {
        return Err(BosunError::error(
            format!("{MANIFEST_PATH} is missing: nothing was adopted here"),
            "nothing adopted here; remove a splice-installed bosun by hand, see docs/adopt-existing-repo.md",
        ));
    };
    let m = Manifest::parse(&String::from_utf8_lossy(&bytes)).map_err(|e| {
        BosunError::error(
            format!("{MANIFEST_PATH}: {e}"),
            format!("fix {MANIFEST_PATH}, or remove the adopted files by hand (docs/adopt-existing-repo.md)"),
        )
    })?;

    let known = known_paths();
    let mut files: Vec<Value> = Vec::new();
    let mut deletes: Vec<&str> = Vec::new();
    for f in &m.files {
        if let Some(why) = unsafe_reason(&root, &f.path, &known) {
            files.push(json!({
                "path": f.path, "action": "kept",
                "fix": format!("{why}; bosun uninstall leaves it alone, remove it by hand if it is yours"),
            }));
            continue;
        }
        let action = match read_opt(&root.join(&f.path))? {
            None => "gone",
            Some(b) if fnv1a_hex(&b) == f.hash => {
                deletes.push(&f.path);
                "delete"
            }
            Some(_) => "kept",
        };
        files.push(json!({"path": f.path, "action": action}));
    }

    let mut labels: Vec<Value> = Vec::new();
    let mut registry = json!({"action": "skipped"});
    let mut keep_labels: Vec<String> = m.labels.clone();
    let mut keep_registry = m.registry_issue;
    if a.labels && a.dry_run {
        labels = m
            .labels
            .iter()
            .map(|l| json!({"name": l, "action": "delete"}))
            .collect();
        if let Some(n) = m.registry_issue {
            registry = json!({"action": "close", "number": n});
        }
    } else if a.labels && (!m.labels.is_empty() || m.registry_issue.is_some()) {
        // GitHub first: a failure there leaves the manifest, so the command can be rerun.
        let repo = std::env::var("BOSUN_REPO")
            .ok()
            .and_then(|v| parse_repo(&v))
            .or_else(|| {
                git_out(&root, &["remote", "get-url", "origin"]).and_then(|u| parse_remote_url(&u))
            })
            .ok_or_else(|| {
                BosunError::error(
                    "cannot tell which GitHub repo this is (no BOSUN_REPO and no parseable origin remote)",
                    "set BOSUN_REPO=owner/name, or run without --labels",
                )
            })?;
        let gh = LiveGithub::new(&repo.owner, &repo.name)?;
        if let Some(n) = m.registry_issue {
            gh.update_issue(
                n,
                IssuePatch {
                    state: Some(PatchState::Closed),
                    state_reason: Some("completed".into()),
                    ..Default::default()
                },
            )?;
            registry = json!({"action": "close", "number": n});
            keep_registry = None;
        }
        keep_labels.clear();
        for l in &m.labels {
            if gh.list_issues(&[l.as_str()], IssueState::Open)?.is_empty() {
                gh.delete_label(l)?;
                labels.push(json!({"name": l, "action": "delete"}));
            } else {
                labels.push(json!({
                    "name": l, "action": "in-use",
                    "fix": format!("open issues still carry {l}; close or relabel them, then run `bosun uninstall --labels` again"),
                }));
                keep_labels.push(l.clone());
            }
        }
    }

    if !a.dry_run {
        let mut removed: Vec<PathBuf> = Vec::new();
        for p in &deletes {
            let path = root.join(p);
            std::fs::remove_file(&path).map_err(|e| io_err("delete", &path, e))?;
            removed.push(path);
        }
        // The manifest stays while labels or the registry issue are left, for a later --labels.
        let manifest_path = root.join(MANIFEST_PATH);
        if keep_labels.is_empty() && keep_registry.is_none() {
            std::fs::remove_file(&manifest_path)
                .map_err(|e| io_err("delete", &manifest_path, e))?;
            removed.push(manifest_path);
        } else {
            let rest = Manifest {
                bosun_version: m.bosun_version.clone(),
                files: Vec::new(),
                labels: keep_labels,
                registry_issue: keep_registry,
            };
            std::fs::write(&manifest_path, rest.to_toml())
                .map_err(|e| io_err("write", &manifest_path, e))?;
        }
        // Empty directories that held them, innermost first; a non-empty one stays.
        for path in removed {
            let mut dir = path.parent();
            while let Some(d) = dir.filter(|d| *d != root && d.starts_with(&root)) {
                if std::fs::remove_dir(d).is_err() {
                    break;
                }
                dir = d.parent();
            }
        }
    }
    Ok(Outcome::data(json!({
        "dry_run": a.dry_run,
        "files": files,
        "labels": labels,
        "registry": registry,
    })))
}
