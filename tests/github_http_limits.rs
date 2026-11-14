//! HTTP client: 429 retry, a hung-up server, and the https-or-loopback base check.
mod common;

use std::io::Read;
use std::net::TcpListener;

use bosun::github::http::Http;
use common::TOKEN_CANARY;
use common::fake::FakeGithub;

const ISSUE: &str = "/repos/acme/widget/issues/1";

#[test]
fn a_429_then_200_is_retried() {
    let fake = FakeGithub::start();
    fake.issue(1, "t", &[], "");
    fake.fail_next(429, 1);
    let http = Http::new(&fake.url(), TOKEN_CANARY, 0);
    assert_eq!(http.get_json(ISSUE).unwrap().unwrap()["number"], 1);
    assert_eq!(fake.requests().len(), 2);
}

#[test]
fn a_server_that_hangs_up_is_an_error_naming_the_request() {
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}", l.local_addr().unwrap());
    std::thread::spawn(move || {
        for s in l.incoming().flatten() {
            let mut s = s;
            let _ = s.read(&mut [0u8; 64]);
        }
    });
    let e = Http::new(&url, TOKEN_CANARY, 0)
        .get_json(ISSUE)
        .unwrap_err();
    assert!(e.message.contains(ISSUE), "{}", e.message);
    assert!(!e.message.contains(TOKEN_CANARY));
}

fn status_with_base(base: &str) -> (serde_json::Value, String) {
    let out = common::bosun()
        .env("BOSUN_GITHUB_API", base)
        .args(["status"])
        .output()
        .unwrap();
    (
        common::one_object(&out.stdout),
        String::from_utf8_lossy(&out.stderr).into(),
    )
}

#[test]
fn a_plain_http_base_to_a_remote_host_is_refused_before_any_token_is_sent() {
    for base in [
        "http://evil.example.com",
        "ftp://x",
        "http://127.0.0.1@evil.example",
    ] {
        let (v, err) = status_with_base(base);
        assert_eq!(v["ok"], false, "{base}");
        let msg = v["error"]["message"].as_str().unwrap();
        assert!(msg.contains("BOSUN_GITHUB_API"), "{base}: {msg}");
        assert!(!err.contains(TOKEN_CANARY));
    }
}

#[test]
fn a_loopback_base_is_not_refused_by_the_scheme_check() {
    let fake = FakeGithub::start();
    let (v, _) = status_with_base(&fake.url());
    let msg = v["error"]["message"].as_str().unwrap_or("");
    assert!(!msg.contains("BOSUN_GITHUB_API"), "{v}");
}
