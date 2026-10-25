//! A stateful fake GitHub on 127.0.0.1 (tiny_http). Lists page at 2 items with absolute `Link`
//! URLs; every request needs an `Authorization` header; every request is recorded.
#![allow(dead_code)]

use serde_json::{Value, json};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

pub const PAGE_SIZE: usize = 2;
const TS: &str = "2026-10-22T10:00:00Z";

#[derive(Debug, Clone)]
pub struct FakeIssue {
    pub number: u64,
    pub title: String,
    pub body: String,
    pub state: String,
    /// Left out of the issue list (GET by number still works): GitHub's list lags.
    pub hidden: bool,
    pub labels: Vec<String>,
    pub is_pr: bool,
    pub user: String,
    pub closed_at: Option<String>,
    pub state_reason: Option<String>,
    pub updated_at: String,
    /// Slug of the app the issue was written via (`performed_via_github_app`).
    pub via_app: Option<String>,
}

#[derive(Debug, Clone)]
pub struct FakeComment {
    pub id: u64,
    pub issue: u64,
    pub body: String,
    pub user: String,
    pub created_at: String,
    pub updated_at: String,
    /// `None`: a plain user token (`null`); `Some(slug)`: written via that app.
    pub via_app: Option<String>,
    /// Leave `performed_via_github_app` out of the JSON.
    pub no_app_field: bool,
    pub association: String,
}

/// How the fake serves a job log.
#[derive(Debug, Clone, Default)]
pub enum LogMode {
    /// The text itself.
    #[default]
    Direct,
    /// A 302 to this URL, as real GitHub does.
    Redirect(String),
}

/// Everything the fake knows.
#[derive(Default)]
pub struct State {
    /// Issue numbers that GET answers with 410 (deleted).
    pub gone: Vec<u64>,
    pub issues: Vec<FakeIssue>,
    pub comments: Vec<FakeComment>,
    pub labels: Vec<(String, String)>,
    pub variables: HashMap<String, String>,
    pub pulls: Vec<Value>,
    pub files: HashMap<u64, Vec<Value>>,
    pub statuses: HashMap<String, Vec<Value>>,
    pub check_runs: HashMap<String, Vec<Value>>,
    pub contents: HashMap<String, String>,
    pub reviews: HashMap<u64, Vec<Value>>,
    /// `base...head` -> files of the comparison.
    pub compares: HashMap<String, Vec<Value>>,
    pub runs: Vec<Value>,
    pub jobs: Vec<Value>,
    pub logs: HashMap<u64, String>,
    pub log_mode: LogMode,
    /// Pull number -> merge method of its enabled auto-merge.
    pub auto_merge: HashMap<u64, String>,
    /// Answers every GraphQL call with this error.
    pub graphql_error: Option<String>,
    /// Answers every GraphQL call with this HTTP status and message.
    pub graphql_status: Option<(u16, String)>,
    pub log: Vec<Logged>,
    /// Extra fields of `GET /repos/o/r` (default branch main, auto-merge off).
    pub repo: serde_json::Map<String, Value>,
    /// Tag of the latest release, if any.
    pub release: Option<String>,
    /// (status, remaining) answered before any routing.
    pub fail: Option<(u16, u32)>,
    /// Repository permission by login; a login not listed has `write`.
    pub permissions: HashMap<String, String>,
    /// (path suffix, status) answered for every matching request.
    pub fail_paths: Vec<(String, u16)>,
    /// `fail_paths` answers carry the request's Authorization header as the message.
    pub echo_auth: bool,
    /// Scripted mutations; see `FakeGithub::after_requests`.
    pub hooks: Vec<Hook>,
    /// `base...head` -> `status` of the comparison (default `ahead`).
    pub compare_statuses: HashMap<String, String>,
    /// PR number -> shas of its commits; unset, a PR has every sha named in `compares`.
    pub pull_commits: HashMap<u64, Vec<String>>,
}

type Mutation = Box<dyn FnOnce(&mut State) + Send>;

/// Runs once, before the request that follows `at` requests whose path ends with `suffix`.
pub struct Hook {
    suffix: String,
    at: usize,
    seen: usize,
    run: Option<Mutation>,
}

#[derive(Debug, Clone)]
pub struct Logged {
    pub method: String,
    /// Path without the query, still percent-encoded.
    pub path: String,
    pub body: Option<String>,
    /// Authorization, Accept, X-GitHub-Api-Version and User-Agent as received.
    pub headers: [Option<String>; 4],
}

pub struct FakeGithub {
    url: String,
    pub state: Arc<Mutex<State>>,
    stop: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
}

impl FakeGithub {
    pub fn start() -> FakeGithub {
        let server = tiny_http::Server::http("127.0.0.1:0").expect("bind fake github");
        let url = format!("http://{}", server.server_addr().to_ip().expect("ip"));
        let state = Arc::new(Mutex::new(State::default()));
        let stop = Arc::new(AtomicBool::new(false));
        let handle = {
            let (state, stop, url) = (state.clone(), stop.clone(), url.clone());
            std::thread::spawn(move || serve(server, state, stop, url))
        };
        FakeGithub {
            url,
            state,
            stop,
            handle: Some(handle),
        }
    }

    pub fn url(&self) -> String {
        self.url.clone()
    }

    pub fn issue(&self, number: u64, title: &str, labels: &[&str], body: &str) {
        let mut s = self.state.lock().unwrap();
        s.issues.push(FakeIssue {
            number,
            title: title.into(),
            body: body.into(),
            state: "open".into(),
            hidden: false,
            labels: labels.iter().map(|l| l.to_string()).collect(),
            is_pr: false,
            user: "octocat".into(),
            closed_at: None,
            state_reason: None,
            updated_at: TS.into(),
            via_app: None,
        });
    }

    /// Close an issue as of `at` (RFC 3339), the way GitHub reports `closed_at`.
    pub fn close_issue(&self, number: u64, at: &str) {
        self.close_issue_as(number, at, "completed");
    }

    /// Close an issue as of `at` with a `state_reason` (`completed`, `not_planned`, `duplicate`).
    pub fn close_issue_as(&self, number: u64, at: &str, reason: &str) {
        let mut s = self.state.lock().unwrap();
        let i = s
            .issues
            .iter_mut()
            .find(|i| i.number == number)
            .expect("issue");
        i.state = "closed".into();
        i.closed_at = Some(at.into());
        i.state_reason = Some(reason.into());
    }

    /// Put a label on an issue.
    pub fn label(&self, number: u64, label: &str) {
        let mut s = self.state.lock().unwrap();
        let i = s
            .issues
            .iter_mut()
            .find(|i| i.number == number)
            .expect("issue");
        i.labels.push(label.into());
    }

    /// Make `number` a deleted issue: out of the list, GET answers 410.
    pub fn gone(&self, number: u64) {
        self.hide_from_list(number);
        self.state.lock().unwrap().gone.push(number);
    }

    /// Leave an issue out of the list while GET by number still returns it.
    pub fn hide_from_list(&self, number: u64) {
        let mut s = self.state.lock().unwrap();
        s.issues
            .iter_mut()
            .find(|i| i.number == number)
            .unwrap()
            .hidden = true;
    }

    /// An entry in the issues list that is a pull request.
    pub fn pull_as_issue(&self, number: u64, title: &str) {
        self.issue(number, title, &[], "");
        self.state.lock().unwrap().issues.last_mut().unwrap().is_pr = true;
    }

    pub fn comment(&self, issue: u64, body: &str) -> u64 {
        add_comment(&mut self.state.lock().unwrap(), issue, body)
    }

    /// An open pull request from `head_ref` (head sha `sha<number>`) into `main`.
    pub fn pull(&self, number: u64, title: &str, head_ref: &str, body: &str) {
        let mut s = self.state.lock().unwrap();
        s.pulls
            .push(pull_json(number, title, head_ref, "main", body));
    }

    /// Set a top-level field of a pull request (`draft`, `merged`, `state`, `labels`, ...).
    pub fn pull_set(&self, number: u64, key: &str, value: Value) {
        let mut s = self.state.lock().unwrap();
        let p = s.pulls.iter_mut().find(|p| p["number"] == number);
        p.unwrap_or_else(|| panic!("no pull {number}"))[key] = value;
    }

    /// What GitHub does on an auto-merge: the pull closes as merged and the issue it closes
    /// (`Closes #N`) closes too.
    pub fn merge_pull(&self, number: u64, closes_issue: u64) {
        self.pull_set(number, "state", json!("closed"));
        self.pull_set(number, "merged", json!(true));
        let mut s = self.state.lock().unwrap();
        if let Some(i) = s.issues.iter_mut().find(|i| i.number == closes_issue) {
            i.state = "closed".into();
        }
    }

    /// Mark a pull request merged, with the sha of its merge commit.
    pub fn pull_merged(&self, number: u64, merge_commit_sha: &str) {
        self.pull_set(number, "merged", json!(true));
        self.pull_set(number, "state", json!("closed"));
        self.pull_set(number, "merge_commit_sha", json!(merge_commit_sha));
    }

    pub fn pull_file(
        &self,
        number: u64,
        filename: &str,
        status: &str,
        additions: u64,
        deletions: u64,
        patch: Option<&str>,
    ) {
        let mut f = json!({"sha": "abc", "filename": filename, "status": status,
            "additions": additions, "deletions": deletions, "changes": additions + deletions});
        if let Some(p) = patch {
            f["patch"] = json!(p);
        }
        let mut s = self.state.lock().unwrap();
        s.files.entry(number).or_default().push(f);
    }

    /// Set `previous_filename` on the last file added to `number`.
    pub fn renamed_from(&self, number: u64, previous: &str) {
        let mut s = self.state.lock().unwrap();
        let last = s.files.get_mut(&number).unwrap().last_mut().unwrap();
        last["previous_filename"] = json!(previous);
    }

    /// A file of the comparison `base...head`.
    pub fn compare_file(&self, base: &str, head: &str, filename: &str, patch: &str) {
        let f = json!({"filename": filename, "status": "modified", "additions": 1,
            "deletions": 0, "patch": patch});
        let mut s = self.state.lock().unwrap();
        s.compares
            .entry(format!("{base}...{head}"))
            .or_default()
            .push(f);
    }

    /// The `status` GitHub reports for the comparison `base...head` (`behind`, `diverged`, ...).
    pub fn compare_status(&self, base: &str, head: &str, status: &str) {
        let mut s = self.state.lock().unwrap();
        s.compare_statuses
            .insert(format!("{base}...{head}"), status.into());
    }

    /// The commits of PR `number` (`GET /pulls/{n}/commits`).
    pub fn pull_commit_shas(&self, number: u64, shas: &[&str]) {
        let mut s = self.state.lock().unwrap();
        s.pull_commits
            .insert(number, shas.iter().map(|x| x.to_string()).collect());
    }

    pub fn review(&self, number: u64, user: &str, state: &str, commit_id: &str) {
        let mut s = self.state.lock().unwrap();
        let list = s.reviews.entry(number).or_default();
        let id = list.len() + 1;
        list.push(json!({"id": id, "user": {"login": user}, "state": state,
            "commit_id": commit_id, "submitted_at": TS}));
    }

    pub fn pull_body(&self, number: u64) -> String {
        let s = self.state.lock().unwrap();
        let p = s.pulls.iter().find(|p| p["number"] == number);
        p.unwrap_or_else(|| panic!("no pull {number}"))["body"]
            .as_str()
            .unwrap_or("")
            .to_string()
    }

    /// A commit status as if posted earlier by CI.
    pub fn status(&self, sha: &str, context: &str, state: &str, description: &str) {
        add_status(
            &mut self.state.lock().unwrap(),
            sha,
            context,
            state,
            description,
        );
    }

    /// A commit status posted by `creator` (statuses default to `github-actions[bot]`).
    pub fn status_by(
        &self,
        sha: &str,
        context: &str,
        state: &str,
        description: &str,
        creator: &str,
    ) {
        let mut s = self.state.lock().unwrap();
        add_status(&mut s, sha, context, state, description);
        let last = s.statuses.get_mut(sha).unwrap().last_mut().unwrap();
        last["creator"] = json!({"login": creator});
    }

    /// Every status ever posted to `sha`, oldest first.
    pub fn statuses_of(&self, sha: &str) -> Vec<Value> {
        let s = self.state.lock().unwrap();
        s.statuses.get(sha).cloned().unwrap_or_default()
    }

    pub fn check_run(&self, sha: &str, name: &str, status: &str, conclusion: Option<&str>) {
        let mut s = self.state.lock().unwrap();
        let list = s.check_runs.entry(sha.into()).or_default();
        let id = list.len() + 1;
        list.push(
            json!({"id": id, "name": name, "status": status, "conclusion": conclusion,
            "head_sha": sha, "app": {"slug": "github-actions"}}),
        );
    }

    /// A check run created by the GitHub App `app` (check runs default to `github-actions`).
    pub fn check_run_by(&self, sha: &str, name: &str, conclusion: &str, app: &str) {
        self.check_run(sha, name, "completed", Some(conclusion));
        let mut s = self.state.lock().unwrap();
        let last = s.check_runs.get_mut(sha).unwrap().last_mut().unwrap();
        last["app"] = json!({"slug": app});
    }

    /// Answer failing requests (`fail_path`) with a message that echoes the Authorization header,
    /// as a hostile server would.
    pub fn echo_auth_on_fail(&self) {
        self.state.lock().unwrap().echo_auth = true;
    }

    /// A file at a ref; the contents API serves it base64-encoded.
    pub fn content(&self, path: &str, git_ref: &str, text: &str) {
        let mut s = self.state.lock().unwrap();
        s.contents.insert(format!("{git_ref}:{path}"), text.into());
    }

    /// A workflow run of `sha` with one job and its log.
    pub fn run_with_job(&self, sha: &str, job_name: &str, conclusion: &str, log: &str) {
        let mut s = self.state.lock().unwrap();
        let n = s.runs.len() as u64 + 1;
        let (run_id, job_id) = (1000 + n, 5000 + n);
        s.runs
            .push(json!({"id": run_id, "head_sha": sha, "status": "completed",
            "conclusion": conclusion}));
        s.jobs.push(json!({"id": job_id, "run_id": run_id, "name": job_name,
            "status": "completed", "conclusion": conclusion,
            "html_url": format!("https://github.test/acme/widget/actions/runs/{run_id}/job/{job_id}")}));
        s.logs.insert(job_id, log.into());
    }

    pub fn log_mode(&self, mode: LogMode) {
        self.state.lock().unwrap().log_mode = mode;
    }

    /// The merge method of the pull's enabled auto-merge, if any.
    pub fn auto_merge_enabled(&self, number: u64) -> Option<String> {
        self.state.lock().unwrap().auto_merge.get(&number).cloned()
    }

    pub fn graphql_status(&self, status: u16, message: &str) {
        self.state.lock().unwrap().graphql_status = Some((status, message.into()));
    }

    pub fn graphql_error(&self, message: &str) {
        self.state.lock().unwrap().graphql_error = Some(message.into());
    }

    /// Set a field of the repository document, for example `allow_auto_merge`.
    pub fn repo_set(&self, key: &str, value: Value) {
        self.state.lock().unwrap().repo.insert(key.into(), value);
    }

    pub fn release(&self, tag: &str) {
        self.state.lock().unwrap().release = Some(tag.into());
    }

    /// The author of an issue or pull entry.
    pub fn issue_by(&self, number: u64, user: &str) {
        let mut s = self.state.lock().unwrap();
        s.issues
            .iter_mut()
            .find(|i| i.number == number)
            .unwrap()
            .user = user.into();
    }

    /// An issue or pull entry written through the GitHub App `slug`.
    pub fn issue_via_app(&self, number: u64, slug: &str) {
        let mut s = self.state.lock().unwrap();
        s.issues
            .iter_mut()
            .find(|i| i.number == number)
            .unwrap()
            .via_app = Some(slug.into());
    }

    /// A comment written by `user`.
    pub fn comment_by(&self, issue: u64, user: &str, body: &str) {
        let mut s = self.state.lock().unwrap();
        let id = add_comment(&mut s, issue, body);
        s.comments.iter_mut().find(|c| c.id == id).unwrap().user = user.into();
    }

    /// A comment written by `user` at `at` (RFC 3339); the issue's `updated_at` moves up to it.
    pub fn comment_at(&self, issue: u64, user: &str, body: &str, at: &str) -> u64 {
        let mut s = self.state.lock().unwrap();
        let id = add_comment(&mut s, issue, body);
        let c = s.comments.iter_mut().find(|c| c.id == id).unwrap();
        c.user = user.into();
        c.created_at = at.into();
        c.updated_at = at.into();
        let i = s.issues.iter_mut().find(|i| i.number == issue).unwrap();
        // `updated_at` starts at the sentinel `TS` (the fake's "now"), which any explicit time
        // replaces, even an earlier one; after that it only moves forward.
        if i.updated_at.as_str() < at || i.updated_at == TS {
            i.updated_at = at.into();
        }
        id
    }

    /// Set the `updated_at` of an issue.
    pub fn issue_updated(&self, number: u64, at: &str) {
        let mut s = self.state.lock().unwrap();
        s.issues
            .iter_mut()
            .find(|i| i.number == number)
            .unwrap()
            .updated_at = at.into();
    }

    /// Change the last comment: `f` gets it mutably (app, association, edits).
    pub fn last_comment(&self, f: impl FnOnce(&mut FakeComment)) {
        f(self.state.lock().unwrap().comments.last_mut().unwrap());
    }

    /// The repository permission `login` has (default `write`).
    pub fn permission(&self, login: &str, permission: &str) {
        self.state
            .lock()
            .unwrap()
            .permissions
            .insert(login.into(), permission.into());
    }

    pub fn variable(&self, name: &str, value: &str) {
        self.state
            .lock()
            .unwrap()
            .variables
            .insert(name.into(), value.into());
    }

    /// Answer the next `times` requests with `status` and a JSON message.
    pub fn fail_next(&self, status: u16, times: u32) {
        self.state.lock().unwrap().fail = Some((status, times));
    }

    /// Answer every request whose path ends with `suffix` with `status`.
    pub fn fail_path(&self, suffix: &str, status: u16) {
        let mut s = self.state.lock().unwrap();
        s.fail_paths.push((suffix.into(), status));
    }

    /// Mutate the state once, after `n` requests to a path ending with `suffix` were served:
    /// request `n + 1` already sees the change.
    pub fn after_requests(
        &self,
        suffix: &str,
        n: usize,
        f: impl FnOnce(&mut State) + Send + 'static,
    ) {
        self.state.lock().unwrap().hooks.push(Hook {
            suffix: suffix.into(),
            at: n,
            seen: 0,
            run: Some(Box::new(f)),
        });
    }

    pub fn requests(&self) -> Vec<Logged> {
        self.state.lock().unwrap().log.clone()
    }

    pub fn issue_state(&self, number: u64) -> FakeIssue {
        let s = self.state.lock().unwrap();
        s.issues
            .iter()
            .find(|i| i.number == number)
            .cloned()
            .unwrap_or_else(|| panic!("no issue {number}"))
    }

    pub fn labels_of(&self, number: u64) -> Vec<String> {
        self.issue_state(number).labels
    }

    pub fn comments_of(&self, issue: u64) -> Vec<FakeComment> {
        let s = self.state.lock().unwrap();
        s.comments
            .iter()
            .filter(|c| c.issue == issue)
            .cloned()
            .collect()
    }

    pub fn label_names(&self) -> Vec<String> {
        self.state
            .lock()
            .unwrap()
            .labels
            .iter()
            .map(|l| l.0.clone())
            .collect()
    }
}

impl Drop for FakeGithub {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}

pub fn add_comment(s: &mut State, issue: u64, body: &str) -> u64 {
    let id = s.comments.iter().map(|c| c.id).max().unwrap_or(100) + 1;
    s.comments.push(FakeComment {
        id,
        issue,
        body: body.into(),
        user: "octocat".into(),
        created_at: TS.into(),
        updated_at: TS.into(),
        via_app: None,
        no_app_field: false,
        association: "MEMBER".into(),
    });
    id
}

fn issue_json(i: &FakeIssue) -> Value {
    let labels: Vec<Value> = i
        .labels
        .iter()
        .map(|n| json!({"name": n, "color": "ededed"}))
        .collect();
    let mut v = json!({"number": i.number, "title": i.title, "body": i.body, "state": i.state,
        "labels": labels, "user": {"login": i.user}, "closed_at": i.closed_at,
        "state_reason": i.state_reason, "updated_at": i.updated_at});
    if let Some(slug) = &i.via_app {
        v["performed_via_github_app"] = json!({"slug": slug});
    }
    if i.is_pr {
        v["pull_request"] = json!({"url": "https://example.test/pull"});
    }
    v
}

fn pull_json(number: u64, title: &str, head_ref: &str, base_ref: &str, body: &str) -> Value {
    json!({"number": number, "node_id": format!("PR_node{number}"), "title": title,
        "body": body, "state": "open", "merged": false, "draft": false, "labels": [],
        "user": {"login": "octocat"},
        "head": {"sha": format!("sha{number}"), "ref": head_ref, "label": format!("acme:{head_ref}")},
        "base": {"sha": "base0000", "ref": base_ref}})
}

fn add_status(s: &mut State, sha: &str, context: &str, state: &str, description: &str) -> Value {
    let list = s.statuses.entry(sha.into()).or_default();
    let v = json!({"id": list.len() + 1, "context": context, "state": state,
        "description": description, "created_at": TS,
        "creator": {"login": "github-actions[bot]"}});
    list.push(v.clone());
    v
}

/// Wrap a page's array as GitHub does for run, job and check-run lists.
fn wrapped(mut reply: Reply, key: &str, total: usize) -> Reply {
    reply.1 = json!({"total_count": total, key: reply.1});
    reply
}

fn b64_wrapped(text: &str) -> String {
    const A: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut flat = String::new();
    for c in text.as_bytes().chunks(3) {
        let n = c
            .iter()
            .enumerate()
            .fold(0u32, |a, (i, b)| a | (*b as u32) << (16 - 8 * i));
        for i in 0..4 {
            flat.push(if i <= c.len() {
                A[(n >> (18 - 6 * i) & 63) as usize] as char
            } else {
                '='
            });
        }
    }
    flat.as_bytes()
        .chunks(60)
        .map(|l| format!("{}\n", std::str::from_utf8(l).unwrap()))
        .collect()
}

fn pull_pos(s: &State, n: &str) -> Option<usize> {
    let n: u64 = n.parse().ok()?;
    s.pulls.iter().position(|p| p["number"] == n)
}

fn comment_json(c: &FakeComment) -> Value {
    let mut v = json!({"id": c.id, "body": c.body, "user": {"login": c.user},
        "created_at": c.created_at, "updated_at": c.updated_at,
        "author_association": c.association,
        "issue_url": format!("https://api.github.com/repos/o/r/issues/{}", c.issue),
        "performed_via_github_app": c.via_app.as_ref().map(|s| json!({"slug": s, "id": 1}))});
    if c.no_app_field {
        v.as_object_mut()
            .unwrap()
            .remove("performed_via_github_app");
    }
    v
}

fn dec(s: &str) -> String {
    let b = s.as_bytes();
    let (mut out, mut i) = (Vec::new(), 0);
    while i < b.len() {
        if b[i] == b'%'
            && let Some(v) = s
                .get(i + 1..i + 3)
                .and_then(|h| u8::from_str_radix(h, 16).ok())
        {
            out.push(v);
            i += 3;
            continue;
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

type Reply = (u16, Value, Vec<(String, String)>);

fn ok(v: Value) -> Reply {
    (200, v, vec![])
}

fn msg(status: u16, m: &str) -> Reply {
    (status, json!({"message": m}), vec![])
}

/// One page of `items` plus a `Link: rel="next"` header with an absolute URL.
fn page(
    items: Vec<Value>,
    query: &HashMap<String, String>,
    base: &str,
    path: &str,
    raw_query: &str,
) -> Reply {
    let n: usize = query.get("page").and_then(|p| p.parse().ok()).unwrap_or(1);
    let chunk: Vec<Value> = items
        .iter()
        .skip((n - 1) * PAGE_SIZE)
        .take(PAGE_SIZE)
        .cloned()
        .collect();
    let mut headers = vec![];
    if n * PAGE_SIZE < items.len() {
        let rest: Vec<&str> = raw_query
            .split('&')
            .filter(|p| !p.is_empty() && !p.starts_with("page="))
            .collect();
        let q = [rest, vec![&format!("page={}", n + 1)[..]]]
            .concat()
            .join("&");
        headers.push((
            "Link".to_string(),
            format!("<{base}{path}?{q}>; rel=\"next\""),
        ));
    }
    (200, Value::Array(chunk), headers)
}

fn route(
    s: &mut State,
    method: &str,
    path: &str,
    raw_query: &str,
    body: &Value,
    base: &str,
) -> Reply {
    let query: HashMap<String, String> = raw_query
        .split('&')
        .filter_map(|p| p.split_once('='))
        .map(|(k, v)| (k.to_string(), dec(v)))
        .collect();
    let seg: Vec<String> = path.trim_start_matches('/').split('/').map(dec).collect();
    let seg: Vec<&str> = seg.iter().map(String::as_str).collect();
    if method == "POST" && seg == ["graphql"] {
        if let Some((code, m)) = s.graphql_status.clone() {
            return msg(code, &m);
        }
        return graphql(s, body);
    }
    let ["repos", _, _, rest @ ..] = seg.as_slice() else {
        return msg(404, "Not Found");
    };
    let find = |s: &State, n: &str| {
        n.parse::<u64>()
            .ok()
            .and_then(|n| s.issues.iter().position(|i| i.number == n))
    };
    match (method, rest) {
        ("GET", ["issues"]) if query.get("sort").is_some_and(|s| s == "updated") => {
            // `fail_path("recent", status)` fails only this updated-since listing.
            if let Some((_, code)) = s.fail_paths.iter().find(|f| f.0 == "recent") {
                return msg(*code, "fake recent failure");
            }
            let since = query.get("since").cloned().unwrap_or_default();
            let per: usize = query
                .get("per_page")
                .and_then(|p| p.parse().ok())
                .unwrap_or(30);
            let mut hits: Vec<&FakeIssue> = s
                .issues
                .iter()
                .filter(|i| !i.hidden && !i.is_pr && i.updated_at >= since)
                .collect();
            hits.sort_by(|a, b| (&b.updated_at, b.number).cmp(&(&a.updated_at, a.number)));
            // Paged by `per_page` (not the fake's PAGE_SIZE), with a Link header.
            let n: usize = query.get("page").and_then(|p| p.parse().ok()).unwrap_or(1);
            let items: Vec<Value> = hits
                .iter()
                .skip((n - 1) * per)
                .take(per)
                .map(|i| issue_json(i))
                .collect();
            let mut r = ok(Value::Array(items));
            if n * per < hits.len() {
                let rest: Vec<&str> = raw_query
                    .split('&')
                    .filter(|p| !p.is_empty() && !p.starts_with("page="))
                    .collect();
                let q = [rest, vec![&format!("page={}", n + 1)[..]]]
                    .concat()
                    .join("&");
                r.2.push((
                    "Link".to_string(),
                    format!("<{base}{path}?{q}>; rel=\"next\""),
                ));
            }
            r
        }
        ("GET", ["issues"]) => {
            let want: Vec<&str> = query
                .get("labels")
                .map(|l| l.split(',').collect())
                .unwrap_or_default();
            let state = query.get("state").map(String::as_str).unwrap_or("open");
            let items = s
                .issues
                .iter()
                .filter(|i| {
                    !i.hidden
                        && (state == "all" || i.state == state)
                        && want.iter().all(|w| i.labels.iter().any(|l| l == w))
                })
                .map(issue_json)
                .collect();
            page(items, &query, base, path, raw_query)
        }
        ("POST", ["issues"]) => {
            let number = s.issues.iter().map(|i| i.number).max().unwrap_or(0) + 1;
            let labels = body["labels"]
                .as_array()
                .map(|a| {
                    a.iter()
                        .filter_map(|l| l.as_str().map(String::from))
                        .collect()
                })
                .unwrap_or_default();
            let issue = FakeIssue {
                number,
                title: body["title"].as_str().unwrap_or("").into(),
                body: body["body"].as_str().unwrap_or("").into(),
                state: "open".into(),
                hidden: false,
                labels,
                is_pr: false,
                user: "octocat".into(),
                closed_at: None,
                state_reason: None,
                updated_at: TS.into(),
                via_app: None,
            };
            let v = issue_json(&issue);
            s.issues.push(issue);
            (201, v, vec![])
        }
        ("GET", ["issues", n]) if n.parse().is_ok_and(|n| s.gone.contains(&n)) => msg(410, "Gone"),
        ("GET", ["issues", n]) => find(s, n)
            .map(|p| ok(issue_json(&s.issues[p])))
            .unwrap_or_else(|| msg(404, "Not Found")),
        ("PATCH", ["issues", n]) => {
            let Some(p) = find(s, n) else {
                return msg(404, "Not Found");
            };
            let i = &mut s.issues[p];
            if let Some(t) = body["title"].as_str() {
                i.title = t.into();
            }
            if let Some(b) = body["body"].as_str() {
                i.body = b.into();
            }
            if let Some(st) = body["state"].as_str() {
                i.state = st.into();
                i.closed_at = (st == "closed").then(|| TS.to_string());
                i.state_reason = (st == "closed").then(|| {
                    body["state_reason"]
                        .as_str()
                        .unwrap_or("completed")
                        .to_string()
                });
            }
            if let Some(r) = body["state_reason"].as_str() {
                i.state_reason = Some(r.into());
            }
            ok(issue_json(i))
        }
        ("POST", ["issues", n, "labels"]) => {
            let Some(p) = find(s, n) else {
                // A pull request is an issue on GitHub: label it in the pulls list.
                let Some(q) = pull_pos(s, n) else {
                    return msg(404, "Not Found");
                };
                for l in body["labels"].as_array().into_iter().flatten() {
                    let want = l.as_str().unwrap_or("");
                    let list = s.pulls[q]["labels"].as_array_mut().unwrap();
                    if !list.iter().any(|x| x["name"] == want) {
                        list.push(json!({"name": want}));
                    }
                }
                return ok(s.pulls[q]["labels"].clone());
            };
            for l in body["labels"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
            {
                if !s.issues[p].labels.iter().any(|x| x == l) {
                    s.issues[p].labels.push(l.into());
                }
            }
            ok(Value::Array(
                s.issues[p]
                    .labels
                    .iter()
                    .map(|n| json!({"name": n}))
                    .collect(),
            ))
        }
        ("PUT", ["pulls", n, "ccr", "auto_merge"]) => {
            let method = body["merge_method"].as_str().unwrap_or("");
            if !["merge", "squash", "rebase"].contains(&method) {
                return msg(400, "merge_method must be one of merge, squash, rebase.");
            }
            let Some(n) = n
                .parse::<u64>()
                .ok()
                .filter(|n| s.pulls.iter().any(|p| p["number"].as_u64() == Some(*n)))
            else {
                return msg(404, "Not Found");
            };
            s.auto_merge.insert(n, method.to_uppercase());
            ok(json!({}))
        }
        ("DELETE", ["issues", n, "labels", name]) => {
            let Some(p) = find(s, n) else {
                return msg(404, "Not Found");
            };
            let before = s.issues[p].labels.len();
            s.issues[p].labels.retain(|l| l != name);
            if s.issues[p].labels.len() == before {
                return msg(404, "Label does not exist");
            }
            ok(Value::Array(vec![]))
        }
        ("GET", ["labels", name]) => match s.labels.iter().find(|l| l.0 == *name) {
            Some(l) => ok(json!({"name": l.0, "color": l.1})),
            None => msg(404, "Not Found"),
        },
        ("POST", ["labels"]) => {
            let name = body["name"].as_str().unwrap_or("").to_string();
            if s.labels.iter().any(|l| l.0 == name) {
                return (
                    422,
                    json!({"message": "Validation Failed", "errors": [{"code": "already_exists"}]}),
                    vec![],
                );
            }
            s.labels
                .push((name.clone(), body["color"].as_str().unwrap_or("").into()));
            (201, json!({"name": name, "color": body["color"]}), vec![])
        }
        ("GET", ["issues", n, "comments"]) => {
            let n: u64 = n.parse().unwrap_or(0);
            let items = s
                .comments
                .iter()
                .filter(|c| c.issue == n)
                .map(comment_json)
                .collect();
            page(items, &query, base, path, raw_query)
        }
        ("POST", ["issues", n, "comments"]) => {
            let Some(p) = find(s, n) else {
                return msg(404, "Not Found");
            };
            let number = s.issues[p].number;
            let id = add_comment(s, number, body["body"].as_str().unwrap_or(""));
            (
                201,
                comment_json(s.comments.iter().find(|c| c.id == id).unwrap()),
                vec![],
            )
        }
        ("GET", ["collaborators", user, "permission"]) => {
            let p = s.permissions.get(*user).cloned().unwrap_or("write".into());
            ok(json!({"permission": p, "role_name": p}))
        }
        ("GET", ["issues", "comments", id]) => {
            let id: u64 = id.parse().unwrap_or(0);
            match s.comments.iter().find(|c| c.id == id) {
                Some(c) => ok(comment_json(c)),
                None => msg(404, "Not Found"),
            }
        }
        ("PATCH", ["issues", "comments", id]) => {
            let id: u64 = id.parse().unwrap_or(0);
            match s.comments.iter_mut().find(|c| c.id == id) {
                Some(c) => {
                    c.body = body["body"].as_str().unwrap_or("").into();
                    c.updated_at = "2026-10-22T11:00:00Z".into();
                    ok(comment_json(c))
                }
                None => msg(404, "Not Found"),
            }
        }
        ("GET", ["pulls"]) => {
            let state = query.get("state").map(String::as_str).unwrap_or("open");
            let head = query.get("head").map(String::as_str);
            let items = s
                .pulls
                .iter()
                .filter(|p| {
                    (state == "all" || p["state"] == state)
                        && head.is_none_or(|h| p["head"]["label"] == h)
                })
                .map(|p| {
                    // lists do not carry `merged`
                    let mut p = p.clone();
                    p.as_object_mut().unwrap().remove("merged");
                    p
                })
                .collect();
            page(items, &query, base, path, raw_query)
        }
        ("POST", ["pulls"]) => {
            let number = s
                .pulls
                .iter()
                .filter_map(|p| p["number"].as_u64())
                .chain(s.issues.iter().map(|i| i.number))
                .max()
                .unwrap_or(0)
                + 1;
            let head = body["head"].as_str().unwrap_or("");
            let branch = head.split_once(':').map_or(head, |h| h.1);
            let p = pull_json(
                number,
                body["title"].as_str().unwrap_or(""),
                branch,
                body["base"].as_str().unwrap_or("main"),
                body["body"].as_str().unwrap_or(""),
            );
            s.pulls.push(p.clone());
            (201, p, vec![])
        }
        ("GET", ["pulls", n]) => match pull_pos(s, n) {
            Some(p) => ok(s.pulls[p].clone()),
            None => msg(404, "Not Found"),
        },
        ("PATCH", ["pulls", n]) => match pull_pos(s, n) {
            Some(p) => {
                if let Some(b) = body["body"].as_str() {
                    s.pulls[p]["body"] = json!(b);
                }
                ok(s.pulls[p].clone())
            }
            None => msg(404, "Not Found"),
        },
        ("GET", ["pulls", n, "files"]) => {
            let items = n.parse().ok().and_then(|n: u64| s.files.get(&n)).cloned();
            page(items.unwrap_or_default(), &query, base, path, raw_query)
        }
        ("GET", ["pulls", n, "commits"]) => {
            let n: u64 = n.parse().unwrap_or(0);
            let shas: Vec<String> = s.pull_commits.get(&n).cloned().unwrap_or_else(|| {
                s.compares
                    .keys()
                    .flat_map(|k| k.split("...").map(String::from))
                    .collect()
            });
            let items = shas.iter().map(|x| json!({"sha": x})).collect();
            page(items, &query, base, path, raw_query)
        }
        ("GET", ["compare", range]) => match s.compares.get(*range) {
            Some(files) => {
                let num =
                    |k: &str, d: usize| query.get(k).and_then(|p| p.parse().ok()).unwrap_or(d);
                let (n, per) = (num("page", 1), num("per_page", 300));
                let chunk: Vec<&Value> = files.iter().skip((n - 1) * per).take(per).collect();
                let status = s
                    .compare_statuses
                    .get(*range)
                    .map_or("ahead", |x| x.as_str());
                ok(json!({"status": status, "files": chunk}))
            }
            None => msg(404, "Not Found"),
        },
        ("GET", ["pulls", n, "reviews"]) => {
            let items = n.parse().ok().and_then(|n: u64| s.reviews.get(&n)).cloned();
            page(items.unwrap_or_default(), &query, base, path, raw_query)
        }
        ("POST", ["statuses", sha]) => {
            let f = |k: &str| body[k].as_str().unwrap_or("").to_string();
            let mut v = add_status(s, sha, &f("context"), &f("state"), &f("description"));
            v["target_url"] = body["target_url"].clone();
            s.statuses.get_mut(*sha).unwrap().last_mut().unwrap()["target_url"] =
                body["target_url"].clone();
            (201, v, vec![])
        }
        ("GET", ["commits", sha, "status"]) => {
            let mut seen = std::collections::HashSet::new();
            let newest: Vec<Value> = s
                .statuses
                .get(*sha)
                .into_iter()
                .flatten()
                .rev()
                .filter(|v| seen.insert(v["context"].as_str().unwrap_or("").to_string()))
                .cloned()
                .collect();
            ok(
                json!({"sha": sha, "state": "pending", "total_count": newest.len(),
                "statuses": newest}),
            )
        }
        ("GET", ["commits", sha, "statuses"]) => {
            let items = s
                .statuses
                .get(*sha)
                .into_iter()
                .flatten()
                .rev()
                .cloned()
                .collect();
            page(items, &query, base, path, raw_query)
        }
        ("GET", ["commits", sha, "check-runs"]) => {
            let items = s.check_runs.get(*sha).cloned().unwrap_or_default();
            let total = items.len();
            wrapped(
                page(items, &query, base, path, raw_query),
                "check_runs",
                total,
            )
        }
        ("GET", ["contents", file @ ..]) => {
            let file = file.join("/");
            let git_ref = query.get("ref").map(String::as_str).unwrap_or("main");
            if let Some(text) = s.contents.get(&format!("{git_ref}:{file}")) {
                return ok(json!({"type": "file", "encoding": "base64", "name": file,
                    "path": file, "size": text.len(), "content": b64_wrapped(text)}));
            }
            let dir = format!("{git_ref}:{file}/");
            let mut entries: Vec<Value> = s
                .contents
                .keys()
                .filter(|k| k.starts_with(&dir))
                .map(|k| json!({"type": "file", "path": &k[git_ref.len() + 1..]}))
                .collect();
            entries.sort_by_key(|e| e["path"].to_string());
            if entries.is_empty() {
                msg(404, "Not Found")
            } else {
                ok(Value::Array(entries))
            }
        }
        ("GET", ["actions", "runs"]) => {
            let sha = query.get("head_sha").map(String::as_str).unwrap_or("");
            let items: Vec<Value> = s
                .runs
                .iter()
                .filter(|r| r["head_sha"] == sha)
                .cloned()
                .collect();
            let total = items.len();
            wrapped(
                page(items, &query, base, path, raw_query),
                "workflow_runs",
                total,
            )
        }
        ("GET", ["actions", "runs", id, "jobs"]) => {
            let id: u64 = id.parse().unwrap_or(0);
            let items: Vec<Value> = s
                .jobs
                .iter()
                .filter(|j| j["run_id"] == id)
                .cloned()
                .collect();
            let total = items.len();
            wrapped(page(items, &query, base, path, raw_query), "jobs", total)
        }
        ("GET", ["actions", "jobs", id, "logs"]) => {
            let id: u64 = id.parse().unwrap_or(0);
            let Some(text) = s.logs.get(&id) else {
                return msg(404, "Not Found");
            };
            match &s.log_mode {
                LogMode::Direct => ok(json!(text)),
                LogMode::Redirect(url) => {
                    (302, Value::Null, vec![("Location".into(), url.clone())])
                }
            }
        }
        ("GET", []) => {
            let mut v = json!({"default_branch": "main", "allow_auto_merge": false});
            for (k, x) in &s.repo {
                v[k] = x.clone();
            }
            ok(v)
        }
        ("GET", ["releases", "latest"]) => match &s.release {
            Some(t) => ok(json!({"tag_name": t})),
            None => msg(404, "Not Found"),
        },
        ("GET", ["actions", "variables", name]) => match s.variables.get(*name) {
            Some(v) => ok(json!({"name": name, "value": v})),
            None => msg(404, "Not Found"),
        },
        _ => msg(404, "Not Found"),
    }
}

fn graphql(s: &mut State, body: &Value) -> Reply {
    let error = |m: &str| ok(json!({"data": null, "errors": [{"message": m}]}));
    if let Some(m) = s.graphql_error.clone() {
        return error(&m);
    }
    if !body["query"]
        .as_str()
        .unwrap_or("")
        .contains("enablePullRequestAutoMerge")
    {
        return error("unsupported query");
    }
    let id = body["variables"]["id"].as_str().unwrap_or("");
    let method = body["variables"]["method"].as_str().unwrap_or("");
    let Some(n) = s
        .pulls
        .iter()
        .find(|p| p["node_id"] == id)
        .and_then(|p| p["number"].as_u64())
    else {
        return error(&format!(
            "Could not resolve to a node with the global id of '{id}'"
        ));
    };
    s.auto_merge.insert(n, method.into());
    ok(json!({"data": {"enablePullRequestAutoMerge": {"pullRequest": {"number": n}}}}))
}

fn serve(server: tiny_http::Server, state: Arc<Mutex<State>>, stop: Arc<AtomicBool>, base: String) {
    while !stop.load(Ordering::SeqCst) {
        let mut req = match server.recv_timeout(Duration::from_millis(25)) {
            Ok(Some(r)) => r,
            Ok(None) => continue,
            Err(_) => break,
        };
        let method = req.method().as_str().to_string();
        let full = req.url().to_string();
        let (path, raw_query) = full.split_once('?').unwrap_or((&full, ""));
        let get = |n: &str| {
            req.headers()
                .iter()
                .find(|h| h.field.as_str().as_str().eq_ignore_ascii_case(n))
                .map(|h| h.value.as_str().to_string())
        };
        let headers = [
            "Authorization",
            "Accept",
            "X-GitHub-Api-Version",
            "User-Agent",
        ]
        .map(get);
        let authed = headers[0].is_some();
        let auth = headers[0].clone().unwrap_or_default();
        let mut raw = String::new();
        let _ = std::io::Read::read_to_string(req.as_reader(), &mut raw);
        let body: Value = serde_json::from_str(&raw).unwrap_or(Value::Null);
        let mut s = state.lock().unwrap();
        s.log.push(Logged {
            method: method.clone(),
            path: path.into(),
            body: (!raw.is_empty()).then(|| raw.clone()),
            headers,
        });
        let (status, value, headers) = if !authed {
            msg(401, "Requires authentication")
        } else if let Some((code, left)) = s.fail.filter(|f| f.1 > 0) {
            s.fail = Some((code, left - 1));
            msg(code, "fake failure")
        } else if let Some((_, code)) = s.fail_paths.iter().find(|f| path.ends_with(&f.0)) {
            let code = *code;
            if s.echo_auth {
                msg(code, auth.as_str())
            } else {
                msg(code, "fake path failure")
            }
        } else {
            let mut hooks = std::mem::take(&mut s.hooks);
            for h in hooks.iter_mut().filter(|h| path.ends_with(&h.suffix)) {
                if h.seen == h.at
                    && let Some(f) = h.run.take()
                {
                    f(&mut s);
                }
                h.seen += 1;
            }
            hooks.append(&mut s.hooks);
            s.hooks = hooks;
            route(&mut s, &method, path, raw_query, &body, &base)
        };
        drop(s);
        let text_body = value.as_str().map(str::to_string);
        let mut resp = tiny_http::Response::from_data(match (&text_body, &value) {
            _ if status == 204 => vec![],
            (_, Value::Null) if status == 302 => vec![],
            (Some(t), _) => t.clone().into_bytes(),
            _ => value.to_string().into_bytes(),
        })
        .with_status_code(status);
        for (k, v) in std::iter::once(("Content-Type".to_string(), "application/json".to_string()))
            .chain(headers)
        {
            resp.add_header(tiny_http::Header::from_bytes(k.as_bytes(), v.as_bytes()).unwrap());
        }
        let _ = req.respond(resp);
    }
}

/// The "other host" a log redirect points at: serves one log text and records the names of the
/// headers each request carried (lowercase), so tests can assert the token never arrived.
pub struct LogHost {
    pub url: String,
    pub seen: Arc<Mutex<Vec<Vec<String>>>>,
    stop: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
}

impl LogHost {
    pub fn start(text: &str) -> LogHost {
        let server = tiny_http::Server::http("127.0.0.1:0").expect("bind log host");
        let url = format!("http://{}", server.server_addr().to_ip().expect("ip"));
        let seen = Arc::new(Mutex::new(Vec::new()));
        let stop = Arc::new(AtomicBool::new(false));
        let (seen2, stop2, text) = (seen.clone(), stop.clone(), text.to_string());
        let handle = std::thread::spawn(move || {
            while !stop2.load(Ordering::SeqCst) {
                let Ok(Some(req)) = server.recv_timeout(Duration::from_millis(25)) else {
                    continue;
                };
                let names = req
                    .headers()
                    .iter()
                    .map(|h| h.field.as_str().as_str().to_ascii_lowercase())
                    .collect();
                seen2.lock().unwrap().push(names);
                let _ = req.respond(tiny_http::Response::from_string(text.clone()));
            }
        });
        LogHost {
            url,
            seen,
            stop,
            handle: Some(handle),
        }
    }

    /// Header names of every request received.
    pub fn headers_seen(&self) -> Vec<Vec<String>> {
        self.seen.lock().unwrap().clone()
    }
}

impl Drop for LogHost {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}
