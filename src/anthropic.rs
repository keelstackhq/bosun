//! A minimal Anthropic Messages API client for `bosun review-ci` (spec §7, review mode b).
//! One call, no retries. The key comes from env `ANTHROPIC_API_KEY` and is never printed, logged
//! or put in an error; the struct has no `Debug` that shows it.

use std::time::Duration;

use serde_json::{Value, json};

use crate::output::BosunError;

const DEFAULT_BASE: &str = "https://api.anthropic.com";
const VERSION: &str = "2023-06-01";

/// What one call returned.
#[derive(Debug, Clone, PartialEq)]
pub struct Reply {
    /// The text blocks of the answer, concatenated.
    pub text: String,
    pub input_tokens: u64,
    pub output_tokens: u64,
}

pub struct Anthropic {
    base: String,
    key: String,
    agent: ureq::Agent,
}

impl std::fmt::Debug for Anthropic {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Anthropic({})", self.base)
    }
}

impl Anthropic {
    /// Base URL from `BOSUN_ANTHROPIC_API`, key from `ANTHROPIC_API_KEY`.
    pub fn from_env() -> Result<Anthropic, BosunError> {
        let key = std::env::var("ANTHROPIC_API_KEY")
            .ok()
            .map(|k| k.trim().to_string())
            .filter(|k| !k.is_empty())
            .ok_or_else(|| {
                BosunError::error(
                    "no Anthropic API key",
                    "set the ANTHROPIC_API_KEY secret (and pass it to the step as an env var)",
                )
            })?;
        let base = std::env::var("BOSUN_ANTHROPIC_API").unwrap_or_else(|_| DEFAULT_BASE.into());
        check_base(&base)?;
        Ok(Anthropic::new(&base, &key))
    }

    pub fn new(base: &str, key: &str) -> Anthropic {
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .max_redirects(0)
            .http_status_as_error(false)
            .tls_config(
                ureq::tls::TlsConfig::builder()
                    .root_certs(ureq::tls::RootCerts::PlatformVerifier)
                    .build(),
            )
            .timeout_connect(Some(Duration::from_secs(10)))
            .timeout_recv_response(Some(Duration::from_secs(300)))
            .timeout_recv_body(Some(Duration::from_secs(300)))
            .build()
            .into();
        Anthropic {
            base: base.trim_end_matches('/').to_string(),
            key: key.to_string(),
            agent,
        }
    }

    /// Replace the key, and anything shaped like an Anthropic key, in an error text.
    fn clean(&self, text: &str) -> String {
        static KEY_LIKE: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
            regex::Regex::new(r"sk-ant-[A-Za-z0-9_\-]+").expect("static regex")
        });
        let text = text.replace(&self.key, "[redacted]");
        KEY_LIKE.replace_all(&text, "[redacted]").into_owned()
    }

    /// One Messages call: a system prompt and one user message.
    pub fn message(
        &self,
        model: &str,
        max_tokens: u32,
        system: &str,
        user: &str,
    ) -> Result<Reply, BosunError> {
        let body = json!({"model": model, "max_tokens": max_tokens, "system": system,
            "messages": [{"role": "user", "content": user}]})
        .to_string();
        let fail = |detail: String| {
            BosunError::error(
                format!("Anthropic API: {}", self.clean(&detail)),
                "check the network, ANTHROPIC_API_KEY and BOSUN_ANTHROPIC_API, then run the command again",
            )
        };
        let mut resp = self
            .agent
            .post(&format!("{}/v1/messages", self.base))
            .header("x-api-key", &self.key)
            .header("anthropic-version", VERSION)
            .header("content-type", "application/json")
            .send(body.as_bytes())
            .map_err(|e| fail(e.to_string()))?;
        let status = resp.status().as_u16();
        let raw = resp
            .body_mut()
            .read_to_vec()
            .map_err(|e| fail(e.to_string()))?;
        let v: Value = serde_json::from_slice(&raw).unwrap_or(Value::Null);
        if !(200..=299).contains(&status) {
            let kind = v["error"]["type"].as_str().unwrap_or("error");
            // Redact the whole message first, then cut it.
            let msg: String = self
                .clean(v["error"]["message"].as_str().unwrap_or(""))
                .chars()
                .take(200)
                .collect();
            return Err(fail(
                format!("{status} {kind} {msg}").trim_end().to_string(),
            ));
        }
        let text: String = v["content"]
            .as_array()
            .map(|blocks| {
                blocks
                    .iter()
                    .filter(|b| b["type"] == "text")
                    .filter_map(|b| b["text"].as_str())
                    .collect()
            })
            .unwrap_or_default();
        let (input, output) = (
            v["usage"]["input_tokens"].as_u64(),
            v["usage"]["output_tokens"].as_u64(),
        );
        let (Some(input_tokens), Some(output_tokens)) = (input, output) else {
            return Err(fail(
                "the answer has no usage.input_tokens/output_tokens".into(),
            ));
        };
        Ok(Reply {
            text,
            input_tokens,
            output_tokens,
        })
    }
}

/// The key travels in a header: the base must be https, or http on this machine.
fn check_base(base: &str) -> Result<(), BosunError> {
    let host = base.strip_prefix("http://").map(|r| {
        let end = r.find(['/', '?', '#']).unwrap_or(r.len());
        let auth = &r[..end];
        let auth = auth.rsplit('@').next().unwrap_or(auth);
        // "[::1]:80" is not a loopback name we accept; host is what precedes the port.
        auth.split(':').next().unwrap_or(auth).to_string()
    });
    let ok = base.starts_with("https://")
        || host
            .as_deref()
            .is_some_and(|h| h == "127.0.0.1" || h == "localhost")
            && !base.contains('@');
    if ok {
        Ok(())
    } else {
        Err(BosunError::error(
            "BOSUN_ANTHROPIC_API must be an https:// URL (http:// only for 127.0.0.1 or localhost)",
            "unset BOSUN_ANTHROPIC_API to use https://api.anthropic.com, or set an https:// URL",
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::check_base;

    #[test]
    fn the_base_must_be_https_or_loopback() {
        for ok in [
            "https://api.anthropic.com",
            "http://127.0.0.1:8080",
            "http://localhost",
        ] {
            assert!(check_base(ok).is_ok(), "{ok}");
        }
        for bad in [
            "http://example.com",
            "http://127.0.0.1@evil.com",
            "http://localhost.evil.com",
            "ftp://x",
            "api.anthropic.com",
        ] {
            assert!(check_base(bad).is_err(), "{bad}");
        }
    }
}
