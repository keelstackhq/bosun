//! `bosun hook stop`: a child with a live lease and no `result` or `handoff` is blocked once, and
//! a stale gate run is mentioned (spec §5, §7). It never loops and fails open.

use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::{Value, json};

use super::{cache, git_out};
use crate::app::{App, trusted_notes};
use crate::core::gate_record::{GateRecord, fingerprint, is_stale, wait_marker_live};
use crate::core::records::{Note, Record, parse_record};
use crate::ctx::Origin;
use crate::github::IssueState;
use crate::registry;

/// Run the hook: a panic allows (the session may stop).
pub fn run(cwd: Option<&Path>, input: &Value) -> Option<String> {
    if input.get("stop_hook_active").and_then(Value::as_bool) == Some(true) {
        return None;
    }
    let dir = cwd
        .map(Path::to_path_buf)
        .or_else(|| std::env::current_dir().ok())?;
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(|_| {
        eprintln!("bosun: internal error in the Stop hook; allowing");
    }));
    let judged = std::panic::catch_unwind(|| judge(&dir));
    std::panic::set_hook(previous);
    judged.unwrap_or(None)
}

fn judge(dir: &Path) -> Option<String> {
    let block = unfinished(dir).unwrap_or_else(|e| {
        eprintln!("bosun: warning: Stop hook cannot check the lease ({e}); allowing");
        None
    });
    let warn = stale_gate(dir).then_some(
        "bosun: the work tree changed since the last passing `bosun gate`; run `bosun gate` \
         before you finish",
    );
    let mut out = serde_json::Map::new();
    if let Some(reason) = block {
        out.insert("decision".into(), json!("block"));
        out.insert("reason".into(), json!(reason));
    }
    if let Some(w) = warn {
        out.insert("systemMessage".into(), json!(w));
    }
    (!out.is_empty()).then(|| Value::Object(out).to_string())
}

/// The block reason when this session holds a live lease without a later result or handoff.
/// Only a session no human answers is blocked (spawned children and routine roots): for a
/// local or Slack session a stop ends a turn, not the session, and it reads nothing.
fn unfinished(dir: &Path) -> Result<Option<String>, String> {
    let app = App::load(Some(dir)).map_err(|e| e.message)?;
    let origin = app.ctx.origin;
    if matches!(origin, Origin::Local | Origin::Slack) {
        return Ok(None);
    }
    let gh = app.gh.as_ref();
    let me = app.ctx.session.as_str();
    let mine = |r: &Record| {
        matches!(r, Record::Result { session, .. } | Record::Handoff { session, .. }
            if session == me)
    };
    let done_after = |notes: &[Note], id: u64| {
        notes
            .iter()
            .filter(|n| n.id > id)
            .filter_map(|n| parse_record(&n.body))
            .any(|r| mine(&r))
    };
    if waiting(dir, me) {
        return Ok(None);
    }
    let leases = cache::leases(&app, cache::path(dir).as_deref())?;
    // `bosun handoff` (roots only) posts on the registry; read it once, only when needed.
    let mut registry: Option<Vec<Note>> = None;
    for lease in leases.iter().filter(|l| l.session == me) {
        let notes = trusted_notes(gh, lease.issue).map_err(|e| e.message)?;
        if done_after(&notes, lease.comment_id) {
            continue;
        }
        if origin != Origin::Spawned {
            if registry.is_none() {
                let issue = gh.list_issues(&[registry::LABEL], IssueState::Open);
                registry = Some(match issue.map_err(|e| e.message)?.first() {
                    Some(i) => trusted_notes(gh, i.number).map_err(|e| e.message)?,
                    None => Vec::new(),
                });
            }
            if done_after(registry.as_deref().unwrap_or_default(), lease.comment_id) {
                continue;
            }
        }
        let n = lease.issue;
        let outcomes = crate::cli::RESULT_OUTCOMES.join("|");
        let how = if origin == Origin::Spawned {
            format!("`bosun result {n} --outcome <{outcomes}>`")
        } else {
            format!("`bosun result {n} --outcome <{outcomes}>` or `bosun handoff`")
        };
        return Ok(Some(format!(
            "you hold the lease on #{n} but posted no result or handoff. Run {how}, then stop."
        )));
    }
    Ok(None)
}

/// `<git dir>/bosun/gate.json`: per worktree, so parallel worktrees never overwrite each other.
pub fn record_path(dir: &Path) -> Option<PathBuf> {
    let git_dir = git_out(dir, &["rev-parse", "--absolute-git-dir"])?;
    Some(PathBuf::from(git_dir).join("bosun").join("gate.json"))
}

fn git_raw(dir: &Path, args: &[&str]) -> Option<String> {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .output()
        .ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).into_owned())
}

/// HEAD and the fingerprint of the work tree, and whether the tree has changes.
pub fn tree_state(dir: &Path) -> Option<(GateRecord, bool)> {
    let head = git_out(dir, &["rev-parse", "HEAD"])?;
    let status = git_raw(dir, &["status", "--porcelain=v1"])?;
    let diff = git_raw(dir, &["diff", "--no-ext-diff", "HEAD"]).unwrap_or_default();
    let rec = GateRecord {
        head,
        fingerprint: fingerprint(&status, &diff),
    };
    Some((rec, !status.trim().is_empty()))
}

/// Forget the record after a failed gate run, so an earlier pass on this tree no longer counts.
pub fn clear_record(dir: &Path) {
    if let Some(file) = record_path(dir)
        && let Err(e) = std::fs::remove_file(&file)
        && e.kind() != std::io::ErrorKind::NotFound
    {
        eprintln!("bosun: warning: cannot remove the gate record: {e}");
    }
}

/// Store the state of the tree after a passing gate run; a write problem is only logged.
pub fn write_record(dir: &Path) {
    let (Some(file), Some((rec, _))) = (record_path(dir), tree_state(dir)) else {
        return;
    };
    let write = || -> std::io::Result<()> {
        std::fs::create_dir_all(file.parent().unwrap_or(Path::new(".")))?;
        std::fs::write(&file, rec.to_json().to_string())
    };
    if let Err(e) = write() {
        eprintln!("bosun: warning: cannot write the gate record: {e}");
    }
}

fn stale_gate(dir: &Path) -> bool {
    let Some((now, dirty)) = tree_state(dir) else {
        return false;
    };
    let record = record_path(dir)
        .and_then(|f| std::fs::read_to_string(f).ok())
        .and_then(|t| serde_json::from_str::<Value>(&t).ok())
        .and_then(|v| GateRecord::from_json(&v));
    is_stale(record.as_ref(), &now, dirty)
}

/// `<git dir>/bosun/wait/`: one `<pid>.json` marker per running `bosun wait`.
fn wait_dir(dir: &Path) -> Option<PathBuf> {
    Some(record_path(dir)?.with_file_name("wait"))
}

fn epoch_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

/// Marks a running `bosun wait` of this session; the marker goes away on drop.
pub struct WaitMarker(Option<PathBuf>);

impl WaitMarker {
    pub fn start(dir: &Path, session: &str) -> Self {
        let pid = std::process::id();
        let file = wait_dir(dir).map(|d| d.join(format!("{pid}.json")));
        let body = json!({"pid": pid, "session": session, "started": epoch_secs()}).to_string();
        let write = |f: &Path| -> std::io::Result<()> {
            std::fs::create_dir_all(f.parent().unwrap_or(Path::new(".")))?;
            std::fs::write(f, &body)
        };
        if let Some(f) = &file
            && let Err(e) = write(f)
        {
            eprintln!("bosun: warning: cannot write the wait marker: {e}");
        }
        Self(file)
    }
}

impl Drop for WaitMarker {
    fn drop(&mut self) {
        if let Some(f) = &self.0 {
            let _ = std::fs::remove_file(f);
        }
    }
}

/// Does this session have a running `bosun wait`? Markers of dead waits are removed.
fn waiting(dir: &Path, session: &str) -> bool {
    let Some(entries) = wait_dir(dir).and_then(|d| std::fs::read_dir(d).ok()) else {
        return false;
    };
    let now = epoch_secs();
    let has_proc = Path::new("/proc/self").exists();
    let mut live = false;
    for path in entries.flatten().map(|e| e.path()) {
        let Some(v) = std::fs::read_to_string(&path)
            .ok()
            .and_then(|t| serde_json::from_str::<Value>(&t).ok())
        else {
            continue;
        };
        let pid = v["pid"].as_u64().unwrap_or(0);
        let alive = has_proc.then(|| Path::new(&format!("/proc/{pid}")).exists());
        let age = now.saturating_sub(v["started"].as_u64().unwrap_or(0));
        if !wait_marker_live(alive, age, crate::cli::WAIT_TIMEOUT_SECS) {
            let _ = std::fs::remove_file(&path);
        } else if v["session"].as_str() == Some(session) {
            live = true;
        }
    }
    live
}
