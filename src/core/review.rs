//! What a reviewer needs besides the diff (spec §7): the lessons that apply to the changed files
//! and the use cases of the task's epic.

use serde::{Deserialize, Serialize};

use super::glob::Glob;

/// A lesson of `docs/lessons.toml` (`[[lesson]]`). Only `id` and `rule` are required.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Lesson {
    pub id: String,
    pub rule: String,
    #[serde(default)]
    pub why: String,
    /// Globs of the files the lesson applies to; none means global.
    #[serde(default)]
    pub scope: Vec<String>,
    #[serde(default)]
    pub evidence: Vec<String>,
    #[serde(default)]
    pub added: String,
    #[serde(default)]
    pub last_hit: String,
    #[serde(default)]
    pub hits: u64,
}

#[derive(Deserialize)]
struct LessonFile {
    #[serde(default)]
    lesson: Vec<Lesson>,
}

/// The lessons of a `lessons.toml` text; no text means no lessons.
pub fn parse_lessons(text: Option<&str>) -> Result<Vec<Lesson>, String> {
    match text {
        None => Ok(vec![]),
        Some(t) => toml::from_str::<LessonFile>(t)
            .map(|f| f.lesson)
            .map_err(|e| format!("docs/lessons.toml is not valid: {e}")),
    }
}

/// Global lessons and those whose scope matches any of `files`.
pub fn lessons_for(lessons: Vec<Lesson>, files: &[String]) -> Vec<Lesson> {
    lessons
        .into_iter()
        .filter(|l| {
            l.scope.is_empty()
                || l.scope
                    .iter()
                    .filter_map(|g| Glob::parse(g).ok())
                    .any(|g| files.iter().any(|f| g.matches(f)))
        })
        .collect()
}

/// The `- UC-…` lines of an epic body.
pub fn use_cases(body: &str) -> Vec<String> {
    body.lines()
        .map(str::trim)
        .filter(|l| l.starts_with("- UC-"))
        .map(str::to_string)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const TOML: &str = r#"
[[lesson]]
id = "L-001"
rule = "global rule"

[[lesson]]
id = "L-002"
rule = "core rule"
scope = ["src/core/**"]
hits = 2

[[lesson]]
id = "L-003"
rule = "web rule"
scope = ["web/**"]
"#;

    #[test]
    fn scope_and_global() {
        let all = parse_lessons(Some(TOML)).unwrap();
        assert_eq!(all[1].hits, 2);
        let got = lessons_for(all, &["src/core/x.rs".to_string()]);
        let ids: Vec<&str> = got.iter().map(|l| l.id.as_str()).collect();
        assert_eq!(ids, ["L-001", "L-002"]);
    }

    #[test]
    fn missing_or_broken_file() {
        assert!(parse_lessons(None).unwrap().is_empty());
        assert!(parse_lessons(Some("[[lesson]]\nid = 1")).is_err());
    }

    #[test]
    fn use_case_lines() {
        let body = "Epic\n- UC-M1-1: a\n  - UC-M1-2: b\n* not\n";
        assert_eq!(use_cases(body), ["- UC-M1-1: a", "- UC-M1-2: b"]);
    }
}
