//! Task metadata: one HTML comment in the issue body, `<!-- bosun:task {json} -->`, and the
//! `bosun:area/<a>` label. Missing keys take their defaults; bad JSON is no metadata.

use serde::{Deserialize, Serialize};

const OPEN: &str = "<!-- bosun:task";

fn feature() -> String {
    "feature".into()
}

fn normal() -> String {
    "normal".into()
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TaskMeta {
    #[serde(default)]
    pub item: String,
    #[serde(default)]
    pub task: String,
    #[serde(default)]
    pub epic: Option<u64>,
    #[serde(default)]
    pub touch: Vec<String>,
    #[serde(default)]
    pub depends: Vec<String>,
    #[serde(default = "feature")]
    pub kind: String,
    #[serde(default = "normal")]
    pub risk: String,
    #[serde(default)]
    pub allow: Vec<String>,
    /// Capabilities the task may use, for example `prod-write` (spec section 8).
    #[serde(default)]
    pub capabilities: Vec<String>,
}

/// The task metadata of an issue body, if it has a valid marker.
pub fn parse_task_meta(body: &str) -> Option<TaskMeta> {
    let rest = &body[body.find(OPEN)? + OPEN.len()..];
    let json = &rest[..rest.find("-->")?];
    serde_json::from_str(json.trim()).ok()
}

/// The area named by a `bosun:area/<a>` label.
pub fn area_of<S: AsRef<str>>(labels: &[S]) -> Option<String> {
    labels
        .iter()
        .find_map(|l| l.as_ref().strip_prefix("bosun:area/"))
        .map(str::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn full_marker_parses() {
        let body = r#"Intro text.

<!-- bosun:task {"item":"M1","task":"T1","epic":12,"touch":["src/core/**"],"depends":["M1/T2"],"kind":"mainfix","risk":"high","allow":["Cargo.lock"]} -->
more"#;
        let m = parse_task_meta(body).unwrap();
        assert_eq!(
            (m.item.as_str(), m.task.as_str(), m.epic),
            ("M1", "T1", Some(12))
        );
        assert_eq!(m.touch, vec!["src/core/**"]);
        assert_eq!(m.depends, vec!["M1/T2"]);
        assert_eq!((m.kind.as_str(), m.risk.as_str()), ("mainfix", "high"));
        assert_eq!(m.allow, vec!["Cargo.lock"]);
    }

    #[test]
    fn defaults_fill_missing_keys() {
        let m = parse_task_meta("<!-- bosun:task {} -->").unwrap();
        assert_eq!((m.kind.as_str(), m.risk.as_str()), ("feature", "normal"));
        assert!(m.touch.is_empty() && m.epic.is_none());
    }

    #[test]
    fn no_marker_or_bad_json_is_none() {
        assert!(parse_task_meta("plain").is_none());
        assert!(parse_task_meta("<!-- bosun:task {oops -->").is_none());
        assert!(parse_task_meta("<!-- bosun:task {} ").is_none());
        assert!(parse_task_meta(r#"<!-- bosun:epic {"item":"M1"} -->"#).is_none());
    }

    #[test]
    fn area_label() {
        assert_eq!(
            area_of(&["bug", "bosun:area/lifecycle"]).as_deref(),
            Some("lifecycle")
        );
        assert_eq!(area_of(&["bug"]), None);
    }
}
