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
    /// `performed_via_github_app`: `None` when GitHub left the field out, `Some(None)` when it
    /// is `null` (a plain user token), `Some(Some(slug))` when an app wrote the comment.
    #[serde(
        default,
        rename = "performed_via_github_app",
        deserialize_with = "app_slug"
    )]
    pub via_app: Option<Option<String>>,
    #[serde(default)]
    pub author_association: String,
    /// `.../issues/<n>`: the issue or PR the comment is on.
    #[serde(default)]
    pub issue_url: String,
}

impl Comment {
    /// Written with a plain user token: the app field is present and `null`. A missing field
    /// is not proof, so it counts as written by an app (fail closed).
    pub fn by_user_token(&self) -> bool {
        self.via_app == Some(None)
    }

    /// Never edited since it was created.
    pub fn unedited(&self) -> bool {
        self.created_at == self.updated_at
    }
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Label {
    pub name: String,
    #[serde(default)]
    pub color: String,
}

/// A pull request. `merged` is only sent by the single-pull endpoint; lists leave it `false`.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(from = "RawPull")]
pub struct Pull {
    pub number: u64,
    pub node_id: String,
    pub title: String,
    pub head_sha: String,
    pub head_ref: String,
    pub base_sha: String,
    pub base_ref: String,
    pub body: String,
    pub state: String,
    pub merged: bool,
    pub draft: bool,
    pub labels: Vec<String>,
    pub user: String,
    /// Files the PR changes; 0 when GitHub did not say (lists omit it).
    pub changed_files: u64,
}

#[derive(Deserialize)]
struct RawRef {
    sha: String,
    #[serde(rename = "ref")]
    name: String,
}

#[derive(Deserialize)]
struct RawPull {
    number: u64,
    #[serde(default)]
    node_id: String,
    #[serde(default)]
    title: String,
    head: RawRef,
    base: RawRef,
    #[serde(default, deserialize_with = "text_or_empty")]
    body: String,
    state: String,
    #[serde(default)]
    merged: bool,
    #[serde(default)]
    draft: bool,
    #[serde(default, deserialize_with = "label_names")]
    labels: Vec<String>,
    #[serde(default, deserialize_with = "login")]
    user: String,
    #[serde(default)]
    changed_files: u64,
}

impl From<RawPull> for Pull {
    fn from(r: RawPull) -> Pull {
        Pull {
            number: r.number,
            node_id: r.node_id,
            title: r.title,
            head_sha: r.head.sha,
            head_ref: r.head.name,
            base_sha: r.base.sha,
            base_ref: r.base.name,
            body: r.body,
            state: r.state,
            merged: r.merged,
            draft: r.draft,
            labels: r.labels,
            user: r.user,
            changed_files: r.changed_files,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct PrFile {
    pub filename: String,
    pub status: String,
    #[serde(default)]
    pub additions: u64,
    #[serde(default)]
    pub deletions: u64,
    /// Absent for binary and very large files.
    #[serde(default)]
    pub patch: Option<String>,
    #[serde(default)]
    pub previous_filename: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Review {
    #[serde(default, deserialize_with = "login")]
    pub user: String,
    pub state: String,
    #[serde(default, deserialize_with = "text_or_empty")]
    pub commit_id: String,
}

/// One commit status as the combined-status endpoint lists it.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Status {
    pub context: String,
    pub state: String,
    #[serde(default, deserialize_with = "text_or_empty")]
    pub description: String,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct CheckRun {
    pub name: String,
    pub status: String,
    /// `None` until the run completes.
    #[serde(default)]
    pub conclusion: Option<String>,
}

/// A job of an Actions workflow run.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Job {
    pub id: u64,
    pub name: String,
    pub run_id: u64,
    #[serde(default, deserialize_with = "text_or_empty")]
    pub conclusion: String,
    #[serde(default, deserialize_with = "text_or_empty")]
    pub html_url: String,
}

fn text_or_empty<'de, D: Deserializer<'de>>(d: D) -> Result<String, D::Error> {
    Ok(Option::<String>::deserialize(d)?.unwrap_or_default())
}

/// True when the key is present with a non-null value.
fn present<'de, D: Deserializer<'de>>(d: D) -> Result<bool, D::Error> {
    Ok(Option::<serde::de::IgnoredAny>::deserialize(d)?.is_some())
}

fn app_slug<'de, D: Deserializer<'de>>(d: D) -> Result<Option<Option<String>>, D::Error> {
    #[derive(Deserialize)]
    struct App {
        #[serde(default)]
        slug: String,
    }
    Ok(Some(Option::<App>::deserialize(d)?.map(|a| a.slug)))
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
