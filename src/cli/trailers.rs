//! `bosun trailers`: commit trailers for traceability (spec §4). `stamp` runs offline from a
//! git commit-msg hook; `install` writes that hook; `list` finds one session's commits.

use std::path::{Path, PathBuf};

use serde_json::json;

use super::{TrailersArgs, TrailersCmd};
use crate::core::trailers::{MODEL, SESSION, TASK, stamp};
use crate::ctx::session_id;
use crate::hooks::{cache, git_out};
use crate::output::{BosunError, CmdResult, Outcome};
use crate::time::Timestamp;

const MARK: &str = "# bosun:trailers";
const HOOK: &str = "#!/bin/sh\n# bosun:trailers\n# Stamp Bosun-Session/Task/Model trailers; never block a commit.\ncommand -v bosun >/dev/null 2>&1 && bosun trailers stamp \"$1\" >/dev/null 2>&1\nexit 0\n";

fn env(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|v| !v.trim().is_empty())
}

pub fn run(dir: Option<&Path>, args: &TrailersArgs) -> CmdResult {
    let dir = dir.map_or_else(|| PathBuf::from("."), Path::to_path_buf);
    match &args.cmd {
        TrailersCmd::Stamp { file } => stamp_file(&dir, file),
        TrailersCmd::Install => install(&dir).map(Outcome::data),
        TrailersCmd::List { session } => list(&dir, session),
    }
}

fn stamp_file(dir: &Path, file: &Path) -> CmdResult {
    let msg = std::fs::read_to_string(file).map_err(|e| {
        BosunError::error(
            format!("cannot read {}: {e}", file.display()),
            "pass the commit message file git gives the commit-msg hook",
        )
    })?;
    // No session id known (a human commit): leave the trailer out.
    let session =
        if env("BOSUN_SESSION").is_some() || env("CLAUDE_CODE_REMOTE_SESSION_ID").is_some() {
            session_id(
                env("BOSUN_SESSION").as_deref(),
                env("CLAUDE_CODE_REMOTE_SESSION_ID").as_deref(),
                "unknown",
            )
        } else {
            String::new()
        };
    let now = env("BOSUN_NOW")
        .and_then(|v| crate::time::parse_rfc3339(&v))
        .unwrap_or_else(Timestamp::now);
    // The task: BOSUN_TASK, else the issue of this session's cached lease (offline).
    let task = env("BOSUN_TASK")
        .or_else(|| cache::cached_issue(dir, &session, now).map(|n| n.to_string()))
        .unwrap_or_default();
    let model = env("BOSUN_MODEL")
        .or_else(|| env("ANTHROPIC_MODEL"))
        .unwrap_or_default();
    let want = [(SESSION, session), (TASK, task), (MODEL, model)];
    let out = stamp(&msg, &want);
    let changed = out != msg;
    if changed {
        std::fs::write(file, out).map_err(|e| {
            BosunError::error(
                format!("cannot write {}: {e}", file.display()),
                "check the file is writable",
            )
        })?;
    }
    Ok(Outcome::data(json!({"stamped": changed})))
}

/// Write the commit-msg hook unless another hook is there. Idempotent.
pub fn install(dir: &Path) -> Result<serde_json::Value, BosunError> {
    let path = git_out(dir, &["rev-parse", "--git-path", "hooks/commit-msg"])
        .map(|p| dir.join(p))
        .ok_or_else(|| {
            BosunError::error(
                "not in a git repository",
                "run `bosun trailers install` inside the checkout",
            )
        })?;
    match std::fs::read_to_string(&path) {
        Ok(old) if old == HOOK => return Ok(json!({"hook": path.display().to_string()})),
        Ok(old) if old.contains(MARK) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => {
            return Err(BosunError::error(
                format!("cannot read {}: {e}", path.display()),
                "check the hooks directory permissions, then rerun",
            ));
        }
        Ok(_) => {
            return Err(BosunError::conflict(
                format!("{} is another hook", path.display()),
                "add the line `bosun trailers stamp \"$1\" || true` to it, or move it away and rerun",
            ));
        }
    }
    let io = |e: std::io::Error| {
        BosunError::error(
            format!("cannot write {}: {e}", path.display()),
            "check the hooks directory is writable",
        )
    };
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(io)?;
    }
    std::fs::write(&path, HOOK).map_err(io)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).map_err(io)?;
    }
    Ok(json!({"hook": path.display().to_string()}))
}

fn list(dir: &Path, session: &str) -> CmdResult {
    // The recipe reverts onto origin/main, so search what main has (HEAD when there is no remote).
    let base = if git_out(dir, &["rev-parse", "--verify", "-q", "origin/main"]).is_some() {
        "origin/main".to_string()
    } else {
        "HEAD".to_string()
    };
    let raw = git_out(
        dir,
        &[
            "log",
            &base,
            "--fixed-strings",
            &format!("--grep={SESSION}: {session}"),
            "--format=%H%x1f%s%x1f%B%x1e",
        ],
    )
    .unwrap_or_default();
    if git_out(dir, &["rev-parse", "--git-dir"]).is_none() {
        return Err(BosunError::error(
            "not in a git repository",
            "run `bosun trailers list` inside the checkout",
        ));
    }
    let line = format!("{SESSION}: {session}");
    let prefix = format!("{line}/");
    let mut commits = Vec::new();
    for rec in raw.split('\x1e') {
        let mut f = rec.trim().splitn(3, '\x1f');
        let (Some(sha), Some(subject), Some(body)) = (f.next(), f.next(), f.next()) else {
            continue;
        };
        if body.lines().any(|l| l == line || l.starts_with(&prefix)) {
            commits.push(json!({"sha": sha, "subject": subject}));
        }
    }
    let shas: Vec<&str> = commits.iter().filter_map(|c| c["sha"].as_str()).collect();
    let recipe = (!shas.is_empty()).then(|| {
        format!(
            "git switch -c revert/{} origin/main && git revert --no-edit {}",
            session
                .chars()
                .map(|c| if c.is_ascii_alphanumeric() || c == '.' || c == '_' {
                    c
                } else {
                    '-'
                })
                .collect::<String>(),
            shas.join(" ")
        )
    });
    Ok(Outcome::data(json!({
        "session": session, "commits": commits, "revert": recipe,
    })))
}

/// `bosun pr`: the PR's commits (merge commits skipped) that lack a required trailer. With
/// `require` a gap is an error; else it is only reported in the returned `data.trailers`.
pub fn check_pr(
    gh: &dyn crate::github::Github,
    pr: u64,
    require: bool,
) -> Result<serde_json::Value, BosunError> {
    let mut checked = 0;
    let mut gaps = Vec::new();
    let mut unread = 0;
    let shas = match gh.pull_commits(pr) {
        Ok(s) => s,
        Err(e) if !require => {
            eprintln!("bosun: cannot list the commits of PR #{pr}: {}", e.message);
            Vec::new()
        }
        Err(e) => return Err(e),
    };
    if require && shas.len() >= 250 {
        return Err(BosunError::error(
            format!(
                "PR #{pr} has 250 or more commits, more than can be listed, so trailers cannot be fully checked"
            ),
            "split the work into smaller PRs",
        ));
    }
    for sha in shas {
        match gh.commit(&sha) {
            Ok(Some(c)) if c.parents.len() < 2 => {
                checked += 1;
                let miss = crate::core::trailers::missing(&c.message);
                if !miss.is_empty() {
                    gaps.push(json!({"sha": sha, "missing": miss}));
                }
            }
            Ok(Some(_)) => {}
            _ => unread += 1,
        }
    }
    let data = json!({"checked": checked, "unread": unread, "required": require, "missing": gaps});
    if require && (!gaps.is_empty() || unread > 0) {
        let shas: Vec<&str> = gaps.iter().filter_map(|g| g["sha"].as_str()).collect();
        if shas.is_empty() {
            return Err(BosunError::error(
                format!(
                    "PR #{pr}: {unread} commit(s) could not be read, so trailers cannot be checked"
                ),
                "rerun `bosun pr` in a minute, or set `[trailers] require = false`",
            )
            .with_data(json!({"pr": pr, "trailers": data})));
        }
        return Err(BosunError::error(
            format!(
                "PR #{pr} has {} commit(s) without Bosun-Session/Bosun-Task trailers: {}",
                shas.len(),
                shas.join(", ")
            ),
            "close this PR, run `bosun trailers install`, redo the commits stamped on a new branch (`git switch -c <new> origin/main && git merge --squash <old> && git commit`), push it and run `bosun pr` from it (never force-push)",
        )
        .with_data(json!({"pr": pr, "trailers": data})));
    }
    Ok(data)
}
