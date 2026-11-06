//! `bosun uninstall`: take back what `bosun adopt` wrote, as `.bosun/adopt.toml` records it.

use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use super::UninstallArgs;
use crate::core::adopt::{MANIFEST_PATH, Manifest, fnv1a_hex};
use crate::ctx::{git_out, parse_remote_url, parse_repo};
use crate::github::{Github, IssuePatch, LiveGithub, PatchState};
use crate::output::{BosunError, CmdResult, Outcome};

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
    let text = std::fs::read_to_string(root.join(MANIFEST_PATH)).map_err(|_| {
        BosunError::error(
            format!("{MANIFEST_PATH} is missing: nothing was adopted here"),
            "nothing adopted here; remove a splice-installed bosun by hand, see docs/adopt-existing-repo.md",
        )
    })?;
    let m = Manifest::parse(&text).map_err(|e| {
        BosunError::error(
            format!("{MANIFEST_PATH}: {e}"),
            format!("fix {MANIFEST_PATH}, or remove the adopted files by hand (docs/adopt-existing-repo.md)"),
        )
    })?;

    let mut files: Vec<Value> = Vec::new();
    let mut deletes: Vec<&str> = Vec::new();
    for f in &m.files {
        let action = match std::fs::read(root.join(&f.path)) {
            Err(_) => "gone",
            Ok(b) if fnv1a_hex(&b) == f.hash => {
                deletes.push(&f.path);
                "delete"
            }
            Ok(_) => "kept",
        };
        files.push(json!({"path": f.path, "action": action}));
    }
    let labels: Vec<Value> = if a.labels {
        m.labels
            .iter()
            .map(|l| json!({"name": l, "action": "delete"}))
            .collect()
    } else {
        Vec::new()
    };
    let registry = match (a.labels, m.registry_issue) {
        (true, Some(n)) => json!({"action": "close", "number": n}),
        _ => json!({"action": "skipped"}),
    };

    if !a.dry_run {
        // GitHub first: a failure there leaves the manifest, so the command can be rerun.
        if a.labels && (!m.labels.is_empty() || m.registry_issue.is_some()) {
            let repo = std::env::var("BOSUN_REPO")
                .ok()
                .and_then(|v| parse_repo(&v))
                .or_else(|| {
                    git_out(&root, &["remote", "get-url", "origin"])
                        .and_then(|u| parse_remote_url(&u))
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
            }
            for l in &m.labels {
                gh.delete_label(l)?;
            }
        }
        let mut removed: Vec<PathBuf> = Vec::new();
        for p in deletes.iter().copied().chain([MANIFEST_PATH]) {
            let path = root.join(p);
            std::fs::remove_file(&path).map_err(|e| {
                BosunError::error(
                    format!("cannot delete {}: {e}", path.display()),
                    "make the file deletable, then rerun `bosun uninstall`",
                )
            })?;
            removed.push(path);
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
