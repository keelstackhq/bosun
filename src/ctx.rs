//! The context every command runs in: repo, session, origin, clock, config (spec D30, D31).

use std::path::{Path, PathBuf};
use std::process::Command;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

use crate::config::{Config, Limits};
use crate::output::BosunError;
use crate::time::Timestamp;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Origin {
    Spawned,
    Slack,
    Local,
    Routine,
}

impl Origin {
    pub fn as_str(self) -> &'static str {
        match self {
            Origin::Spawned => "spawned",
            Origin::Slack => "slack",
            Origin::Local => "local",
            Origin::Routine => "routine",
        }
    }

    /// `BOSUN_ORIGIN` wins; else `spawned` when `CLAUDE_CODE_CHILD_SESSION=1`; else `local`.
    pub fn resolve(
        bosun_origin: Option<&str>,
        child_session: Option<&str>,
    ) -> Result<Origin, BosunError> {
        match bosun_origin.map(str::trim).filter(|s| !s.is_empty()) {
            Some(v) => v.parse(),
            None if child_session.map(str::trim) == Some("1") => Ok(Origin::Spawned),
            None => Ok(Origin::Local),
        }
    }
}

impl FromStr for Origin {
    type Err = BosunError;
    fn from_str(s: &str) -> Result<Origin, BosunError> {
        match s {
            "spawned" => Ok(Origin::Spawned),
            "slack" => Ok(Origin::Slack),
            "local" => Ok(Origin::Local),
            "routine" => Ok(Origin::Routine),
            other => Err(BosunError::error(
                format!("BOSUN_ORIGIN={other:?} is not an origin"),
                "set BOSUN_ORIGIN to one of spawned, slack, local, routine (or unset it)",
            )),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RepoId {
    pub owner: String,
    pub name: String,
}

impl std::fmt::Display for RepoId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}/{}", self.owner, self.name)
    }
}

/// `owner/name` (the `BOSUN_REPO` form).
pub fn parse_repo(s: &str) -> Option<RepoId> {
    let (owner, name) = s.trim().split_once('/')?;
    let ok = |p: &str| !p.is_empty() && !p.contains(['/', ':', '@', ' ']);
    (ok(owner) && ok(name)).then(|| RepoId {
        owner: owner.into(),
        name: name.into(),
    })
}

/// Parse an `origin` URL: `https://host/o/r(.git)`, `git@host:o/r.git`, `ssh://git@host/o/r.git`.
pub fn parse_remote_url(url: &str) -> Option<RepoId> {
    let url = url.trim();
    let path = match url.split_once("://") {
        Some((_, rest)) => rest.split_once('/')?.1,
        None => url.split_once('@')?.1.split_once(':')?.1,
    };
    let path = path.trim_end_matches('/');
    parse_repo(path.strip_suffix(".git").unwrap_or(path))
}

/// Session id (D31): `BOSUN_SESSION`, else the remote session id with `cse_` -> `session_`,
/// else `local-<host>`.
pub fn session_id(bosun_session: Option<&str>, remote_session: Option<&str>, host: &str) -> String {
    let set = |v: Option<&str>| v.map(str::trim).filter(|s| !s.is_empty()).map(String::from);
    if let Some(s) = set(bosun_session) {
        return s;
    }
    if let Some(s) = set(remote_session) {
        return match s.strip_prefix("cse_") {
            Some(rest) => format!("session_{rest}"),
            None => s,
        };
    }
    format!("local-{host}")
}

fn hostname() -> String {
    std::env::var("HOSTNAME")
        .ok()
        .or_else(|| std::fs::read_to_string("/etc/hostname").ok())
        .map(|h| h.trim().to_string())
        .filter(|h| !h.is_empty())
        .unwrap_or_else(|| "unknown".into())
}

fn git_out(dir: &Path, args: &[&str]) -> Option<String> {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .output()
        .ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).trim().to_string())
        .filter(|s| !s.is_empty())
}

fn env(name: &str) -> Option<String> {
    std::env::var(name).ok()
}

#[derive(Debug, Clone)]
pub struct Ctx {
    pub dir: PathBuf,
    pub repo: RepoId,
    pub session: String,
    pub origin: Origin,
    pub now: Timestamp,
    pub config: Config,
    pub limits: Limits,
}

impl Ctx {
    /// Build the context from the environment. `dir` is the `-C` directory, else the cwd.
    /// Config files are read from the git top-level (falling back to `dir`).
    pub fn load(dir: Option<&Path>) -> Result<Ctx, BosunError> {
        let dir = match dir {
            Some(d) => d.to_path_buf(),
            None => std::env::current_dir().map_err(|e| {
                BosunError::error(
                    format!("cannot read the current directory: {e}"),
                    "run bosun from inside the repo or pass -C <dir>",
                )
            })?,
        };
        let root = git_out(&dir, &["rev-parse", "--show-toplevel"])
            .map_or_else(|| dir.clone(), PathBuf::from);
        let no_repo = || {
            BosunError::error(
                "cannot tell which GitHub repo this is (no BOSUN_REPO and no parseable origin remote)",
                "set BOSUN_REPO=owner/name",
            )
        };
        let repo = match env("BOSUN_REPO") {
            Some(v) => parse_repo(&v).ok_or_else(|| {
                BosunError::error(
                    format!("BOSUN_REPO={v:?} is not owner/name"),
                    "set BOSUN_REPO=owner/name",
                )
            })?,
            None => git_out(&dir, &["remote", "get-url", "origin"])
                .and_then(|u| parse_remote_url(&u))
                .ok_or_else(no_repo)?,
        };
        let mut config = Config::load(&root)?;
        if config.project.is_empty() {
            config.project = repo.name.clone();
        }
        let now = Timestamp::resolve(env("BOSUN_NOW").as_deref()).map_err(|m| {
            BosunError::error(
                m,
                "set BOSUN_NOW to an RFC 3339 time such as 2026-10-22T09:00:00Z, or unset it",
            )
        })?;
        Ok(Ctx {
            limits: Limits::load(&root)?,
            session: session_id(
                env("BOSUN_SESSION").as_deref(),
                env("CLAUDE_CODE_REMOTE_SESSION_ID").as_deref(),
                &hostname(),
            ),
            origin: Origin::resolve(
                env("BOSUN_ORIGIN").as_deref(),
                env("CLAUDE_CODE_CHILD_SESSION").as_deref(),
            )?,
            dir,
            repo,
            now,
            config,
        })
    }
}
