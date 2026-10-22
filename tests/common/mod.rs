//! Shared test helpers. Every bosun child runs hermetic: no real token, no real GitHub.
#![allow(dead_code)]

use assert_cmd::Command;
use fake::FakeGithub;
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

/// Run `bosun <args>` as `session` at `now` against the fake; returns (exit code, envelope).
/// Asserts the token never leaks and every error has a fix.
pub fn run(fake: &FakeGithub, session: &str, now: &str, args: &[&str]) -> (i32, Value) {
    let out = bosun()
        .env("BOSUN_GITHUB_API", fake.url())
        .env("BOSUN_SESSION", session)
        .env("BOSUN_NOW", now)
        .args(args)
        .output()
        .unwrap();
    let all = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(!all.contains(TOKEN_CANARY), "token leaked");
    let v = one_object(&out.stdout);
    if v["ok"] == false {
        assert!(!v["error"]["fix"].as_str().unwrap().trim().is_empty());
    }
    (out.status.code().unwrap(), v)
}

/// A lease comment body as `bosun claim` writes it.
pub fn lease_note(session: &str, expires: &str, touch: &str) -> String {
    format!(
        "Claimed.\n\n{{\"bosun\":\"lease\",\"session\":\"{session}\",\"origin\":\"local\",\"expires\":\"{expires}\",\"touch\":[\"{touch}\"]}}"
    )
}

pub mod fake;
