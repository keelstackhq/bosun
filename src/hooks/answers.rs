//! The owner's AskUserQuestion answers: `<git common dir>/bosun/answers.json`, a map
//! `question_id -> {answer, repo, pr, head, at, session, cli_session_id, tool_use_id}` beside the lease cache (spec §8, approval batches).
//! Written by `bosun hook post-tool-use`; read by `approve --list` (holds) and `approve --chat`.

use std::path::{Path, PathBuf};

use serde_json::{Map, Value, json};

use crate::hooks::git_out;
use crate::time::{Timestamp, parse_rfc3339};

const MAX_AGE_SECS: i64 = 14 * 86_400;
/// How far (seconds) an answer's `at` may lie after now: a later one never counts, so a
/// future-dated entry cannot outrank every later answer, and it is pruned on the next write.
const FUTURE_SKEW_SECS: i64 = 60;

/// Whether `at` is not later than now plus [`FUTURE_SKEW_SECS`].
fn not_future(at: &str, now: Timestamp) -> bool {
    when(at) <= now.0 + FUTURE_SKEW_SECS
}

/// One stored answer: `answer` is `approve` or `hold`; `session` is the bosun identity the hook resolves like the
/// CLI does (`BOSUN_SESSION`, else the remote session id; T5 needs the same session as the
/// orchestrator running `approve --chat`); `cli_session_id` and `tool_use_id` come from the hook
/// input (audit only: the CLI's own UUID never equals the bosun identity).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Answer {
    pub answer: String,
    /// `owner/name` of the repo the question was about; an answer for another repo never counts.
    pub repo: String,
    pub pr: u64,
    pub head: String,
    pub at: String,
    pub session: String,
    pub cli_session_id: String,
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
        session: text("session").unwrap_or_default(),
        cli_session_id: text("cli_session_id").unwrap_or_default(),
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

/// The ask `approve --list` issued as a Claude Tag post for `question_id` (stored with answer
/// `asked`), for `repo`. It is no answer: [`current`] and [`held`] ignore it.
pub fn asked(dir: &Path, repo: &str, question_id: &str) -> Option<Answer> {
    get(dir, repo, question_id).filter(|a| a.answer == "asked")
}

/// Record the Claude Tag asks `(question_id, pr, head)` that `approve --list` posts, so the
/// Slack `--chat` path can require an ask that was actually issued.
pub fn record_asks(
    dir: &Path,
    repo: &str,
    session: &str,
    at: &str,
    asks: &[(String, u64, String)],
) {
    let items: Vec<(String, Answer)> = asks
        .iter()
        .map(|(id, pr, head)| {
            let a = Answer {
                answer: "asked".into(),
                repo: repo.into(),
                pr: *pr,
                head: head.clone(),
                at: at.into(),
                session: session.into(),
                cli_session_id: "approve-list".into(),
                tool_use_id: String::new(),
            };
            (id.clone(), a)
        })
        .collect();
    if let Err(e) = store(dir, &items) {
        eprintln!("bosun: warning: cannot record the asks: {e}");
    }
}

/// The newest answer to `pr` of `repo` at exactly `head` (by `at`; a hold wins a tie). A new
/// head is asked again, so only the exact head counts. An answer dated after now (plus a small
/// skew) is ignored.
pub fn current(dir: &Path, repo: &str, pr: u64, head: &str) -> Option<Answer> {
    let map = read(&path(dir)?)?;
    let now = Timestamp::now();
    map.values()
        .filter_map(answer_of)
        .filter(|a| {
            a.repo == repo
                && a.pr == pr
                && a.head == head
                && a.answer != "asked"
                && not_future(&a.at, now)
        })
        .max_by_key(|a| (when(&a.at), a.answer == "hold"))
}

/// Whether the owner's newest answer for `pr` at exactly `head` is a hold.
pub fn held(dir: &Path, repo: &str, pr: u64, head: &str) -> bool {
    current(dir, repo, pr, head).is_some_and(|a| a.answer == "hold")
}

/// Store `(question_id, answer)` pairs, replacing earlier answers to the same ids. Entries older
/// than 14 days, dated after now (plus the skew), or for another head of a stored PR are dropped;
/// an answer dated after now is refused. Atomic (tmp + rename).
/// The file is a local hint, never proof (spec 14): a guard stops confused workers, not liars.
/// An answer with no `session`, `cli_session_id` or `repo` is refused and nothing is stored; so is a corrupt
/// file (it is not reset).
pub fn store(dir: &Path, items: &[(String, Answer)]) -> std::io::Result<()> {
    use std::io::{Error, ErrorKind};
    let Some(file) = path(dir) else {
        return Ok(());
    };
    if items
        .iter()
        .any(|(_, a)| a.session.is_empty() || a.cli_session_id.is_empty() || a.repo.is_empty())
    {
        return Err(Error::new(
            ErrorKind::InvalidInput,
            "an answer needs the bosun session, the hook's session_id and the repo",
        ));
    }
    let now = Timestamp::now();
    if items.iter().any(|(_, a)| !not_future(&a.at, now)) {
        return Err(Error::new(
            ErrorKind::InvalidInput,
            "an answer is dated after now",
        ));
    }
    let Some(mut map) = read(&file) else {
        return Err(Error::new(
            ErrorKind::InvalidData,
            "answers.json is corrupt",
        ));
    };
    map.retain(|_, v| {
        let Some(a) = answer_of(v) else { return false };
        when(&a.at) >= now.0 - MAX_AGE_SECS
            && not_future(&a.at, now)
            && !items
                .iter()
                .any(|(_, n)| n.repo == a.repo && n.pr == a.pr && n.head != a.head)
    });
    for (id, a) in items {
        let v = json!({"answer": a.answer, "repo": a.repo, "pr": a.pr, "head": a.head, "at": a.at,
            "session": a.session, "cli_session_id": a.cli_session_id, "tool_use_id": a.tool_use_id});
        map.insert(id.clone(), v);
    }
    if let Some(parent) = file.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let tmp = file.with_extension(format!("json.tmp{}", std::process::id()));
    std::fs::write(&tmp, Value::Object(map).to_string())?;
    std::fs::rename(&tmp, &file)
}
