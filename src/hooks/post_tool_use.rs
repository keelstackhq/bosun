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
    let Some(dir) = cwd
        .map(Path::to_path_buf)
        .or_else(|| std::env::current_dir().ok())
    else {
        eprintln!("bosun: warning: no cwd; the owner's answer is not stored");
        return None;
    };
    let Some(repo) = crate::ctx::repo_at(&dir) else {
        eprintln!("bosun: warning: cannot tell the repo; the owner's answer is not stored");
        return None;
    };
    for (_, a) in &mut items {
        a.repo = repo.to_string();
    }
    if let Err(e) = answers::store(&dir, &items) {
        eprintln!("bosun: warning: cannot store the owner's answer: {e}");
    }
    None
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
                session_id: field("session_id"),
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
