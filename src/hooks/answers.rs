//! The owner's AskUserQuestion answers: `<git common dir>/bosun/answers.json`, a map
//! `question_id -> {answer, repo, pr, head, at, session_id, tool_use_id}` beside the lease cache (spec §8, approval batches).
//! Written by `bosun hook post-tool-use`; read by `approve --list` (holds) and `approve --chat`.

use std::path::{Path, PathBuf};

use serde_json::{Map, Value, json};

use crate::hooks::git_out;
use crate::time::parse_rfc3339;

const MAX_AGE_SECS: i64 = 14 * 86_400;

/// One stored answer: `answer` is `approve` or `hold`; `session_id` and `tool_use_id` come from
/// the hook input (T5 needs the same session as the orchestrator running `approve --chat`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Answer {
    pub answer: String,
    /// `owner/name` of the repo the question was about; an answer for another repo never counts.
    pub repo: String,
    pub pr: u64,
    pub head: String,
    pub at: String,
    pub session_id: String,
    pub tool_use_id: String,
}

/// `<git common dir>/bosun/answers.json`; `None` outside a git repo.
pub fn path(dir: &Path) -> Option<PathBuf> {
    let common = git_out(dir, &["rev-parse", "--git-common-dir"])?;
    Some(dir.join(common).join("bosun").join("answers.json"))
}

/// The stored map. A missing file is an empty map; a file that is not a JSON object is corrupt:
/// warn on stderr and return `None`, so approvals fail closed and the file is never reset.
fn read(file: &Path) -> Option<Map<String, Value>> {
    let Ok(raw) = std::fs::read_to_string(file) else {
        return Some(Map::new());
    };
    if raw.trim().is_empty() {
        return Some(Map::new());
    }
    let map = serde_json::from_str::<Value>(&raw)
        .ok()
        .and_then(|v| v.as_object().cloned());
    if map.is_none() {
        eprintln!(
            "bosun: warning: {} is corrupt and is not used; delete it to start over",
            file.display()
        );
    }
    map
}

fn answer_of(v: &Value) -> Option<Answer> {
    let text = |k: &str| v.get(k).and_then(Value::as_str).map(str::to_string);
    Some(Answer {
        answer: text("answer")?,
        repo: text("repo").filter(|r| !r.is_empty())?,
        pr: v.get("pr")?.as_u64()?,
        head: text("head")?,
        at: text("at")?,
        session_id: text("session_id").unwrap_or_default(),
        tool_use_id: text("tool_use_id").unwrap_or_default(),
    })
}

fn when(at: &str) -> i64 {
    parse_rfc3339(at).map_or(i64::MIN, |t| t.0)
}

/// The stored answer to `question_id` for `repo`, if any.
pub fn get(dir: &Path, repo: &str, question_id: &str) -> Option<Answer> {
    let a = answer_of(read(&path(dir)?)?.get(question_id)?)?;
    (a.repo == repo).then_some(a)
}

/// The newest answer to `pr` of `repo` at exactly `head` (by `at`; a hold wins a tie). A new
/// head is asked again, so only the exact head counts.
pub fn current(dir: &Path, repo: &str, pr: u64, head: &str) -> Option<Answer> {
    let map = read(&path(dir)?)?;
    map.values()
        .filter_map(answer_of)
        .filter(|a| a.repo == repo && a.pr == pr && a.head == head)
        .max_by_key(|a| (when(&a.at), a.answer == "hold"))
}

/// Whether the owner's newest answer for `pr` at exactly `head` is a hold.
pub fn held(dir: &Path, repo: &str, pr: u64, head: &str) -> bool {
    current(dir, repo, pr, head).is_some_and(|a| a.answer == "hold")
}

/// Store `(question_id, answer)` pairs, replacing earlier answers to the same ids. Entries older
/// than 14 days and entries for another head of a stored PR are dropped. Atomic (tmp + rename).
/// The file is a local hint, never proof (spec 14): a guard stops confused workers, not liars.
/// An answer with no `session_id` or no `repo` is refused and nothing is stored; so is a corrupt
/// file (it is not reset).
pub fn store(dir: &Path, items: &[(String, Answer)]) -> std::io::Result<()> {
    use std::io::{Error, ErrorKind};
    let Some(file) = path(dir) else {
        return Ok(());
    };
    if items
        .iter()
        .any(|(_, a)| a.session_id.is_empty() || a.repo.is_empty())
    {
        return Err(Error::new(
            ErrorKind::InvalidInput,
            "an answer needs the hook's session_id and the repo",
        ));
    }
    let Some(mut map) = read(&file) else {
        return Err(Error::new(
            ErrorKind::InvalidData,
            "answers.json is corrupt",
        ));
    };
    let newest = items.iter().map(|(_, a)| when(&a.at)).max().unwrap_or(0);
    map.retain(|_, v| {
        let Some(a) = answer_of(v) else { return false };
        when(&a.at) >= newest - MAX_AGE_SECS
            && !items
                .iter()
                .any(|(_, n)| n.repo == a.repo && n.pr == a.pr && n.head != a.head)
    });
    for (id, a) in items {
        let v = json!({"answer": a.answer, "repo": a.repo, "pr": a.pr, "head": a.head, "at": a.at,
            "session_id": a.session_id, "tool_use_id": a.tool_use_id});
        map.insert(id.clone(), v);
    }
    if let Some(parent) = file.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let tmp = file.with_extension(format!("json.tmp{}", std::process::id()));
    std::fs::write(&tmp, Value::Object(map).to_string())?;
    std::fs::rename(&tmp, &file)
}
