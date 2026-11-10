//! The owner's AskUserQuestion answers: `<git common dir>/bosun/answers.json`, a map
//! `question_id -> {answer, pr, head, at}` beside the lease cache (spec §8, approval batches).
//! Written by `bosun hook post-tool-use`; read by `approve --list` (holds) and `approve --chat`.

use std::path::{Path, PathBuf};

use serde_json::{Map, Value, json};

use crate::hooks::git_out;
use crate::time::parse_rfc3339;

/// One stored answer: `answer` is `approve` or `hold`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Answer {
    pub answer: String,
    pub pr: u64,
    pub head: String,
    pub at: String,
}

/// `<git common dir>/bosun/answers.json`; `None` outside a git repo.
pub fn path(dir: &Path) -> Option<PathBuf> {
    let common = git_out(dir, &["rev-parse", "--git-common-dir"])?;
    Some(dir.join(common).join("bosun").join("answers.json"))
}

fn read(file: &Path) -> Map<String, Value> {
    std::fs::read_to_string(file)
        .ok()
        .and_then(|raw| serde_json::from_str::<Value>(&raw).ok())
        .and_then(|v| v.as_object().cloned())
        .unwrap_or_default()
}

fn answer_of(v: &Value) -> Option<Answer> {
    Some(Answer {
        answer: v.get("answer")?.as_str()?.into(),
        pr: v.get("pr")?.as_u64()?,
        head: v.get("head")?.as_str()?.into(),
        at: v.get("at")?.as_str()?.into(),
    })
}

/// The stored answer to `question_id`, if any.
pub fn get(dir: &Path, question_id: &str) -> Option<Answer> {
    answer_of(read(&path(dir)?).get(question_id)?)
}

/// Whether the owner held `pr` at exactly `head` (a new head is asked again).
pub fn held(dir: &Path, pr: u64, head: &str) -> bool {
    path(dir).is_some_and(|f| {
        read(&f)
            .values()
            .filter_map(answer_of)
            .any(|a| a.answer == "hold" && a.pr == pr && a.head == head)
    })
}

/// Answers older than this before the newest stored one are dropped (their PRs moved on).
const KEEP_SECS: i64 = 30 * 24 * 3600;

/// Store `(question_id, answer)` pairs, replacing earlier answers to the same ids and pruning
/// answers [`KEEP_SECS`] older than the newest. A lock file serialises concurrent hooks; the
/// write is atomic (temp file + rename).
pub fn store(dir: &Path, items: &[(String, Answer)]) -> std::io::Result<()> {
    let Some(file) = path(dir) else {
        return Ok(());
    };
    if let Some(parent) = file.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let _lock = Lock::take(&file.with_extension("json.lock"))?;
    let mut map = read(&file);
    for (id, a) in items {
        let v = json!({"answer": a.answer, "pr": a.pr, "head": a.head, "at": a.at});
        map.insert(id.clone(), v);
    }
    let at = |v: &Value| v.get("at").and_then(Value::as_str).and_then(parse_rfc3339);
    if let Some(newest) = map.values().filter_map(at).max() {
        map.retain(|_, v| at(v).is_some_and(|t| t.0 >= newest.0 - KEEP_SECS));
    }
    let tmp = file.with_extension(format!("json.tmp{}", std::process::id()));
    std::fs::write(&tmp, Value::Object(map).to_string())?;
    std::fs::rename(&tmp, &file)
}

/// An exclusive lock file, removed on drop. A lock older than 10 s is stale (a killed hook).
struct Lock(PathBuf);

impl Lock {
    fn take(path: &Path) -> std::io::Result<Lock> {
        for _ in 0..200 {
            match std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(path)
            {
                Ok(_) => return Ok(Lock(path.to_path_buf())),
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                    let stale = std::fs::metadata(path)
                        .and_then(|m| m.modified())
                        .is_ok_and(|t| t.elapsed().is_ok_and(|d| d.as_secs() >= 10));
                    if stale {
                        let _ = std::fs::remove_file(path);
                    } else {
                        std::thread::sleep(std::time::Duration::from_millis(25));
                    }
                }
                Err(e) => return Err(e),
            }
        }
        Err(std::io::Error::other("answers.json.lock is held"))
    }
}

impl Drop for Lock {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn a(pr: u64, at: &str) -> Answer {
        Answer {
            answer: "approve".into(),
            pr,
            head: "h".into(),
            at: at.into(),
        }
    }

    #[test]
    fn store_prunes_old_and_keeps_concurrent() {
        let tmp = tempfile::tempdir().unwrap();
        let ok = std::process::Command::new("git")
            .args(["init", "-q"])
            .current_dir(tmp.path())
            .status()
            .unwrap();
        assert!(ok.success());
        let d = tmp.path();
        store(d, &[("old".into(), a(1, "2026-09-01T00:00:00Z"))]).unwrap();
        let threads: Vec<_> = (0..8)
            .map(|i| {
                let d = d.to_path_buf();
                std::thread::spawn(move || {
                    store(&d, &[(format!("q{i}"), a(i, "2026-11-10T00:00:00Z"))]).unwrap()
                })
            })
            .collect();
        threads.into_iter().for_each(|t| t.join().unwrap());
        assert!(get(d, "old").is_none());
        assert!((0..8).all(|i| get(d, &format!("q{i}")).is_some()));
    }
}
