//! GitHub as the shared store: the `Github` trait and its live implementation (spec §1, D28).

pub mod b64;
pub mod http;
pub mod types;

use http::{Http, enc};
use serde_json::{Value, json};
pub use types::{CheckRun, Comment, Issue, Job, Label, PrFile, Pull, Review, Status};

pub use crate::output::BosunError;
pub type Result<T> = std::result::Result<T, BosunError>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IssueState {
    Open,
    Closed,
    All,
}

impl IssueState {
    pub fn as_str(self) -> &'static str {
        match self {
            IssueState::Open => "open",
            IssueState::Closed => "closed",
            IssueState::All => "all",
        }
    }
}

/// The states an issue can be moved to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PatchState {
    Open,
    Closed,
}

impl PatchState {
    pub fn as_str(self) -> &'static str {
        match self {
            PatchState::Open => "open",
            PatchState::Closed => "closed",
        }
    }
}

/// Fields to change on an issue; `None` leaves a field alone.
#[derive(Debug, Clone, Default)]
pub struct IssuePatch {
    pub title: Option<String>,
    pub body: Option<String>,
    pub state: Option<PatchState>,
    pub state_reason: Option<String>,
}

/// Compare pages read at most (3000 files, GitHub's own cap).
const MAX_COMPARE_PAGES: usize = 30;

pub trait Github {
    /// Issues (never pull requests) carrying all of `labels`.
    fn list_issues(&self, labels: &[&str], state: IssueState) -> Result<Vec<Issue>>;
    fn get_issue(&self, number: u64) -> Result<Option<Issue>>;
    fn create_issue(&self, title: &str, body: &str, labels: &[&str]) -> Result<Issue>;
    fn update_issue(&self, number: u64, patch: IssuePatch) -> Result<Issue>;
    fn add_labels(&self, number: u64, labels: &[&str]) -> Result<()>;
    /// A label that is not on the issue is fine.
    fn remove_label(&self, number: u64, label: &str) -> Result<()>;
    /// Create the label; one that already exists is fine.
    fn ensure_label(&self, name: &str, color: &str, description: &str) -> Result<()>;
    fn list_comments(&self, number: u64) -> Result<Vec<Comment>>;
    fn add_comment(&self, number: u64, body: &str) -> Result<Comment>;
    fn update_comment(&self, id: u64, body: &str) -> Result<Comment>;
    /// A repository Actions variable; `None` when it is not set.
    fn variable(&self, name: &str) -> Result<Option<String>>;
    fn get_pull(&self, number: u64) -> Result<Option<Pull>>;
    /// Pull requests in `state`; `head` is `owner:branch`.
    fn list_pulls(&self, head: Option<&str>, state: IssueState) -> Result<Vec<Pull>>;
    /// Every changed file, all pages.
    fn pull_files(&self, number: u64) -> Result<Vec<PrFile>>;
    /// The files changed between two commits (`base...head`).
    fn compare_files(&self, base: &str, head: &str) -> Result<Vec<PrFile>>;
    fn create_pull(&self, title: &str, head: &str, base: &str, body: &str) -> Result<Pull>;
    fn update_pull_body(&self, number: u64, body: &str) -> Result<()>;
    fn list_reviews(&self, number: u64) -> Result<Vec<Review>>;
    /// `state` is `success`, `failure`, `pending` or `error`; the description is cut to 140
    /// characters.
    fn create_status(
        &self,
        sha: &str,
        context: &str,
        state: &str,
        description: &str,
        target_url: Option<&str>,
    ) -> Result<()>;
    /// The combined status of `sha`: the newest status per context.
    fn statuses(&self, sha: &str) -> Result<Vec<Status>>;
    fn check_runs(&self, sha: &str) -> Result<Vec<CheckRun>>;
    /// A file at a ref; `None` when it is missing or is a directory.
    fn file_at(&self, path: &str, git_ref: &str) -> Result<Option<String>>;
    /// `method` is `SQUASH`, `MERGE` or `REBASE`; `node_id` is the pull request's.
    fn enable_auto_merge(&self, node_id: &str, method: &str) -> Result<()>;
    /// Jobs of the workflow runs of `sha` that failed, were cancelled or timed out.
    fn failed_jobs(&self, sha: &str) -> Result<Vec<Job>>;
    /// The last `tail` lines of a job's log.
    fn job_log(&self, job_id: u64, tail: usize) -> Result<String>;
    /// The repository itself; `None` when it is not readable (404).
    fn repo_info(&self) -> Result<Option<RepoInfo>>;
    /// The newest published release; `None` when there is none.
    fn latest_release(&self) -> Result<Option<Release>>;
}

#[derive(Debug, Clone, PartialEq, serde::Deserialize)]
pub struct RepoInfo {
    pub default_branch: String,
    #[serde(default)]
    pub allow_auto_merge: bool,
    /// Present with an authenticated token: what the token may do.
    #[serde(default)]
    pub permissions: Option<Value>,
}

#[derive(Debug, Clone, PartialEq, serde::Deserialize)]
pub struct Release {
    pub tag_name: String,
}

const STATUS_STATES: [&str; 4] = ["success", "failure", "pending", "error"];
const MERGE_METHODS: [&str; 3] = ["SQUASH", "MERGE", "REBASE"];
const AUTO_MERGE: &str = "mutation($id: ID!, $method: PullRequestMergeMethod!) { \
    enablePullRequestAutoMerge(input: {pullRequestId: $id, mergeMethod: $method}) \
    { pullRequest { number } } }";

pub struct LiveGithub {
    http: Http,
    owner: String,
    repo: String,
}

impl LiveGithub {
    /// Base URL and token come from the environment (`BOSUN_GITHUB_API`, `GITHUB_TOKEN`).
    pub fn new(owner: &str, repo: &str) -> Result<Self> {
        Ok(LiveGithub {
            http: Http::from_env()?,
            owner: owner.to_string(),
            repo: repo.to_string(),
        })
    }

    /// Explicit settings, for tests.
    pub fn with(base_url: &str, token: &str, owner: &str, repo: &str, max_sleep_ms: u64) -> Self {
        LiveGithub {
            http: Http::new(base_url, token, max_sleep_ms),
            owner: owner.to_string(),
            repo: repo.to_string(),
        }
    }

    fn path(&self, rest: &str) -> String {
        format!("/repos/{}/{}{rest}", enc(&self.owner), enc(&self.repo))
    }

    fn parse<T: serde::de::DeserializeOwned>(&self, what: &str, v: Value) -> Result<T> {
        serde_json::from_value(v).map_err(|e| {
            BosunError::error(
                format!("GitHub sent an unexpected {what}: {e}"),
                "retry; if it persists check https://www.githubstatus.com",
            )
        })
    }

    fn parse_all<T: serde::de::DeserializeOwned>(
        &self,
        what: &str,
        v: Vec<Value>,
    ) -> Result<Vec<T>> {
        v.into_iter().map(|i| self.parse(what, i)).collect()
    }
}

impl Github for LiveGithub {
    fn repo_info(&self) -> Result<Option<RepoInfo>> {
        match self.http.get_json(&self.path(""))? {
            Some(v) => Ok(Some(self.parse("repository", v)?)),
            None => Ok(None),
        }
    }

    fn latest_release(&self) -> Result<Option<Release>> {
        match self.http.get_json(&self.path("/releases/latest"))? {
            Some(v) => Ok(Some(self.parse("release", v)?)),
            None => Ok(None),
        }
    }

    fn list_issues(&self, labels: &[&str], state: IssueState) -> Result<Vec<Issue>> {
        let mut path = self.path(&format!("/issues?state={}&per_page=100", state.as_str()));
        if !labels.is_empty() {
            let joined = labels.iter().map(|l| enc(l)).collect::<Vec<_>>().join(",");
            path.push_str(&format!("&labels={joined}"));
        }
        let issues: Vec<Issue> = self.parse_all("issue", self.http.get_all(&path)?)?;
        Ok(issues.into_iter().filter(|i| !i.pull_request).collect())
    }

    fn get_issue(&self, number: u64) -> Result<Option<Issue>> {
        match self
            .http
            .get_json(&self.path(&format!("/issues/{number}")))?
        {
            Some(v) => Ok(Some(self.parse("issue", v)?)),
            None => Ok(None),
        }
    }

    fn create_issue(&self, title: &str, body: &str, labels: &[&str]) -> Result<Issue> {
        let v = self.http.send_json(
            "POST",
            &self.path("/issues"),
            &json!({"title": title, "body": body, "labels": labels}),
        )?;
        self.parse("issue", v)
    }

    fn update_issue(&self, number: u64, patch: IssuePatch) -> Result<Issue> {
        let mut body = serde_json::Map::new();
        if let Some(t) = patch.title {
            body.insert("title".into(), json!(t));
        }
        if let Some(b) = patch.body {
            body.insert("body".into(), json!(b));
        }
        if let Some(s) = patch.state {
            body.insert("state".into(), json!(s.as_str()));
        }
        if let Some(r) = patch.state_reason {
            body.insert("state_reason".into(), json!(r));
        }
        let v = self.http.send_json(
            "PATCH",
            &self.path(&format!("/issues/{number}")),
            &Value::Object(body),
        )?;
        self.parse("issue", v)
    }

    fn add_labels(&self, number: u64, labels: &[&str]) -> Result<()> {
        let path = self.path(&format!("/issues/{number}/labels"));
        self.http
            .send_json("POST", &path, &json!({"labels": labels}))?;
        Ok(())
    }

    fn remove_label(&self, number: u64, label: &str) -> Result<()> {
        let path = self.path(&format!("/issues/{number}/labels/{}", enc(label)));
        self.http
            .send_accepting("DELETE", &path, &Value::Null, &[404])?;
        Ok(())
    }

    fn ensure_label(&self, name: &str, color: &str, description: &str) -> Result<()> {
        let body = json!({"name": name, "color": color, "description": description});
        self.http
            .send_accepting("POST", &self.path("/labels"), &body, &[422])?;
        Ok(())
    }

    fn list_comments(&self, number: u64) -> Result<Vec<Comment>> {
        let path = self.path(&format!("/issues/{number}/comments?per_page=100"));
        self.parse_all("comment", self.http.get_all(&path)?)
    }

    fn add_comment(&self, number: u64, body: &str) -> Result<Comment> {
        let path = self.path(&format!("/issues/{number}/comments"));
        let v = self.http.send_json("POST", &path, &json!({"body": body}))?;
        self.parse("comment", v)
    }

    fn update_comment(&self, id: u64, body: &str) -> Result<Comment> {
        let path = self.path(&format!("/issues/comments/{id}"));
        let v = self
            .http
            .send_json("PATCH", &path, &json!({"body": body}))?;
        self.parse("comment", v)
    }

    fn variable(&self, name: &str) -> Result<Option<String>> {
        let path = self.path(&format!("/actions/variables/{}", enc(name)));
        Ok(self
            .http
            .get_json(&path)?
            .and_then(|v| v["value"].as_str().map(str::to_string)))
    }

    fn get_pull(&self, number: u64) -> Result<Option<Pull>> {
        match self
            .http
            .get_json(&self.path(&format!("/pulls/{number}")))?
        {
            Some(v) => Ok(Some(self.parse("pull request", v)?)),
            None => Ok(None),
        }
    }

    fn list_pulls(&self, head: Option<&str>, state: IssueState) -> Result<Vec<Pull>> {
        let mut path = self.path(&format!("/pulls?state={}&per_page=100", state.as_str()));
        if let Some(h) = head {
            path.push_str(&format!("&head={}", enc(h)));
        }
        self.parse_all("pull request", self.http.get_all(&path)?)
    }

    fn pull_files(&self, number: u64) -> Result<Vec<PrFile>> {
        let path = self.path(&format!("/pulls/{number}/files?per_page=100"));
        self.parse_all("pull request file", self.http.get_all(&path)?)
    }

    fn compare_files(&self, base: &str, head: &str) -> Result<Vec<PrFile>> {
        // The endpoint pages its `files` array; read pages until one is short.
        let mut all: Vec<Value> = Vec::new();
        for page in 1..=MAX_COMPARE_PAGES {
            let path = self.path(&format!(
                "/compare/{}...{}?page={page}&per_page=100",
                enc(base),
                enc(head)
            ));
            let Some(v) = self.http.get_json(&path)? else {
                return Err(BosunError::error(
                    format!("GitHub cannot compare {base}...{head}"),
                    "check that both commits exist; a force-push may have removed the old one",
                ));
            };
            let files = v["files"].as_array().cloned().unwrap_or_default();
            let short = files.len() < 100;
            all.extend(files);
            if short {
                break;
            }
        }
        self.parse_all("compared file", all)
    }

    fn create_pull(&self, title: &str, head: &str, base: &str, body: &str) -> Result<Pull> {
        let v = self.http.send_json(
            "POST",
            &self.path("/pulls"),
            &json!({"title": title, "head": head, "base": base, "body": body}),
        )?;
        self.parse("pull request", v)
    }

    fn update_pull_body(&self, number: u64, body: &str) -> Result<()> {
        let path = self.path(&format!("/pulls/{number}"));
        self.http
            .send_json("PATCH", &path, &json!({"body": body}))?;
        Ok(())
    }

    fn list_reviews(&self, number: u64) -> Result<Vec<Review>> {
        let path = self.path(&format!("/pulls/{number}/reviews?per_page=100"));
        self.parse_all("review", self.http.get_all(&path)?)
    }

    fn create_status(
        &self,
        sha: &str,
        context: &str,
        state: &str,
        description: &str,
        target_url: Option<&str>,
    ) -> Result<()> {
        if !STATUS_STATES.contains(&state) {
            return Err(BosunError::error(
                format!("status state {state:?} is not one GitHub accepts"),
                "use success, failure, pending or error",
            ));
        }
        let description: String = description.chars().take(140).collect();
        let mut body = json!({"state": state, "context": context, "description": description});
        if let Some(u) = target_url {
            body["target_url"] = json!(u);
        }
        let path = self.path(&format!("/statuses/{}", enc(sha)));
        self.http.send_json("POST", &path, &body)?;
        Ok(())
    }

    fn statuses(&self, sha: &str) -> Result<Vec<Status>> {
        let path = self.path(&format!("/commits/{}/status?per_page=100", enc(sha)));
        match self.http.get_json(&path)? {
            Some(v) => self.parse_all(
                "status",
                v["statuses"].as_array().cloned().unwrap_or_default(),
            ),
            None => Ok(vec![]),
        }
    }

    fn check_runs(&self, sha: &str) -> Result<Vec<CheckRun>> {
        let path = self.path(&format!("/commits/{}/check-runs?per_page=100", enc(sha)));
        self.parse_all("check run", self.http.get_all_in(&path, "check_runs")?)
    }

    fn file_at(&self, path: &str, git_ref: &str) -> Result<Option<String>> {
        let segments: Vec<String> = path.split('/').map(enc).collect();
        let url = self.path(&format!(
            "/contents/{}?ref={}",
            segments.join("/"),
            enc(git_ref)
        ));
        let Some(v) = self.http.get_json(&url)? else {
            return Ok(None);
        };
        if v["type"] != "file" {
            return Ok(None); // a directory (an array), symlink or submodule
        }
        let unusable = |why: String| {
            BosunError::error(
                format!("GitHub sent {path} at {git_ref} in a form bosun cannot read: {why}"),
                "files over 1 MB are not served by the contents API; keep this file smaller",
            )
        };
        if v["encoding"] != "base64" {
            return Err(unusable(format!("encoding {}", v["encoding"])));
        }
        let bytes = b64::decode(v["content"].as_str().unwrap_or(""))
            .ok_or_else(|| unusable("invalid base64".into()))?;
        String::from_utf8(bytes)
            .map(Some)
            .map_err(|_| unusable("not UTF-8 text".into()))
    }

    fn enable_auto_merge(&self, node_id: &str, method: &str) -> Result<()> {
        if !MERGE_METHODS.contains(&method) {
            return Err(BosunError::error(
                format!("merge method {method:?} is not one GitHub accepts"),
                "use SQUASH, MERGE or REBASE",
            ));
        }
        let body = json!({"query": AUTO_MERGE, "variables": {"id": node_id, "method": method}});
        let v = self.http.send_json("POST", "/graphql", &body)?;
        match v["errors"].as_array().and_then(|e| e.first()) {
            Some(first) => Err(BosunError::error(
                format!(
                    "GitHub refused auto-merge: {}",
                    first["message"].as_str().unwrap_or("unknown GraphQL error")
                ),
                "enable 'Allow auto-merge' in the repository settings and require status checks on the base branch",
            )),
            None => Ok(()),
        }
    }

    fn failed_jobs(&self, sha: &str) -> Result<Vec<Job>> {
        let path = self.path(&format!("/actions/runs?head_sha={}&per_page=100", enc(sha)));
        let mut failed = Vec::new();
        for run in self.http.get_all_in(&path, "workflow_runs")? {
            let Some(id) = run["id"].as_u64() else {
                continue;
            };
            let path = self.path(&format!("/actions/runs/{id}/jobs?per_page=100"));
            let jobs: Vec<Job> = self.parse_all("job", self.http.get_all_in(&path, "jobs")?)?;
            failed.extend(jobs.into_iter().filter(|j| {
                ["failure", "cancelled", "timed_out"].contains(&j.conclusion.as_str())
            }));
        }
        Ok(failed)
    }

    fn job_log(&self, job_id: u64, tail: usize) -> Result<String> {
        let path = self.path(&format!("/actions/jobs/{job_id}/logs"));
        let text = self.http.get_text(&path)?.ok_or_else(|| {
            BosunError::error(
                format!("GitHub has no log for job {job_id}"),
                "logs expire after 90 days; check the job id",
            )
        })?;
        let lines: Vec<&str> = text.lines().collect();
        Ok(lines[lines.len().saturating_sub(tail)..].join("\n"))
    }
}
