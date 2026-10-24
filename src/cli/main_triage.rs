//! `bosun main-triage --good REV [--bad REV] [--run CMD]`. Bisects the first-parent commits
//! (merged PRs) between two revs in temporary worktrees. Works offline on the local checkout.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use serde_json::json;

use crate::cli::MainTriageArgs;
use crate::cli::gate::repo_root;
use crate::core::bisect;
use crate::output::{BosunError, CmdResult, Outcome};

struct Candidate {
    sha: String,
    subject: String,
}

fn git(root: &Path, args: &[&str]) -> Result<String, BosunError> {
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
            "check that --good and --bad name commits in this checkout",
        ));
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// Resolve a rev to a commit sha. A rev that starts with `-` is a usage error.
fn resolve(root: &Path, rev: &str) -> Result<String, BosunError> {
    if rev.starts_with('-') {
        return Err(BosunError::usage(
            format!("{rev:?} is not a rev"),
            "pass a commit, tag or branch name",
        ));
    }
    git(
        root,
        &[
            "rev-parse",
            "--verify",
            "--end-of-options",
            &format!("{rev}^{{commit}}"),
        ],
    )
}

fn candidates(root: &Path, good: &str, bad: &str) -> Result<Vec<Candidate>, BosunError> {
    let range = format!("{good}..{bad}");
    let shas = git(root, &["rev-list", "--first-parent", "--reverse", &range])?;
    shas.lines()
        .map(|sha| {
            Ok(Candidate {
                subject: git(root, &["log", "-1", "--format=%s", sha])?,
                sha: sha.to_string(),
            })
        })
        .collect()
}

/// Child output goes to stderr: stdout stays one JSON object.
#[cfg(unix)]
fn to_stderr() -> Stdio {
    use std::os::fd::AsFd;
    std::io::stderr()
        .as_fd()
        .try_clone_to_owned()
        .map_or_else(|_| Stdio::null(), Stdio::from)
}

/// Child output goes to stderr: stdout stays one JSON object.
#[cfg(windows)]
fn to_stderr() -> Stdio {
    use std::os::windows::io::AsHandle;
    std::io::stderr()
        .as_handle()
        .try_clone_to_owned()
        .map_or_else(|_| Stdio::null(), Stdio::from)
}

/// Elsewhere child output is dropped: it must never reach stdout.
#[cfg(not(any(unix, windows)))]
fn to_stderr() -> Stdio {
    Stdio::null()
}

/// Run `cmd` in a detached worktree at `sha`; the worktree is always removed. True = bad.
fn probe(root: &Path, sha: &str, cmd: &str) -> Result<bool, BosunError> {
    static N: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    let n = N.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let tree: PathBuf = std::env::temp_dir().join(format!(
        "bosun-triage-{}-{n}-{}",
        std::process::id(),
        &sha[..sha.len().min(12)]
    ));
    let tree_s = tree.to_string_lossy().to_string();
    git(root, &["worktree", "add", "--detach", &tree_s, sha])?;
    let status = Command::new("sh")
        .arg("-c")
        .arg(cmd)
        .current_dir(&tree)
        .stdin(Stdio::null())
        .stdout(to_stderr())
        .stderr(to_stderr())
        .status();
    let removed = git(root, &["worktree", "remove", "--force", &tree_s]);
    let status = status.map_err(|e| {
        BosunError::error(
            format!("cannot run the --run command: {e}"),
            "check that sh is on PATH",
        )
    })?;
    removed?;
    Ok(!status.success())
}

pub fn run(dir: Option<&Path>, args: &MainTriageArgs) -> CmdResult {
    let dir = dir.map(Path::to_path_buf).unwrap_or_else(|| ".".into());
    let root = repo_root(&dir)?;
    let cmd = match &args.run {
        Some(c) => c.clone(),
        None => {
            let exe = std::env::current_exe().map_err(|e| {
                BosunError::error(
                    format!("cannot find the bosun binary: {e}"),
                    "pass --run <command>",
                )
            })?;
            format!(
                "'{}' gate test --all",
                exe.display().to_string().replace('\'', "'\\''")
            )
        }
    };
    let good = resolve(&root, &args.good)?;
    let bad = resolve(&root, args.bad.as_deref().unwrap_or("HEAD"))?;
    let bad = bad.as_str();
    // Clean worktrees that a killed run left behind.
    git(&root, &["worktree", "prune"])?;
    let list = candidates(&root, &good, bad)?;
    if list.is_empty() {
        return Err(BosunError::usage(
            format!("no commits between {} and {bad}", good),
            "pass --good with a rev that is older than --bad on the first-parent line",
        ));
    }
    let found = bisect::first_bad(&list, |c| probe(&root, &c.sha, &cmd))?;
    // Every probe may fail for a reason that is not a regression (for example the command cannot
    // run at old commits). If the run fails at --good too, it judges nothing.
    if found.first_bad == Some(0) && probe(&root, &good, &cmd)? {
        return Err(BosunError::needs_human(
            "the run fails at --good too: the command does not judge these commits",
            "pass a --run that works at old commits, or an older --good",
        )
        .with_data(json!({"good": good, "bad": bad, "run": cmd})));
    }
    let first = found.first_bad.map(|i| {
        let c = &list[i];
        json!({"sha": c.sha, "subject": c.subject, "pr": bisect::pr_number(&c.subject)})
    });
    let mut data = json!({
        "good": good,
        "bad": bad,
        "run": cmd,
        "candidates": list.len(),
        "runs": found.probes.len(),
        "first_bad": first,
    });
    if found.first_bad.is_none() {
        data["note"] = json!(format!("main passes at {bad}"));
    }
    Ok(Outcome::data(data))
}
