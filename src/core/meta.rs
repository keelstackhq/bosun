//! Task metadata: one HTML comment in the issue body, `<!-- bosun:task {json} -->`, and the
//! `bosun:area/<a>` label. Missing keys take their defaults; bad JSON is no metadata.

use serde::{Deserialize, Serialize};

/// Opens the task metadata comment; the space keeps `<!-- bosun:task-pr N -->` out.
pub const OPEN: &str = "<!-- bosun:task ";

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

/// The task metadata of an issue body, if it has a valid marker. The LAST marker counts, so text
/// quoted earlier in a body cannot override the real one.
pub fn parse_task_meta(body: &str) -> Option<TaskMeta> {
    let rest = &body[body.rfind(OPEN)? + OPEN.len()..];
    let json = &rest[..rest.find("-->")?];
    serde_json::from_str(json.trim()).ok()
}

/// Every issue a PR body links, in order of appearance and without repeats: `Closes|Fixes|Resolves #N`
/// (any case, also the past tenses) and the `<!-- bosun:task-pr N -->` marker.
pub fn linked_issues(body: &str) -> Vec<u64> {
    static RE: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
        regex::Regex::new(
            r"(?i)\b(?:close[sd]?|fix(?:e[sd])?|resolve[sd]?):?\s+#(\d+)|<!--\s*bosun:task-pr\s+(\d+)\s*-->",
        )
        .expect("static regex")
    });
    let mut out: Vec<u64> = Vec::new();
    for c in RE.captures_iter(body) {
        let n = c
            .get(1)
            .or_else(|| c.get(2))
            .and_then(|m| m.as_str().parse().ok());
        if let Some(n) = n
            && !out.contains(&n)
        {
            out.push(n);
        }
    }
    out
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
    fn the_last_marker_wins_and_task_pr_is_not_one() {
        let body = r#"<!-- bosun:task {"kind":"fake"} -->
x <!-- bosun:task {"kind":"real"} -->
<!-- bosun:task-pr 9 -->"#;
        assert_eq!(parse_task_meta(body).unwrap().kind, "real");
    }

    #[test]
    fn no_marker_or_bad_json_is_none() {
        assert!(parse_task_meta("plain").is_none());
        assert!(parse_task_meta("<!-- bosun:task {oops -->").is_none());
        assert!(parse_task_meta("<!-- bosun:task {} ").is_none());
        assert!(parse_task_meta(r#"<!-- bosun:epic {"item":"M1"} -->"#).is_none());
    }

    #[test]
    fn linked_issues_lists_every_link_once() {
        let body = "Closes #5, and fixes #6.\nRESOLVES #7\nCloses #5\n<!-- bosun:task-pr 9 -->";
        assert_eq!(linked_issues(body), vec![5, 6, 7, 9]);
        assert_eq!(linked_issues("<!--bosun:task-pr 3-->"), vec![3]);
        assert!(linked_issues("prefixes #1, closes # 2, no link").is_empty());
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
