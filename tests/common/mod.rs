//! Shared test helpers. Every bosun child runs hermetic: no real token, no real GitHub.
#![allow(dead_code)]

use assert_cmd::Command;
use serde_json::Value;

/// A token canary: tests assert it never reaches stdout or stderr.
pub const TOKEN_CANARY: &str = "fake-token-canary";

/// The `bosun` binary with a scrubbed environment.
pub fn bosun() -> Command {
    let mut cmd = Command::cargo_bin("bosun").expect("bosun binary");
    cmd.env_remove("GH_TOKEN")
        .env("GITHUB_TOKEN", TOKEN_CANARY)
        .env("BOSUN_GITHUB_API", "http://127.0.0.1:9")
        .env("BOSUN_REPO", "acme/widget")
        .env("BOSUN_SESSION", "session_test")
        .env("BOSUN_NOW", "2026-10-22T10:00:00Z")
        .env_remove("BOSUN_HALT")
        .env_remove("BOSUN_ORIGIN")
        .env_remove("CLAUDE_CODE_CHILD_SESSION")
        .env_remove("CLAUDE_CODE_REMOTE_SESSION_ID");
    cmd
}

/// Parse stdout as exactly one JSON object.
pub fn one_object(stdout: &[u8]) -> Value {
    let text = std::str::from_utf8(stdout).expect("utf-8 stdout");
    let mut stream = serde_json::Deserializer::from_str(text).into_iter::<Value>();
    let first = stream.next().expect("one JSON value").expect("valid JSON");
    assert!(stream.next().is_none(), "more than one JSON value: {text}");
    assert!(first.is_object(), "not an object: {text}");
    first
}
pub mod fake;
