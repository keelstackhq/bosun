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
    pub labels: Vec<String>,
    pub is_pr: bool,
    pub user: String,
}

#[derive(Debug, Clone)]
pub struct FakeComment {
    pub id: u64,
    pub issue: u64,
    pub body: String,
    pub user: String,
    pub created_at: String,
    pub updated_at: String,
}

/// Everything the fake knows. The Task 4 collections are placeholders until its routes land.
#[derive(Default)]
pub struct State {
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
    pub runs: Vec<Value>,
    pub log: Vec<Logged>,
    /// (status, remaining) answered before any routing.
    pub fail: Option<(u16, u32)>,
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
            labels: labels.iter().map(|l| l.to_string()).collect(),
            is_pr: false,
            user: "octocat".into(),
        });
    }

    /// An entry in the issues list that is a pull request.
    pub fn pull_as_issue(&self, number: u64, title: &str) {
        self.issue(number, title, &[], "");
        self.state.lock().unwrap().issues.last_mut().unwrap().is_pr = true;
    }

    pub fn comment(&self, issue: u64, body: &str) -> u64 {
        add_comment(&mut self.state.lock().unwrap(), issue, body)
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

fn add_comment(s: &mut State, issue: u64, body: &str) -> u64 {
    let id = s.comments.iter().map(|c| c.id).max().unwrap_or(100) + 1;
    s.comments.push(FakeComment {
        id,
        issue,
        body: body.into(),
        user: "octocat".into(),
        created_at: TS.into(),
        updated_at: TS.into(),
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
        "labels": labels, "user": {"login": i.user}});
    if i.is_pr {
        v["pull_request"] = json!({"url": "https://example.test/pull"});
    }
    v
}

fn comment_json(c: &FakeComment) -> Value {
    json!({"id": c.id, "body": c.body, "user": {"login": c.user},
        "created_at": c.created_at, "updated_at": c.updated_at})
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
    let ["repos", _, _, rest @ ..] = seg.as_slice() else {
        return msg(404, "Not Found");
    };
    let find = |s: &State, n: &str| {
        n.parse::<u64>()
            .ok()
            .and_then(|n| s.issues.iter().position(|i| i.number == n))
    };
    match (method, rest) {
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
                    (state == "all" || i.state == state)
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
                labels,
                is_pr: false,
                user: "octocat".into(),
            };
            let v = issue_json(&issue);
            s.issues.push(issue);
            (201, v, vec![])
        }
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
            }
            ok(issue_json(i))
        }
        ("POST", ["issues", n, "labels"]) => {
            let Some(p) = find(s, n) else {
                return msg(404, "Not Found");
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
        ("GET", ["actions", "variables", name]) => match s.variables.get(*name) {
            Some(v) => ok(json!({"name": name, "value": v})),
            None => msg(404, "Not Found"),
        },
        _ => msg(404, "Not Found"),
    }
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
        } else {
            route(&mut s, &method, path, raw_query, &body, &base)
        };
        drop(s);
        let mut resp = tiny_http::Response::from_data(if status == 204 {
            vec![]
        } else {
            value.to_string().into_bytes()
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
