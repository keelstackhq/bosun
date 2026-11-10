//! The owner's AskUserQuestion answers: `<git common dir>/bosun/answers.json`, a map
//! `question_id -> {answer, pr, head, at}` beside the lease cache (spec §8, approval batches).
//! Written by `bosun hook post-tool-use`; read by `approve --list` (holds) and `approve --chat`.

use std::path::{Path, PathBuf};

use serde_json::{Map, Value, json};

use crate::hooks::git_out;

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

/// Store `(question_id, answer)` pairs, replacing earlier answers to the same ids. Atomic.
pub fn store(dir: &Path, items: &[(String, Answer)]) -> std::io::Result<()> {
    let Some(file) = path(dir) else {
        return Ok(());
    };
    let mut map = read(&file);
    for (id, a) in items {
        let v = json!({"answer": a.answer, "pr": a.pr, "head": a.head, "at": a.at});
        map.insert(id.clone(), v);
    }
    if let Some(parent) = file.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let tmp = file.with_extension(format!("json.tmp{}", std::process::id()));
    std::fs::write(&tmp, Value::Object(map).to_string())?;
    std::fs::rename(&tmp, &file)
}
