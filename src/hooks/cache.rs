//! The lease cache: `<git common dir>/bosun/leases.json`, so a burst of tool calls reads GitHub
//! once (spec §4). Written atomically; fresh for `BOSUN_LEASE_CACHE_SECS` (default 60).

use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use crate::app::{App, Result, active_issues};
use crate::core::lease::{Lease, TTL_SECS, live_leases};
use crate::hooks::git_out;
use crate::time::{Timestamp, parse_rfc3339};

const DEFAULT_FRESH_SECS: i64 = 60;

fn fresh_secs() -> i64 {
    std::env::var("BOSUN_LEASE_CACHE_SECS")
        .ok()
        .and_then(|v| v.trim().parse().ok())
        .unwrap_or(DEFAULT_FRESH_SECS)
}

/// `<git common dir>/bosun/leases.json`; `None` outside a git repo.
pub fn path(dir: &Path) -> Option<PathBuf> {
    let common = git_out(dir, &["rev-parse", "--git-common-dir"])?;
    Some(dir.join(common).join("bosun").join("leases.json"))
}

fn lease_json(l: &Lease) -> Value {
    json!({"issue": l.issue, "session": l.session, "origin": l.origin,
        "expires": l.expires.to_string(), "touch": l.touch, "comment_id": l.comment_id})
}

fn lease_of(v: &Value) -> Option<Lease> {
    Some(Lease {
        issue: v.get("issue")?.as_u64()?,
        session: v.get("session")?.as_str()?.into(),
        origin: v.get("origin")?.as_str()?.into(),
        expires: parse_rfc3339(v.get("expires")?.as_str()?)?,
        touch: v
            .get("touch")?
            .as_array()?
            .iter()
            .filter_map(|t| t.as_str().map(String::from))
            .collect(),
        comment_id: v.get("comment_id")?.as_u64()?,
    })
}

/// The cached leases and when they were fetched.
fn read(file: &Path) -> Option<(Timestamp, Vec<Lease>)> {
    let v: Value = serde_json::from_str(&std::fs::read_to_string(file).ok()?).ok()?;
    let at = Timestamp(v.get("fetched_at")?.as_i64()?);
    let leases = v.get("leases")?.as_array()?.iter().filter_map(lease_of);
    Some((at, leases.collect()))
}

fn write(file: &Path, now: Timestamp, leases: &[Lease]) -> std::io::Result<()> {
    let list: Vec<Value> = leases.iter().map(lease_json).collect();
    let body = json!({"fetched_at": now.0, "leases": list});
    if let Some(parent) = file.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let tmp = file.with_extension(format!("json.tmp{}", std::process::id()));
    std::fs::write(&tmp, body.to_string())?;
    std::fs::rename(&tmp, file)
}

/// Read the live leases from GitHub and store them in the cache (when there is one).
pub fn refresh(app: &App, file: Option<&Path>) -> Result<Vec<Lease>> {
    let issues: Vec<_> = active_issues(app.gh.as_ref())?
        .into_iter()
        .map(|(i, notes)| (i.number, notes))
        .collect();
    let leases = live_leases(&issues, app.ctx.now);
    if let Some(file) = file
        && let Err(e) = write(file, app.ctx.now, &leases)
    {
        eprintln!("bosun: warning: cannot write the lease cache: {e}");
    }
    Ok(leases)
}

/// Live leases for the edit-lock: the fresh cache, else GitHub, else a cache younger than the
/// lease TTL (with a warning). `Err` means nothing usable: the caller fails open.
pub fn leases(app: &App, file: Option<&Path>) -> std::result::Result<Vec<Lease>, String> {
    let now = app.ctx.now;
    let cached = file.and_then(read);
    let live =
        |ls: Vec<Lease>| -> Vec<Lease> { ls.into_iter().filter(|l| l.is_live(now)).collect() };
    let age = |at: Timestamp| now.0 - at.0;
    if let Some((at, ls)) = cached.clone()
        && (0..fresh_secs()).contains(&age(at))
    {
        return Ok(live(ls));
    }
    match refresh(app, file) {
        Ok(ls) => Ok(ls),
        Err(e) => match cached {
            Some((at, ls)) if (0..TTL_SECS).contains(&age(at)) => {
                eprintln!(
                    "bosun: warning: cannot reach GitHub ({}); using the lease cache from {}s ago",
                    e.message,
                    age(at)
                );
                Ok(live(ls))
            }
            _ => Err(e.message),
        },
    }
}
