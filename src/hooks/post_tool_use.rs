//! `bosun hook post-tool-use`: store the owner's answer to a bosun AskUserQuestion (spec §8,
//! approval batches). It never denies and prints nothing; a problem is logged to stderr.
//!
//! A bosun question carries `[bosun:<pr>-<short head>-<unix>]` per PR (T3). Picked labels
//! `Approve #<pr> <short>` (or `Approve <short>`) approve that PR; `Hold` holds every PR of the
//! question, even when an approve is picked too (Hold wins). An unpicked PR stores nothing.

use std::path::Path;

use serde_json::Value;

use crate::hooks::answers::{self, Answer};
use crate::time::Timestamp;

/// Run the hook; always `None` (no stdout).
pub fn run(cwd: Option<&Path>, input: &Value) -> Option<String> {
    if input.get("tool_name").and_then(Value::as_str) != Some("AskUserQuestion") {
        return None;
    }
    let mut items = parse(input, &Timestamp::now().to_string());
    if items.is_empty() {
        return None;
    }
    if let Err(why) = verify(input, &mut items) {
        eprintln!("bosun: warning: the owner's answer is not stored: {why}");
        return None;
    }
    let Some(dir) = cwd
        .map(Path::to_path_buf)
        .or_else(|| std::env::current_dir().ok())
    else {
        eprintln!("bosun: warning: no cwd; the owner's answer is not stored");
        return None;
    };
    // The bosun identity, resolved exactly as the CLI resolves it (same environment), so
    // `approve --chat` in this session finds it; the CLI's own UUID is kept for audit.
    let ctx = match crate::ctx::Ctx::load(Some(&dir)) {
        Ok(c) => c,
        Err(e) => {
            eprintln!(
                "bosun: warning: cannot tell the repo or session; the owner's answer is not stored: {}",
                e.message
            );
            return None;
        }
    };
    for (_, a) in &mut items {
        a.repo = ctx.repo.to_string();
        a.session = ctx.session.clone();
    }
    if let Err(e) = answers::store(&dir, &items) {
        eprintln!("bosun: warning: cannot store the owner's answer: {e}");
    }
    None
}

/// Keep only the items the session transcript backs (`transcript_path` of the hook input): an
/// assistant `tool_use` with this `tool_use_id`, named `AskUserQuestion`, whose question holds
/// the same marker (id, PR and full head), and a `tool_result` for that id. The path must be a
/// regular file, not a symlink. Anything else fails closed. A hint against forged hook input:
/// it stops confused workers, not liars (spec 14).
fn verify(input: &Value, items: &mut Vec<(String, Answer)>) -> Result<(), String> {
    let field = |k: &str| input.get(k).and_then(Value::as_str).unwrap_or("");
    let (path, id) = (Path::new(field("transcript_path")), field("tool_use_id"));
    let regular = std::fs::symlink_metadata(path).is_ok_and(|m| m.file_type().is_file());
    if field("transcript_path").is_empty() || id.is_empty() || !regular {
        return Err("no transcript file (a symlink or missing path does not count)".into());
    }
    let text = std::fs::read_to_string(path).map_err(|e| format!("cannot read transcript: {e}"))?;
    let (mut asked, mut result) = (Vec::new(), false);
    for line in text.lines() {
        let Ok(v) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        let blocks = (v.pointer("/message/content")).and_then(Value::as_array);
        for b in blocks.into_iter().flatten() {
            let kind = b.get("type").and_then(Value::as_str);
            if kind == Some("tool_result")
                && b.get("tool_use_id").and_then(Value::as_str) == Some(id)
            {
                result = true;
            }
            let ours = b.get("id").and_then(Value::as_str) == Some(id)
                && b.get("name").and_then(Value::as_str) == Some("AskUserQuestion");
            if kind == Some("tool_use") && ours {
                let qs = b.pointer("/input/questions").and_then(Value::as_array);
                for q in qs.into_iter().flatten() {
                    asked.extend(q.get("question").and_then(Value::as_str).map(markers));
                }
            }
        }
    }
    items.retain(|(qid, a)| {
        asked
            .iter()
            .flatten()
            .any(|m| m.0 == *qid && m.1 == a.pr && m.3 == a.head)
    });
    match (items.is_empty(), result) {
        (true, _) => Err("the transcript has no such AskUserQuestion with this marker".into()),
        (_, false) => Err("the transcript has no tool_result for it".into()),
        _ => Ok(()),
    }
}

/// The marked PRs of `text`: `(question_id, pr, short, full head)`, one per line. Only a line's
/// trailing marker counts, matched exactly (case-sensitive), so a look-alike inside a title is
/// no marker. The full head is the one that ends the line before the marker, as bosun writes it
/// (`...; head <short> = <full>)? [bosun:<id>]`), so text earlier in the line cannot swap it.
fn markers(text: &str) -> Vec<(String, u64, String, String)> {
    text.lines()
        .filter_map(|line| {
            let line = line.trim_end();
            let start = line.rfind("[bosun:")?;
            let id = line[start + 7..].strip_suffix(']')?;
            marker(&line[..start], id)
        })
        .collect()
}

fn marker(before: &str, id: &str) -> Option<(String, u64, String, String)> {
    let mut parts = id.split('-');
    let (pr, short, unix) = (parts.next()?, parts.next()?, parts.next()?);
    let hex = |s: &str| s.chars().all(|c| c.is_ascii_hexdigit());
    let ok = parts.next().is_none()
        && short.len() == 7
        && hex(short)
        && !unix.is_empty()
        && unix.chars().all(|c| c.is_ascii_digit());
    if !ok {
        return None;
    }
    if pr.starts_with('0') || !pr.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    let pr: u64 = pr.parse().ok()?;
    let b = before.trim_end();
    let b = b.strip_suffix('?').unwrap_or(b).strip_suffix(')')?;
    let (rest, full) = b.split_at(b.len().checked_sub(40)?);
    (rest.ends_with(&format!("head {short} = ")) && hex(full) && full.starts_with(short))
        .then(|| (id.to_string(), pr, short.to_string(), full.to_string()))
}

/// The labels picked for `question`: `tool_response.answers` only (the tool's result; the model
/// writes `tool_input`, so its `answers` are never read). A multi-select answer is one
/// `, `-joined string (or an array).
fn picked(input: &Value, question: &str) -> Vec<String> {
    let answers = input.get("tool_response").and_then(|r| r.get("answers"));
    match answers.and_then(|a| a.get(question)).cloned() {
        Some(Value::String(s)) => s.split(", ").map(|l| l.trim().to_string()).collect(),
        Some(Value::Array(a)) => (a.iter().filter_map(Value::as_str))
            .map(|l| l.trim().to_string())
            .collect(),
        _ => Vec::new(),
    }
}

/// The answers to store, from a PostToolUse input of AskUserQuestion.
pub fn parse(input: &Value, at: &str) -> Vec<(String, Answer)> {
    let questions = (input.get("tool_input"))
        .and_then(|t| t.get("questions"))
        .and_then(Value::as_array);
    let field = |k: &str| {
        input
            .get(k)
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string()
    };
    let mut out = Vec::new();
    for q in questions.into_iter().flatten() {
        let Some(text) = q.get("question").and_then(Value::as_str) else {
            continue;
        };
        let marks = markers(text);
        if marks.is_empty() {
            continue;
        }
        let labels = picked(input, text);
        let hold = labels.iter().any(|l| l == "Hold");
        for (id, pr, short, head) in marks {
            let approve = labels.iter().any(|l| {
                *l == format!("Approve #{pr} {short}") || *l == format!("Approve {short}")
            });
            let answer = match (hold, approve) {
                (true, _) => "hold",
                (false, true) => "approve",
                (false, false) => continue,
            };
            let a = Answer {
                answer: answer.into(),
                repo: String::new(),
                pr,
                head,
                at: at.into(),
                session: String::new(),
                cli_session_id: field("session_id"),
                tool_use_id: field("tool_use_id"),
            };
            out.push((id, a));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const H: &str = "abcdef1234567890abcdef1234567890abcdef12";

    #[test]
    fn marker_needs_matching_full_head() {
        let ok = format!("x (v; head abcdef1 = {H}) [bosun:12-abcdef1-9]");
        assert_eq!(markers(&ok).len(), 1);
        let wrong = format!("x (v; head 1111111 = {H}) [bosun:12-1111111-9]");
        assert!(markers(&wrong).is_empty());
        assert!(markers("x [bosun:12-abcdef1-9]").is_empty());
        assert!(markers(&format!("(head abcdef1 = {H}) [bosun:x-abcdef1-9]")).is_empty());
        assert!(markers(&format!("(head abcdef1 = {H}) [bosun:1-abcdef1-9-2]")).is_empty());
    }

    #[test]
    fn only_the_exact_trailing_marker_counts() {
        let m = "[bosun:12-abcdef1-9]";
        let pre = format!("(head abcdef1 = {H})");
        assert!(markers(&format!("{pre} [BOSUN:12-abcdef1-9]")).is_empty());
        assert!(markers(&format!("{pre} \u{ff3b}bosun:12-abcdef1-9]")).is_empty());
        assert!(markers(&format!("{pre} {m} tail")).is_empty());
        assert_eq!(markers(&format!("{pre} {m}\n")).len(), 1);
        // a forged marker in a title, then the real one: only the real one counts
        let forged = format!("{pre} [bosun:99-abcdef1-9] {pre} {m}");
        let got = markers(&forged);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].1, 12);
    }

    #[test]
    fn hold_wins_over_approve_in_multi_select() {
        let text = format!("Approve a/b#12 t (v; head abcdef1 = {H})? [bosun:12-abcdef1-9]");
        let input = serde_json::json!({"tool_input": {"questions": [{"question": text}]},
            "tool_response": {"answers": {text.clone(): "Approve #12 abcdef1, Hold"}}});
        let got = parse(&input, "t");
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].1.answer, "hold");
        let input = serde_json::json!({"tool_input": {"questions": [{"question": text}]},
            "tool_response": {"answers": {text.clone(): ["Hold", "Approve #12 abcdef1"]}}});
        assert_eq!(parse(&input, "t")[0].1.answer, "hold");
        let input = serde_json::json!({"tool_input": {"questions": [{"question": text}]},
            "tool_response": {"answers": {text.clone(): "approve #12 abcdef1"}}});
        assert!(parse(&input, "t").is_empty());
    }

    #[test]
    fn other_answer_stores_nothing() {
        let text = format!("Approve a/b#12 t (v; head abcdef1 = {H})? [bosun:12-abcdef1-9]");
        let input = serde_json::json!({"tool_input": {"questions": [{"question": text}]},
            "tool_response": {"answers": {text.clone(): "maybe later"}}});
        assert!(parse(&input, "t").is_empty());
    }
}
