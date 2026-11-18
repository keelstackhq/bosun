//! The no-CI gates (spec §4, D-019). What must pass is read from the default branch's config and
//! profiles at its current sha, never from a checkout or from the PR. `bosun merge` runs those
//! gates itself on a throwaway worktree of the exact PR head; stamp files are never trusted.

use std::path::{Path, PathBuf};

use serde_json::json;

use crate::app::{App, default_ref};
use crate::cli::stamps;
use crate::config::{CONFIG_FILE, Config};
use crate::gates::{self, Request};
use crate::github::{Github, Pull};
use crate::output::BosunError;

fn io_err(what: &str, e: std::io::Error) -> BosunError {
    BosunError::error(
        format!("{what}: {e}"),
        "check that the temp directory is writable",
    )
}

/// Files that steer how the gates build and run; the base's version (or none) always wins.
const TOOL_FILES: [&str; 9] = [
    ".cargo/config",
    ".cargo/config.toml",
    "rust-toolchain",
    "rust-toolchain.toml",
    "clippy.toml",
    ".clippy.toml",
    "deny.toml",
    "rustfmt.toml",
    ".rustfmt.toml",
];

/// Exit 5 when `rel` or any parent of it below `root` is a symlink: a write there would land
/// outside the worktree.
fn no_symlink(root: &Path, rel: &str) -> Result<(), BosunError> {
    let mut at = root.to_path_buf();
    for part in rel.split('/') {
        at.push(part);
        if std::fs::symlink_metadata(&at).is_ok_and(|m| m.file_type().is_symlink()) {
            return Err(BosunError::needs_human(
                format!(
                    "{rel} is (or sits below) a symlink in the PR head, so the gates will not run there"
                ),
                "the author replaces the symlink with a regular file or directory, then reruns `bosun pr <task> --ready`",
            ));
        }
    }
    Ok(())
}

/// The default branch's `.bosun/bosun.toml`, profiles and [`TOOL_FILES`] (also under each stack
/// directory and its parents), as `(path, text or none)`. `None` means the base lacks the file.
struct Overlay {
    files: Vec<(String, Option<String>)>,
    config: Config,
}

impl Overlay {
    fn load(gh: &dyn Github, git_ref: &str) -> Result<Overlay, BosunError> {
        let text = gh.file_at(CONFIG_FILE, git_ref)?.unwrap_or_default();
        let config = Config::parse(&text, CONFIG_FILE)?;
        let mut files: Vec<(String, Option<String>)> = vec![(CONFIG_FILE.into(), Some(text))];
        let mut dirs = vec![String::new()];
        for stack in &config.stack {
            let mut at = String::new();
            for part in gates::stack_rel(&stack.path)
                .split('/')
                .filter(|p| !p.is_empty())
            {
                at = format!("{at}{part}/");
                dirs.push(at.clone());
            }
            let file = format!(".bosun/profiles/{}.toml", stack.profile);
            files.push((file.clone(), gh.file_at(&file, git_ref)?));
        }
        dirs.dedup();
        for d in dirs {
            for f in TOOL_FILES {
                let rel = format!("{d}{f}");
                if !files.iter().any(|(p, _)| *p == rel) {
                    files.push((rel.clone(), gh.file_at(&rel, git_ref)?));
                }
            }
        }
        Ok(Overlay { files, config })
    }

    /// Put the base's files into `root`, deleting any the base lacks. Nothing is touched when a
    /// target is a symlink.
    fn apply(&self, root: &Path) -> Result<(), BosunError> {
        no_symlink(root, ".bosun/profiles")?;
        for (rel, _) in &self.files {
            no_symlink(root, rel)?;
        }
        let _ = std::fs::remove_dir_all(root.join(".bosun/profiles"));
        for (rel, text) in &self.files {
            let path = root.join(rel);
            match text {
                Some(t) => {
                    if let Some(dir) = path.parent() {
                        std::fs::create_dir_all(dir).map_err(|e| io_err("overlay", e))?;
                    }
                    std::fs::write(&path, t).map_err(|e| io_err("overlay", e))?;
                }
                None => {
                    let _ = std::fs::remove_file(&path);
                }
            }
        }
        Ok(())
    }

    /// Whether every file in `root` is still exactly the base's (or absent where the base lacks it).
    fn intact(&self, root: &Path) -> bool {
        (self.files.iter()).all(|(rel, text)| std::fs::read_to_string(root.join(rel)).ok() == *text)
    }
}

fn non_empty(all: Vec<String>) -> Result<Vec<String>, BosunError> {
    if all.iter().all(|g| g == "test-integrity") {
        return Err(BosunError::needs_human(
            "the default branch's profiles require no gate besides test-integrity, so nothing can vouch for the head",
            "an owner fixes the stacks and profiles on the default branch",
        ));
    }
    Ok(all)
}

/// A fresh empty directory under the temp dir (`create_dir` fails on an existing one).
fn fresh_dir(tag: &str) -> Result<PathBuf, BosunError> {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.subsec_nanos());
    for i in 0..20u32 {
        let p =
            std::env::temp_dir().join(format!("bosun-{tag}-{}-{nanos:x}{i}", std::process::id()));
        match std::fs::create_dir(&p) {
            Ok(()) => return Ok(p),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(e) => return Err(io_err("temp dir", e)),
        }
    }
    Err(io_err("temp dir", std::io::ErrorKind::AlreadyExists.into()))
}

/// A throwaway detached worktree, removed and pruned on every path (a panic too).
struct Worktree<'a> {
    repo: &'a Path,
    path: PathBuf,
}

impl Drop for Worktree<'_> {
    fn drop(&mut self) {
        let p = self.path.to_string_lossy().to_string();
        let _ = gates::git(self.repo, &["worktree", "remove", "--force", &p]);
        let _ = std::fs::remove_dir_all(&self.path);
        let _ = gates::git(self.repo, &["worktree", "prune"]);
    }
}

/// The gates a PR must pass, from the default branch's config at its current sha.
pub fn required_gates(gh: &dyn Github) -> Result<Vec<String>, BosunError> {
    let tmp = fresh_dir("gates")?;
    let out = Overlay::load(gh, &default_ref(gh)?)
        .and_then(|o| o.apply(&tmp))
        .and_then(|()| stamps::ready_gates(&tmp));
    let _ = std::fs::remove_dir_all(&tmp);
    non_empty(out?)
}

fn sha40(s: &str) -> bool {
    s.len() == 40 && s.bytes().all(|b| b.is_ascii_hexdigit())
}

/// Run the required gates on a detached worktree of `pull`'s head (fetched when absent); exit 5
/// with the failing gates unless all pass. The worktree is always removed. The gates execute the
/// head's code (tests, build scripts): the signed verdict and the owner's approval are the control.
pub fn verify(app: &App, pull: &Pull) -> Result<Vec<String>, BosunError> {
    let (dir, head) = (app.ctx.dir.as_path(), pull.head_sha.as_str());
    if !sha40(head) || !sha40(&pull.base_sha) {
        return Err(BosunError::needs_human(
            format!(
                "PR #{} head or base is not a 40-hex commit id, so its gates cannot run",
                pull.number
            ),
            format!(
                "rerun `bosun merge {}` once GitHub reports real commit ids",
                pull.number
            ),
        ));
    }
    let have = |sha: &str| stamps::tree_of(dir, sha).is_some();
    for (sha, spec) in [
        (head, format!("pull/{}/head", pull.number)),
        (&*pull.base_sha, pull.base_sha.clone()),
    ] {
        if !have(sha) {
            let _ = gates::git(dir, &["fetch", "-q", "origin", &spec]);
        }
        if !have(sha) {
            return Err(BosunError::needs_human(
                format!(
                    "commit {sha} of PR #{} is not in this checkout, so its gates cannot run",
                    pull.number
                ),
                format!(
                    "`git fetch origin {spec}`, then rerun `bosun merge {}`",
                    pull.number
                ),
            ));
        }
    }
    let wt = Worktree {
        repo: dir,
        path: fresh_dir(&format!("merge-{}", pull.number))?,
    };
    gates::git(
        dir,
        &[
            "worktree",
            "add",
            "-q",
            "--detach",
            "--force",
            &wt.path.to_string_lossy(),
            head,
        ],
    )?;
    run_gates(app, pull, &wt.path)
}

fn run_gates(app: &App, pull: &Pull, wt: &Path) -> Result<Vec<String>, BosunError> {
    let gh = app.gh.as_ref();
    let overlay = Overlay::load(gh, &default_ref(gh)?)?;
    overlay.apply(wt)?;
    let all = non_empty(stamps::ready_gates(wt)?)?;
    let config = &overlay.config;
    let mut failed = Vec::new();
    for gate in &all {
        // Fresh before each gate, and checked after: an earlier gate (head code) cannot steer a later one.
        overlay.apply(wt)?;
        let res = if gate == "test-integrity" {
            super::test_integrity::check_with(Some(wt), Some(&pull.base_sha), &pull.body, &[])
        } else {
            gates::run(&Request {
                root: wt,
                stacks: &config.stack,
                gate,
                base: "HEAD",
                all: true,
            })
        };
        if !overlay.intact(wt) {
            return Err(BosunError::needs_human(
                format!(
                    "gate {gate} of PR #{} changed the default branch's config or tool files",
                    pull.number
                ),
                "the author removes whatever in the head rewrites them; the owner reviews the head",
            ));
        }
        if res.is_err() {
            failed.push(gate.clone());
        }
    }
    if failed.is_empty() {
        return Ok(all);
    }
    let n = pull.number;
    Err(BosunError::needs_human(
        format!("PR #{n} head {} fails its gates: {}", pull.head_sha, failed.join(", ")),
        format!("the author fixes them on the PR branch and reruns `bosun pr <task> --ready`, then rerun `bosun merge {n}`"),
    )
    .with_data(json!({"pr": n, "head_sha": pull.head_sha, "failed_gates": failed})))
}
