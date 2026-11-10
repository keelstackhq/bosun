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
    let items = parse(input, &Timestamp::now().to_string());
    if items.is_empty() {
        return None;
    }
    let Some(dir) = cwd
        .map(Path::to_path_buf)
        .or_else(|| std::env::current_dir().ok())
    else {
        eprintln!("bosun: warning: no cwd; the owner's answer is not stored");
        return None;
    };
    if let Err(e) = answers::store(&dir, &items) {
        eprintln!("bosun: warning: cannot store the owner's answer: {e}");
    }
    None
}

/// The marked PRs of `text`: `(question_id, pr, short, full head)`. The full head is read from
/// `head <short> = <full>` on the marker's line, the last such before the marker.
fn markers(text: &str) -> Vec<(String, u64, String, String)> {
    let mut out = Vec::new();
    for line in text.lines() {
        let mut rest = line;
        let mut offset = 0;
        while let Some(i) = rest.find("[bosun:") {
            let start = offset + i;
            let after = &line[start + 7..];
            let Some(end) = after.find(']') else { break };
            let id = &after[..end];
            if let Some(m) = marker(&line[..start], id) {
                out.push(m);
            }
            offset = start + 7 + end;
            rest = &line[offset..];
        }
    }
    out
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
    let pr: u64 = pr.parse().ok()?;
    let key = format!("head {short} = ");
    let at = before.rfind(&key)? + key.len();
    let full = before.get(at..at + 40)?;
    (hex(full) && full.starts_with(short))
        .then(|| (id.to_string(), pr, short.to_string(), full.to_string()))
}

/// The labels picked for `question`: `tool_response.answers`, else `tool_input.answers`. A
/// multi-select answer is one `, `-joined string (or an array).
fn picked(input: &Value, question: &str) -> Vec<String> {
    let from = |root: &str| input.get(root)?.get("answers")?.get(question).cloned();
    match from("tool_response").or_else(|| from("tool_input")) {
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
                pr,
                head,
                at: at.into(),
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
        let ok = format!("x (head abcdef1 = {H}; v) [bosun:12-abcdef1-9]");
        assert_eq!(markers(&ok).len(), 1);
        let wrong = format!("x (head 1111111 = {H}; v) [bosun:12-1111111-9]");
        assert!(markers(&wrong).is_empty());
        assert!(markers("x [bosun:12-abcdef1-9]").is_empty());
        assert!(markers(&format!("head abcdef1 = {H} [bosun:x-abcdef1-9]")).is_empty());
        assert!(markers(&format!("head abcdef1 = {H} [bosun:1-abcdef1-9-2]")).is_empty());
    }

    #[test]
    fn other_answer_stores_nothing() {
        let text = format!("Approve a/b#12 t (head abcdef1 = {H}; v)? [bosun:12-abcdef1-9]");
        let input = serde_json::json!({"tool_input": {"questions": [{"question": text}]},
            "tool_response": {"answers": {text.clone(): "maybe later"}}});
        assert!(parse(&input, "t").is_empty());
    }
}
