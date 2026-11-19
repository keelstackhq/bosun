//! The no-CI gates (spec §4, D-019): the base's required gates, run on a private repo at the exact
//! PR head; stamp files are never trusted.

use std::path::{Path, PathBuf};

use serde_json::json;

use crate::app::{App, default_ref};
use crate::cli::stamps;
use crate::config::{CONFIG_FILE, Config};
use crate::gates::{self, Request};
use crate::github::{Github, Pull};
use crate::output::BosunError;

fn io_err(what: &str, e: std::io::Error) -> BosunError {
    BosunError::error(format!("{what}: {e}"), "make the temp dir writable")
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

/// Exit 5 when `rel` or a parent of it below `root` is a symlink (a write would leave the worktree).
fn no_symlink(root: &Path, rel: &str) -> Result<(), BosunError> {
    let mut at = root.to_path_buf();
    for part in rel.split('/') {
        at.push(part);
        if std::fs::symlink_metadata(&at).is_ok_and(|m| m.file_type().is_symlink()) {
            return Err(BosunError::needs_human(
                format!(
                    "{rel} is (or is below) a symlink in the PR head, so the gates will not run"
                ),
                "the author replaces the symlink with a regular file or directory, then reruns `bosun pr <task> --ready`",
            ));
        }
    }
    Ok(())
}

/// The base's `.bosun/bosun.toml`, profiles and [`TOOL_FILES`] (also in each stack directory and its
/// parents) as `(path, text)`; `None` means the base lacks the file.
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

    /// Put the base's files into `root`, deleting any it lacks; a symlink target touches nothing.
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

/// A fresh empty directory (mode 0700) under the temp dir, with an unpredictable name.
fn fresh_dir(tag: &str) -> Result<PathBuf, BosunError> {
    let mut raw = [0u8; 12];
    ring::rand::SecureRandom::fill(&ring::rand::SystemRandom::new(), &mut raw)
        .map_err(|_| BosunError::error("no random bytes for a temp dir", "retry"))?;
    let rnd = raw.iter().fold(String::new(), |a, b| format!("{a}{b:02x}"));
    let p = std::env::temp_dir().join(format!("bosun-{tag}-{rnd}"));
    let mut dir = std::fs::DirBuilder::new();
    #[cfg(unix)]
    std::os::unix::fs::DirBuilderExt::mode(&mut dir, 0o700);
    dir.create(&p).map_err(|e| io_err("temp dir", e))?;
    Ok(p)
}

/// The private temp tree of one run (repo, HOME, CARGO_HOME, TMPDIR), removed on every path.
struct Private(PathBuf);

impl Drop for Private {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Fetch settings: verify objects, https only (`file` for the test source), no credentials, no stalls.
const FETCH_CONFIG: &str = concat!(
    "transfer.fsckObjects=true fetch.fsckObjects=true protocol.allow=never protocol.https.allow=always ",
    "protocol.file.allow=user credential.helper= core.askPass=/bin/false ",
    "http.lowSpeedLimit=1000 http.lowSpeedTime=60"
);

/// Exit 5 when a `.cargo` exists in any parent of the repo or of the gate's new HOME, CARGO_HOME
/// or TMPDIR (so also `/tmp` and `/`): cargo reads those, and a runner there would run code.
fn clean_above(repo: &Path, gate: &Path) -> Result<(), BosunError> {
    let at = [repo.parent().unwrap_or(repo), gate];
    let bad = (at.iter().flat_map(|d| d.ancestors()))
        .find(|a| a.join(".cargo").symlink_metadata().is_ok());
    let Some(bad) = bad else { return Ok(()) };
    Err(BosunError::needs_human(
        format!("{}/.cargo exists above the gates' temp repo", bad.display()),
        "remove it or point TMPDIR at a clean directory, then rerun `bosun merge`",
    ))
}

/// The gates a PR must pass, from the default branch's config at its current sha.
pub fn required_gates(gh: &dyn Github) -> Result<Vec<String>, BosunError> {
    let tmp = Private(fresh_dir("gates")?);
    let overlay = Overlay::load(gh, &default_ref(gh)?)?;
    overlay.apply(&tmp.0)?;
    non_empty(stamps::ready_gates(&tmp.0)?)
}

/// Run the required gates on a private repo of `pull`'s head and base (fetched by sha, removed on
/// every path); exit 5 with the failing gates. They run the head's code: the signed verdict and the
/// owner's approval are the control.
pub fn verify(app: &App, pull: &Pull) -> Result<Vec<String>, BosunError> {
    let (head, n) = (pull.head_sha.as_str(), pull.number);
    let sha40 = |s: &str| s.len() == 40 && s.bytes().all(|b| b.is_ascii_hexdigit());
    if !sha40(head) || !sha40(&pull.base_sha) {
        return Err(BosunError::needs_human(
            format!("PR #{n} head or base is not a 40-hex commit id"),
            format!("rerun `bosun merge {n}` once GitHub reports real commit ids"),
        ));
    }
    let private = Private(fresh_dir(&format!("merge-{n}"))?);
    let repo = private.0.join("repo");
    std::fs::create_dir(&repo).map_err(|e| io_err("temp dir", e))?;
    gates::isolate(&private.0.join("setup"));
    clean_above(&repo, &private.0)?;
    let vis = app.gh.repo_info().ok().flatten().and_then(|i| i.visibility);
    let public = vis.is_none_or(|v| v == "public");
    // Never the shared checkout (its config and objects are not trusted): fetch by sha, no token.
    // `BOSUN_GIT_URL` (a test source) counts in debug builds only.
    let host = format!("https://github.com/{}.git", app.ctx.repo);
    let url = (std::env::var("BOSUN_GIT_URL")
        .ok()
        .filter(|_| cfg!(debug_assertions)))
    .unwrap_or(host);
    gates::git(&repo, &["init", "-q"])?;
    let mut args: Vec<&str> = FETCH_CONFIG.split(' ').flat_map(|c| ["-c", c]).collect();
    args.extend(["fetch", "-q", "--no-tags", &url, head, &pull.base_sha]);
    let got = public
        && gates::git(&repo, &args).is_ok()
        && [head, &pull.base_sha]
            .iter()
            .all(|s| stamps::tree_of(&repo, s).is_some());
    if !got {
        let why = if public {
            "cannot be fetched anonymously"
        } else {
            "is in a private repo"
        };
        return Err(BosunError::needs_human(
            format!("PR #{n} {why}, so its gates cannot run"),
            format!(
                "bosun merge needs a public repo; the owner merges PR #{n} on GitHub once the checks are green"
            ),
        ));
    }
    let tree = gates::git(&repo, &["rev-parse", &format!("{head}^{{tree}}")])?;
    gates::git(&repo, &["checkout", "-q", "--detach", head])?;
    run_gates(app, pull, &repo, tree.trim())
}

fn run_gates(app: &App, pull: &Pull, wt: &Path, tree: &str) -> Result<Vec<String>, BosunError> {
    // The PR's base commit, not the branch name: it cannot move between the checks and the run.
    let overlay = Overlay::load(app.gh.as_ref(), &pull.base_sha)?;
    overlay.apply(wt)?;
    let all = non_empty(stamps::ready_gates(wt)?)?;
    let config = &overlay.config;
    let mut failed = Vec::new();
    let private = wt.parent().unwrap_or(wt);
    for (i, gate) in all.iter().enumerate() {
        // Fresh before each gate, and checked after: an earlier gate (head code) cannot steer a later one.
        overlay.apply(wt)?;
        let own = private.join(format!("gate{i}"));
        gates::isolate(&own);
        clean_above(wt, &own)?;
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
        let same = gates::git(wt, &["rev-parse", "HEAD^{tree}"]).is_ok_and(|t| t.trim() == tree);
        if !overlay.intact(wt) || !same {
            return Err(BosunError::needs_human(
                format!(
                    "gate {gate} of PR #{} changed the base's config or tool files, or the checked-out commit",
                    pull.number
                ),
                "the author removes whatever in the head rewrites them; the owner reviews the head",
            ));
        }
        // A gate that skipped vouches for nothing.
        if !res.is_ok_and(|o| !o.data.to_string().contains("\"skipped\":true")) {
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
