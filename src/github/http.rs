//! The only module that talks to the network. Errors and `-v` logs never carry headers, the token
//! or request bodies.

use crate::output::BosunError;
use serde_json::Value;
use std::time::{Duration, Instant};

pub type Result<T> = std::result::Result<T, BosunError>;

const MAX_ATTEMPTS: u32 = 3;
const DEFAULT_MAX_SLEEP_MS: u64 = 60_000;

/// Percent-encode a path segment or query value.
pub fn enc(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

pub struct Resp {
    pub status: u16,
    pub body: Vec<u8>,
    link_next: Option<String>,
    retry_after: Option<u64>,
    rate_remaining_zero: bool,
}

impl Resp {
    fn json(&self) -> Value {
        serde_json::from_slice(&self.body).unwrap_or(Value::Null)
    }
}

pub struct Http {
    base: String,
    token: String,
    agent: ureq::Agent,
    max_sleep_ms: u64,
    verbose: bool,
}

impl std::fmt::Debug for Http {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Http({})", self.base)
    }
}

impl Http {
    /// Base URL, token and sleep cap from the environment (spec D30, D31).
    pub fn from_env() -> Result<Http> {
        let token = ["GITHUB_TOKEN", "GH_TOKEN"]
            .iter()
            .filter_map(|k| std::env::var(k).ok())
            .find(|v| !v.trim().is_empty())
            .ok_or_else(|| {
                BosunError::error(
                    "no GitHub token",
                    "export GITHUB_TOKEN (a token with repo and workflow scopes)",
                )
            })?;
        let base = std::env::var("BOSUN_GITHUB_API")
            .unwrap_or_else(|_| "https://api.github.com".to_string());
        let max_sleep = std::env::var("BOSUN_HTTP_MAX_SLEEP_MS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(DEFAULT_MAX_SLEEP_MS);
        Ok(Http::new(&base, &token, max_sleep))
    }

    pub fn new(base: &str, token: &str, max_sleep_ms: u64) -> Http {
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .max_redirects(0)
            .http_status_as_error(false)
            .tls_config(
                ureq::tls::TlsConfig::builder()
                    .root_certs(ureq::tls::RootCerts::PlatformVerifier)
                    .build(),
            )
            .timeout_connect(Some(Duration::from_secs(10)))
            .timeout_recv_response(Some(Duration::from_secs(30)))
            .timeout_recv_body(Some(Duration::from_secs(30)))
            .build()
            .into();
        Http {
            base: base.trim_end_matches('/').to_string(),
            token: token.to_string(),
            agent,
            max_sleep_ms,
            verbose: std::env::var("BOSUN_VERBOSE").is_ok_and(|v| v == "1"),
        }
    }

    /// GET one JSON document; 404 is `None`.
    pub fn get_json(&self, path: &str) -> Result<Option<Value>> {
        let r = self.request("GET", path, None)?;
        match r.status {
            200..=299 => Ok(Some(r.json())),
            404 => Ok(None),
            _ => Err(self.fail("GET", path, &r)),
        }
    }

    /// GET every page of a list, following `Link: rel="next"`.
    pub fn get_all(&self, path: &str) -> Result<Vec<Value>> {
        self.pages(path, None)
    }

    /// Like `get_all` for endpoints that wrap each page's array in an object under `key`
    /// (`check_runs`, `workflow_runs`, `jobs`).
    pub fn get_all_in(&self, path: &str, key: &str) -> Result<Vec<Value>> {
        self.pages(path, Some(key))
    }

    fn pages(&self, path: &str, key: Option<&str>) -> Result<Vec<Value>> {
        let mut items = Vec::new();
        let mut next = Some(path.to_string());
        while let Some(path) = next {
            let r = self.request("GET", &path, None)?;
            if !(200..=299).contains(&r.status) {
                return Err(self.fail("GET", &path, &r));
            }
            let body = r.json();
            let page = match key {
                Some(k) => body[k].clone(),
                None => body,
            };
            if let Value::Array(page) = page {
                items.extend(page);
            }
            next = match &r.link_next {
                // Only follow links that stay on the configured API host.
                Some(url) => Some(
                    url.strip_prefix(&self.base)
                        .map(str::to_string)
                        .ok_or_else(|| {
                            BosunError::error(
                                format!(
                                    "GitHub GET {}: next page points to another host",
                                    bare(&path)
                                ),
                                "retry; if it persists check https://www.githubstatus.com",
                            )
                        })?,
                ),
                None => None,
            };
        }
        Ok(items)
    }

    /// POST/PATCH/PUT/DELETE with a JSON body; 2xx is `Ok` (204 is `Null`).
    pub fn send_json(&self, method: &str, path: &str, body: &Value) -> Result<Value> {
        let r = self.request(method, path, Some(body))?;
        if (200..=299).contains(&r.status) {
            Ok(r.json())
        } else {
            Err(self.fail(method, path, &r))
        }
    }

    /// Like `send_json` for callers that also accept the statuses in `ok` (404 on a label
    /// removal, 422 on an existing label); returns the status.
    pub fn send_accepting(
        &self,
        method: &str,
        path: &str,
        body: &Value,
        ok: &[u16],
    ) -> Result<u16> {
        let r = self.request(method, path, (!body.is_null()).then_some(body))?;
        if (200..=299).contains(&r.status) || ok.contains(&r.status) {
            Ok(r.status)
        } else {
            Err(self.fail(method, path, &r))
        }
    }

    /// One logical request: up to 3 attempts on 5xx and rate limits (never for POST).
    fn request(&self, method: &str, path: &str, body: Option<&Value>) -> Result<Resp> {
        let mut attempt = 1;
        loop {
            let r = self.once(method, path, body)?;
            let limited = r.status == 429
                || (r.status == 403 && (r.retry_after.is_some() || r.rate_remaining_zero));
            if method != "POST" && attempt < MAX_ATTEMPTS && (r.status >= 500 || limited) {
                let ms = (r.retry_after.unwrap_or(1) * 1000).min(self.max_sleep_ms);
                if ms > 0 {
                    std::thread::sleep(Duration::from_millis(ms));
                }
                attempt += 1;
                continue;
            }
            return Ok(r);
        }
    }

    fn once(&self, method: &str, path: &str, body: Option<&Value>) -> Result<Resp> {
        let url = format!("{}{path}", self.base);
        let started = Instant::now();
        let transport = |detail: String| {
            BosunError::error(
                format!("GitHub {method} {}: {detail}", bare(path)),
                "check the network and BOSUN_GITHUB_API, then retry",
            )
        };
        macro_rules! build {
            ($b:expr) => {
                $b.header("Authorization", format!("Bearer {}", self.token))
                    .header("Accept", "application/vnd.github+json")
                    .header("X-GitHub-Api-Version", "2022-11-28")
                    .header("User-Agent", concat!("bosun/", env!("CARGO_PKG_VERSION")))
            };
        }
        let payload = body.map(|b| b.to_string());
        let result = match method {
            "GET" => build!(self.agent.get(&url)).call(),
            "DELETE" => {
                let b = build!(self.agent.delete(&url));
                match &payload {
                    Some(p) => b
                        .force_send_body()
                        .header("Content-Type", "application/json")
                        .send(p.as_bytes()),
                    None => b.call(),
                }
            }
            m => {
                let b = match m {
                    "POST" => build!(self.agent.post(&url)),
                    "PUT" => build!(self.agent.put(&url)),
                    "PATCH" => build!(self.agent.patch(&url)),
                    other => return Err(transport(format!("unsupported method {other}"))),
                }
                .header("Content-Type", "application/json");
                match &payload {
                    Some(p) => b.send(p.as_bytes()),
                    None => b.send_empty(),
                }
            }
        };
        let mut resp = result.map_err(|e| transport(e.to_string()))?;
        let status = resp.status().as_u16();
        let header = |name: &str| {
            resp.headers()
                .get(name)
                .and_then(|v| v.to_str().ok())
                .map(str::to_string)
        };
        let link_next = header("link").and_then(|v| parse_link_next(&v));
        let retry_after = header("retry-after").and_then(|v| v.trim().parse().ok());
        let rate_remaining_zero = header("x-ratelimit-remaining").is_some_and(|v| v.trim() == "0");
        let body = resp
            .body_mut()
            .read_to_vec()
            .map_err(|e| transport(e.to_string()))?;
        if self.verbose {
            eprintln!(
                "bosun: {method} {} {status} {}ms",
                bare(path),
                started.elapsed().as_millis()
            );
        }
        Ok(Resp {
            status,
            body,
            link_next,
            retry_after,
            rate_remaining_zero,
        })
    }

    /// The error for a non-success response: method, path, status and GitHub's `message`.
    fn fail(&self, method: &str, path: &str, r: &Resp) -> BosunError {
        let api = r.json()["message"].as_str().unwrap_or("").to_string();
        let message = format!("GitHub {method} {}: {} {api}", bare(path), r.status);
        let fix = match r.status {
            401 => "the token was rejected; check GITHUB_TOKEN",
            403 | 429 => "the token lacks permission or hit a rate limit",
            404 if method != "GET" => "check BOSUN_REPO and the issue/PR number",
            _ => "retry; if it persists check https://www.githubstatus.com",
        };
        BosunError::error(message.trim_end(), fix)
    }
}

/// The path without its query string.
fn bare(path: &str) -> &str {
    path.split('?').next().unwrap_or(path)
}

/// The `rel="next"` target of a `Link` header.
fn parse_link_next(value: &str) -> Option<String> {
    value.split(',').find_map(|part| {
        let (target, params) = part.split_once(';')?;
        let is_next = params
            .split(';')
            .any(|p| p.trim().replace(' ', "") == "rel=\"next\"");
        let target = target.trim().strip_prefix('<')?.strip_suffix('>')?;
        is_next.then(|| target.to_string())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn enc_escapes_label_characters() {
        assert_eq!(enc("bosun:state/ready"), "bosun%3Astate%2Fready");
        assert_eq!(enc("a b-c_d.e~"), "a%20b-c_d.e~");
    }

    #[test]
    fn link_next_is_found() {
        let v = r#"<http://x/a?page=2>; rel="next", <http://x/a?page=9>; rel="last""#;
        assert_eq!(parse_link_next(v).as_deref(), Some("http://x/a?page=2"));
        assert_eq!(parse_link_next(r#"<http://x/a>; rel="prev""#), None);
    }
}
