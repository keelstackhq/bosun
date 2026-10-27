//! GitHub as the shared store: the `Github` trait and its live implementation (spec §1, D28).

pub mod b64;
pub mod http;
pub mod types;

use http::{Http, enc};
use serde_json::{Value, json};
pub use types::{CheckRun, Comment, Compare, Issue, Job, Label, PrFile, Pull, Review, Status};

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

/// What `GET /issues/N` says about one number.
pub enum Probe {
    Issue(Box<Issue>),
    /// 404: no such number (the numbers end here).
    Missing,
    /// Deleted (410) or transferred (301): the number is used but holds nothing for this repo.
    Gone,
}

/// Pages (of 100) that `recent_issues` reads.
pub const RECENT_PAGES: usize = 5;

pub trait Github {
    /// Issues (never pull requests) carrying all of `labels`.
    fn list_issues(&self, labels: &[&str], state: IssueState) -> Result<Vec<Issue>>;
    /// Issues (never pull requests) updated since `since` (RFC 3339), newest update first, at
    /// most `RECENT_PAGES` pages of 100; the flag says that more pages were left.
    fn recent_issues(&self, since: &str) -> Result<(Vec<Issue>, bool)>;
    fn get_issue(&self, number: u64) -> Result<Option<Issue>>;
    /// Like `get_issue`, but a deleted or transferred number is `Gone`, not an error.
    fn probe_issue(&self, number: u64) -> Result<Probe>;
    fn create_issue(&self, title: &str, body: &str, labels: &[&str]) -> Result<Issue>;
    fn update_issue(&self, number: u64, patch: IssuePatch) -> Result<Issue>;
    fn add_labels(&self, number: u64, labels: &[&str]) -> Result<()>;
    /// A label that is not on the issue is fine.
    fn remove_label(&self, number: u64, label: &str) -> Result<()>;
    /// Whether the repository has the label.
    fn label_exists(&self, name: &str) -> Result<bool>;
    /// Whether the branch exists on GitHub (is pushed).
    fn branch_exists(&self, name: &str) -> Result<bool>;
    /// Create the label; one that already exists is fine.
    fn ensure_label(&self, name: &str, color: &str, description: &str) -> Result<()>;
    fn list_comments(&self, number: u64) -> Result<Vec<Comment>>;
    fn add_comment(&self, number: u64, body: &str) -> Result<Comment>;
    /// A user's permission on the repository (`admin`, `write`, `read` or `none`; maintain
    /// reads as `write`); `None` when GitHub does not know the user.
    fn permission(&self, user: &str) -> Result<Option<String>>;
    /// One issue or PR comment by id; `None` when it does not exist.
    fn get_comment(&self, id: u64) -> Result<Option<Comment>>;
    fn update_comment(&self, id: u64, body: &str) -> Result<Comment>;
    /// A repository Actions variable.
    fn variable(&self, name: &str) -> Result<Variable>;
    fn get_pull(&self, number: u64) -> Result<Option<Pull>>;
    /// Pull requests in `state`; `head` is `owner:branch`.
    fn list_pulls(&self, head: Option<&str>, state: IssueState) -> Result<Vec<Pull>>;
    /// Every changed file, all pages.
    fn pull_files(&self, number: u64) -> Result<Vec<PrFile>>;
    /// The files changed between two commits (`base...head`).
    fn compare_files(&self, base: &str, head: &str) -> Result<Vec<PrFile>>;
    /// Like `compare_files`, with the comparison's `status` (`ahead` when base is an ancestor).
    fn compare(&self, base: &str, head: &str) -> Result<Compare>;
    /// The shas of the pull request's commits (GitHub lists at most 250).
    fn pull_commits(&self, number: u64) -> Result<Vec<String>>;
    /// The first parent of commit `sha`; `None` when GitHub has no such commit or it has no parent.
    fn first_parent(&self, sha: &str) -> Result<Option<String>>;
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
    /// Every status of `sha`, newest first (not only the newest per context), with its creator.
    fn status_history(&self, sha: &str) -> Result<Vec<Status>>;
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
    /// The status checks the default branch enforces: rulesets plus classic branch protection.
    fn required_status_checks(&self, branch: &str) -> Result<Live>;
    /// Auto-merge through the Claude Code cloud route; `method` is `merge`, `squash` or `rebase`.
    fn enable_auto_merge_rest(&self, number: u64, method: &str) -> Result<()>;
    /// Whether the repo path `rest` (such as `/actions/secrets/NAME`) exists: 200 is `Some(true)`,
    /// 404 `Some(false)`, 403 (the token cannot read it) `None`.
    fn exists(&self, rest: &str) -> Result<Option<bool>>;
}

/// What the Claude Code cloud proxy says when it refuses GraphQL.
pub const CCR_GRAPHQL_REFUSAL: &str = "GraphQL is not available from Claude Code sessions";

/// Turn on squash auto-merge: GraphQL first, the CCR route when the proxy refuses GraphQL.
/// Returns the route used: `graphql` or `ccr`.
pub fn enable_auto_merge_any(gh: &dyn Github, pull: &Pull) -> Result<&'static str> {
    match gh.enable_auto_merge(&pull.node_id, "SQUASH") {
        Ok(()) => Ok("graphql"),
        Err(e) if e.message.contains(" 403 ") && e.message.contains(CCR_GRAPHQL_REFUSAL) => {
            gh.enable_auto_merge_rest(pull.number, "squash")?;
            Ok("ccr")
        }
        Err(e) => Err(e),
    }
}

/// The required status checks of a branch as the token sees them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Live {
    /// The contexts enforced (empty: nothing is, or the branch has no rule).
    Checks(Vec<String>),
    /// One endpoint answered 403 (named): the contexts the other one enforces, which may be
    /// only part of them (classic protection is often unreadable while rulesets are not).
    Partial(Vec<String>, &'static str),
    /// Both endpoints answered 403: a private repo on a free plan has no rulesets, or the token
    /// lacks Administration: read.
    Forbidden,
}

/// A repository Actions variable as the token sees it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Variable {
    Set(String),
    /// 404: the variable does not exist.
    Unset,
    /// 403 (not a rate limit): the token may not read variables.
    Forbidden,
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

    /// The given token (not the `GITHUB_TOKEN` of the environment) against `BOSUN_GITHUB_API`.
    pub fn with_token(token: &str, owner: &str, repo: &str) -> Self {
        LiveGithub {
            http: Http::with_env_url(token),
            owner: owner.to_string(),
            repo: repo.to_string(),
        }
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

    fn recent_issues(&self, since: &str) -> Result<(Vec<Issue>, bool)> {
        let path = self.path(&format!(
            "/issues?state=all&sort=updated&direction=desc&since={}&per_page=100",
            enc(since)
        ));
        let (items, more) = self.http.get_pages(&path, RECENT_PAGES)?;
        let issues: Vec<Issue> = self.parse_all("issue", items)?;
        Ok((
            issues.into_iter().filter(|i| !i.pull_request).collect(),
            more,
        ))
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

    fn probe_issue(&self, number: u64) -> Result<Probe> {
        match self
            .http
            .get_json_or_gone(&self.path(&format!("/issues/{number}")))?
        {
            Ok(Some(v)) => Ok(Probe::Issue(Box::new(self.parse("issue", v)?))),
            Ok(None) => Ok(Probe::Missing),
            Err(()) => Ok(Probe::Gone),
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

    fn label_exists(&self, name: &str) -> Result<bool> {
        let path = self.path(&format!("/labels/{}", enc(name)));
        Ok(self.http.get_json(&path)?.is_some())
    }

    fn branch_exists(&self, name: &str) -> Result<bool> {
        let path = self.path(&format!("/branches/{}", enc(name)));
        Ok(self.http.get_json(&path)?.is_some())
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

    fn permission(&self, user: &str) -> Result<Option<String>> {
        let path = self.path(&format!("/collaborators/{}/permission", enc(user)));
        Ok(self
            .http
            .get_json(&path)?
            .and_then(|v| v["permission"].as_str().map(str::to_string)))
    }

    fn get_comment(&self, id: u64) -> Result<Option<Comment>> {
        match self
            .http
            .get_json(&self.path(&format!("/issues/comments/{id}")))?
        {
            Some(v) => Ok(Some(self.parse("comment", v)?)),
            None => Ok(None),
        }
    }

    fn update_comment(&self, id: u64, body: &str) -> Result<Comment> {
        let path = self.path(&format!("/issues/comments/{id}"));
        let v = self
            .http
            .send_json("PATCH", &path, &json!({"body": body}))?;
        self.parse("comment", v)
    }

    fn required_status_checks(&self, branch: &str) -> Result<Live> {
        let b = enc(branch);
        let rules = self
            .http
            .get_json_or_forbidden(&self.path(&format!("/rules/branches/{b}")))?;
        let classic = self.http.get_json_or_forbidden(
            &self.path(&format!("/branches/{b}/protection/required_status_checks")),
        )?;
        if rules.is_err() && classic.is_err() {
            return Ok(Live::Forbidden);
        }
        let (rules_403, classic_403) = (rules.is_err(), classic.is_err());
        let mut out: Vec<String> = Vec::new();
        for rule in rules
            .ok()
            .flatten()
            .iter()
            .flat_map(|v| v.as_array())
            .flatten()
        {
            if rule["type"] == "required_status_checks" {
                let list = rule["parameters"]["required_status_checks"].as_array();
                out.extend(
                    list.into_iter()
                        .flatten()
                        .filter_map(|c| c["context"].as_str().map(String::from)),
                );
            }
        }
        if let Some(c) = classic.ok().flatten() {
            let ctx = c["contexts"].as_array().into_iter().flatten();
            let checks = c["checks"]
                .as_array()
                .into_iter()
                .flatten()
                .map(|x| &x["context"]);
            out.extend(
                ctx.chain(checks)
                    .filter_map(|x| x.as_str().map(String::from)),
            );
        }
        out.sort();
        out.dedup();
        Ok(if rules_403 {
            Live::Partial(out, "rulesets (/rules/branches)")
        } else if classic_403 {
            Live::Partial(out, "classic branch protection")
        } else {
            Live::Checks(out)
        })
    }

    fn variable(&self, name: &str) -> Result<Variable> {
        let path = self.path(&format!("/actions/variables/{}", enc(name)));
        Ok(match self.http.get_json_or_forbidden(&path)? {
            Ok(Some(v)) => Variable::Set(v["value"].as_str().unwrap_or("").to_string()),
            Ok(None) => Variable::Unset,
            Err(()) => Variable::Forbidden,
        })
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

    fn first_parent(&self, sha: &str) -> Result<Option<String>> {
        let v = self
            .http
            .get_json(&self.path(&format!("/commits/{}", enc(sha))))?;
        Ok(v.and_then(|v| v["parents"][0]["sha"].as_str().map(String::from)))
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
        Ok(self.compare(base, head)?.files)
    }

    fn pull_commits(&self, number: u64) -> Result<Vec<String>> {
        let path = self.path(&format!("/pulls/{number}/commits?per_page=100"));
        Ok(self
            .http
            .get_all(&path)?
            .iter()
            .filter_map(|c| c["sha"].as_str().map(String::from))
            .collect())
    }

    fn compare(&self, base: &str, head: &str) -> Result<Compare> {
        let mut status = String::new();
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
            if page == 1 {
                status = v["status"].as_str().unwrap_or_default().to_string();
            }
            let files = v["files"].as_array().cloned().unwrap_or_default();
            let short = files.len() < 100;
            all.extend(files);
            if short {
                break;
            }
        }
        Ok(Compare {
            status,
            files: self.parse_all("compared file", all)?,
        })
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

    fn status_history(&self, sha: &str) -> Result<Vec<Status>> {
        let path = self.path(&format!("/commits/{}/statuses?per_page=100", enc(sha)));
        self.parse_all("status", self.http.get_all(&path)?)
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
            Some(first) => {
                let why = first["message"].as_str().unwrap_or("unknown GraphQL error");
                let fix = if http::auto_merge_off(why) {
                    http::AUTO_MERGE_OFF_FIX
                } else {
                    "enable 'Allow auto-merge' in the repository settings and require status checks on the base branch"
                };
                Err(BosunError::error(
                    format!("GitHub refused auto-merge: {why}"),
                    fix,
                ))
            }
            None => Ok(()),
        }
    }

    fn enable_auto_merge_rest(&self, number: u64, method: &str) -> Result<()> {
        if !["merge", "squash", "rebase"].contains(&method) {
            return Err(BosunError::error(
                format!("merge method {method:?} is not one GitHub accepts"),
                "use merge, squash or rebase",
            ));
        }
        let path = self.path(&format!("/pulls/{number}/ccr/auto_merge"));
        self.http
            .send_json("PUT", &path, &json!({"merge_method": method}))?;
        Ok(())
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

    fn exists(&self, rest: &str) -> Result<Option<bool>> {
        Ok(match self.http.get_json_or_forbidden(&self.path(rest))? {
            Ok(found) => Some(found.is_some()),
            Err(()) => None,
        })
    }
}
