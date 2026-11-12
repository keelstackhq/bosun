//! Task metadata: one HTML comment in the issue body, `<!-- bosun:task {json} -->`, and the
//! `bosun:area/<a>` label. Missing keys take their defaults; bad JSON is no metadata.

use serde::{Deserialize, Serialize};

/// Opens the task metadata comment; the space keeps `<!-- bosun:task-pr N -->` out.
pub const OPEN: &str = "<!-- bosun:task ";

/// Test-integrity finding kinds a task may allow (plan `allow:`, spec §7). `delete-test` also
/// allows `fewer-tests`.
pub const ALLOW_KINDS: [&str; 4] = ["delete-test", "skip", "only", "fewer-asserts"];

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
    /// Gated by the plan (D-016): `risk: high` or a touch path under a risk rule. The task gets
    /// its own single-task PR and never joins a bundle.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub gated: bool,
    /// Planned changed lines (`size:`), compared with the PR at review (drift).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size: Option<u32>,
}

/// The task metadata of an issue body, if it has a valid marker. The LAST marker counts, so text
/// quoted earlier in a body cannot override the real one.
pub fn parse_task_meta(body: &str) -> Option<TaskMeta> {
    let rest = &body[body.rfind(OPEN)? + OPEN.len()..];
    let json = &rest[..rest.find("-->")?];
    serde_json::from_str(json.trim()).ok()
}

/// The `budget_usd` of the last `<!-- bosun:epic {json} -->` marker of an epic body.
pub fn epic_budget(body: &str) -> Option<f64> {
    const OPEN: &str = "<!-- bosun:epic ";
    let rest = &body[body.rfind(OPEN)? + OPEN.len()..];
    let v: serde_json::Value = serde_json::from_str(rest[..rest.find("-->")?].trim()).ok()?;
    v["budget_usd"].as_f64()
}

const REF: &str = r"(?i)\b(?:close[sd]?|fix(?:e[sd])?|resolve[sd]?):?\s+(?:https?://(?:www\.)?github\.com/([\w.-]+/[\w.-]+)/(issues|pull)/(\d+)|([\w.-]+/[\w.-]+)#(\d+)|#(\d+))";

/// One closing reference of a PR body: `repo` is the lower-cased `owner/repo` of a qualified
/// reference, `None` for a plain `#N`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClosingRef {
    pub repo: Option<String>,
    pub number: u64,
}

/// Every issue reference a body closes, in order and without repeats: `Closes #N`,
/// `Closes owner/repo#N` and `Closes https://github.com/owner/repo/issues/N` (any GitHub closing
/// keyword, any case, optional colon). A URL to a pull request is not an issue.
pub fn closing_refs(body: &str) -> Vec<ClosingRef> {
    static RE: std::sync::LazyLock<regex::Regex> =
        std::sync::LazyLock::new(|| regex::Regex::new(REF).expect("static regex"));
    let mut out: Vec<ClosingRef> = Vec::new();
    for c in RE.captures_iter(body) {
        let r = if let Some(n) = c.get(3) {
            if &c[2] != "issues" {
                continue;
            }
            (Some(c[1].to_lowercase()), n.as_str())
        } else if let Some(n) = c.get(5) {
            (Some(c[4].to_lowercase()), n.as_str())
        } else {
            (None, &c[6])
        };
        if let Ok(number) = r.1.parse()
            && !out.iter().any(|o| o.repo == r.0 && o.number == number)
        {
            out.push(ClosingRef { repo: r.0, number });
        }
    }
    out
}

fn numbers(refs: impl Iterator<Item = u64>) -> Vec<u64> {
    let mut out: Vec<u64> = Vec::new();
    for n in refs {
        if !out.contains(&n) {
            out.push(n);
        }
    }
    out
}

/// Every issue number a PR body closes in any repo (plain and qualified, no repeats): the
/// conservative form, for counting. Use [`closing_issues_in`] to ask about this repo's issues.
pub fn closing_issues(body: &str) -> Vec<u64> {
    numbers(closing_refs(body).into_iter().map(|r| r.number))
}

/// The issues of `repo` (`owner/repo`, any case) a body closes: plain `#N` plus references
/// qualified with that repo. With `repo` `None` only plain references count.
pub fn closing_issues_in(body: &str, repo: Option<&str>) -> Vec<u64> {
    numbers(
        closing_refs(body)
            .into_iter()
            .filter(|r| match (&r.repo, repo) {
                (None, _) => true,
                (Some(a), Some(b)) => a.eq_ignore_ascii_case(b),
                (Some(_), None) => false,
            })
            .map(|r| r.number),
    )
}

/// Every issue of this repo a PR body links, in order of appearance and without repeats: plain
/// `#N` closing references and the `<!-- bosun:task-pr N -->` marker. A reference qualified with
/// another repo is never returned (its number means nothing here); use [`linked_issues_in`] where
/// the repo is known. Unlike [`closing_issues`], the marker counts, although the PR author controls it.
pub fn linked_issues(body: &str) -> Vec<u64> {
    linked_in(body, None)
}

/// [`linked_issues`] plus the references qualified with `repo` (`owner/repo`, any case).
pub fn linked_issues_in(body: &str, repo: &str) -> Vec<u64> {
    linked_in(body, Some(repo))
}

fn linked_in(body: &str, repo: Option<&str>) -> Vec<u64> {
    static RE: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
        regex::Regex::new(&format!(r"{REF}|<!--\s*bosun:task-pr\s+(\d+)\s*-->"))
            .expect("static regex")
    });
    let mut out: Vec<u64> = Vec::new();
    for c in RE.captures_iter(body) {
        let qualified = c.get(1).or_else(|| c.get(4)).map(|m| m.as_str());
        if c.get(3).is_some_and(|_| &c[2] != "issues") {
            continue;
        }
        if let Some(q) = qualified
            && !repo.is_some_and(|r| r.eq_ignore_ascii_case(q))
        {
            continue;
        }
        let n = [3, 5, 6, 7]
            .iter()
            .find_map(|&i| c.get(i))
            .and_then(|m| m.as_str().parse().ok());
        if let Some(n) = n
            && !out.contains(&n)
        {
            out.push(n);
        }
    }
    out
}

/// The area named by a `bosun:area/<a>` label; labels are not locked and the area goes into
/// spawn prompts unquoted, so only letters, digits, `_` and `-` count.
pub fn area_of<S: AsRef<str>>(labels: &[S]) -> Option<String> {
    labels
        .iter()
        .find_map(|l| l.as_ref().strip_prefix("bosun:area/"))
        .filter(|a| safe_word(a, "_-"))
        .map(str::to_string)
}

/// Letters, digits and `extra`, not empty and not starting with `-`.
pub fn safe_word(s: &str, extra: &str) -> bool {
    !s.is_empty()
        && !s.starts_with('-')
        && (s.chars()).all(|c| c.is_ascii_alphanumeric() || extra.contains(c))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_unsafe_area_is_none() {
        assert_eq!(area_of(&["bosun:area/core-io"]), Some("core-io".into()));
        assert_eq!(area_of(&["bosun:area/x; rm -rf ~"]), None);
        for bad in ["--help", "{a,-b}", "x*", "a/b"] {
            assert_eq!(area_of(&[format!("bosun:area/{bad}")]), None);
        }
    }

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
    fn closing_refs_know_all_three_forms() {
        let body = "Closes #1\nfixes: Acme/Repo#2\nResolved https://github.com/Acme/Repo/issues/3\n\
                    closes #1\nCloses https://github.com/o/r/pull/9\nCloses acme/repo#2 and closed #4";
        let refs = closing_refs(body);
        let got: Vec<(Option<&str>, u64)> =
            refs.iter().map(|r| (r.repo.as_deref(), r.number)).collect();
        assert_eq!(
            got,
            vec![
                (None, 1),
                (Some("acme/repo"), 2),
                (Some("acme/repo"), 3),
                (None, 4)
            ]
        );
    }

    #[test]
    fn closing_issues_in_filters_by_repo() {
        let body = "Closes #1\nCloses other/thing#2\nCloses https://github.com/Me/Here/issues/3";
        assert_eq!(closing_issues(body), vec![1, 2, 3]);
        assert_eq!(closing_issues_in(body, Some("me/here")), vec![1, 3]);
        assert_eq!(closing_issues_in(body, None), vec![1]);
    }

    #[test]
    fn linked_issues_never_return_other_repo_refs() {
        let body =
            "Fixes o/r#5 and Closes https://github.com/o/r/issues/6 <!-- bosun:task-pr 9 -->";
        assert_eq!(linked_issues(body), vec![9]);
        assert!(linked_issues("Fixes keelstackhq/splice#40").is_empty());
        assert_eq!(
            linked_issues("Closes #5\nFixes keelstackhq/splice#40"),
            vec![5]
        );
        assert_eq!(linked_issues_in(body, "O/R"), vec![5, 6, 9]);
    }

    #[test]
    fn ref_accepts_http_and_www_urls() {
        let body =
            "Closes http://github.com/o/r/issues/1\nCloses https://www.github.com/o/r/issues/2";
        assert_eq!(closing_issues_in(body, Some("o/r")), vec![1, 2]);
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
