//! The slices of GitHub's REST shapes bosun reads.

use serde::{Deserialize, Deserializer};

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Issue {
    pub number: u64,
    pub title: String,
    /// `None` when GitHub sent `null`; see [`Issue::body_text`].
    #[serde(default)]
    pub body: Option<String>,
    pub state: String,
    #[serde(default, deserialize_with = "label_names")]
    pub labels: Vec<String>,
    /// GitHub lists pull requests as issues; they carry a `pull_request` key.
    #[serde(default, deserialize_with = "present")]
    pub pull_request: bool,
    #[serde(default, deserialize_with = "login")]
    pub user: String,
}

impl Issue {
    /// The body, `""` when GitHub sent `null`.
    pub fn body_text(&self) -> &str {
        self.body.as_deref().unwrap_or("")
    }
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Comment {
    pub id: u64,
    #[serde(default, deserialize_with = "text_or_empty")]
    pub body: String,
    #[serde(default, deserialize_with = "login")]
    pub user: String,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Label {
    pub name: String,
    #[serde(default)]
    pub color: String,
}

fn text_or_empty<'de, D: Deserializer<'de>>(d: D) -> Result<String, D::Error> {
    Ok(Option::<String>::deserialize(d)?.unwrap_or_default())
}

/// True when the key is present with a non-null value.
fn present<'de, D: Deserializer<'de>>(d: D) -> Result<bool, D::Error> {
    Ok(Option::<serde::de::IgnoredAny>::deserialize(d)?.is_some())
}

fn login<'de, D: Deserializer<'de>>(d: D) -> Result<String, D::Error> {
    #[derive(Deserialize)]
    struct User {
        login: String,
    }
    Ok(Option::<User>::deserialize(d)?
        .map(|u| u.login)
        .unwrap_or_default())
}

/// Labels arrive as `[{name, ...}]` (or plain strings in some payloads).
fn label_names<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<String>, D::Error> {
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum L {
        Name(String),
        Obj { name: String },
    }
    Ok(Vec::<L>::deserialize(d)?
        .into_iter()
        .map(|l| match l {
            L::Name(n) | L::Obj { name: n } => n,
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn issue_fixture_parses() {
        let i: Issue =
            serde_json::from_str(include_str!("../../tests/fixtures/github/issue.json")).unwrap();
        assert_eq!(i.number, 1347);
        assert_eq!(i.labels, vec!["bosun:state/ready", "bug"]);
        assert_eq!(i.user, "octocat");
        assert_eq!(i.body_text(), "I'm having a problem with this.");
        assert!(!i.pull_request);
    }

    #[test]
    fn pull_request_key_and_null_body() {
        let i: Issue = serde_json::from_str(
            r#"{"number":2,"title":"t","body":null,"state":"open","pull_request":{"url":"x"}}"#,
        )
        .unwrap();
        assert!(i.pull_request);
        assert_eq!(i.body_text(), "");
        assert!(i.labels.is_empty());
    }

    #[test]
    fn comment_and_label_fixtures_parse() {
        let c: Comment =
            serde_json::from_str(include_str!("../../tests/fixtures/github/comment.json")).unwrap();
        assert_eq!(
            (c.id, c.body.as_str(), c.user.as_str()),
            (1, "Me too", "octocat")
        );
        assert_eq!(c.created_at, "2011-04-14T16:00:49Z");
        let l: Label =
            serde_json::from_str(include_str!("../../tests/fixtures/github/label.json")).unwrap();
        assert_eq!((l.name.as_str(), l.color.as_str()), ("bug", "f29513"));
    }
}
