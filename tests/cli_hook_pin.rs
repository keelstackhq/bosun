mod common;

use std::path::Path;

use common::bosun;
use common::fake::FakeGithub;
use common::npm::{Registry, repo};
use serde_json::{Value, json};

fn start(d: &Path, reg: &Registry, pin_check: Option<&str>) -> (String, String) {
    let fake = FakeGithub::start();
    let mut cmd = bosun();
    cmd.current_dir(d)
        .env("BOSUN_GITHUB_API", fake.url())
        .env("BOSUN_NPM_REGISTRY", &reg.base)
        .args(["hook", "session-start"])
        .write_stdin(json!({"cwd": d.to_str().unwrap()}).to_string());
    match pin_check {
        Some(v) => cmd.env("BOSUN_PIN_CHECK", v),
        None => cmd.env_remove("BOSUN_PIN_CHECK"),
    };
    let out = cmd.output().unwrap();
    assert_eq!(out.status.code(), Some(0));
    let v: Value = common::one_object(&out.stdout);
    (
        v["hookSpecificOutput"]["additionalContext"]
            .as_str()
            .unwrap()
            .to_string(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

fn registry(hoist: &str) -> Registry {
    Registry::start(
        &[
            ("splice", "0.4.0"),
            ("hoist", hoist),
            ("berth", "0.1.0"),
            ("bosun", "0.9.0"),
        ],
        None,
        None,
    )
}

#[test]
fn a_behind_pin_adds_one_line_unless_opted_out() {
    let (d, reg) = (repo(), registry("0.4.1"));
    for setting in [None, Some("1")] {
        let (text, _) = start(d.path(), &reg, setting);
        assert!(
            text.contains("tool pins behind: hoist 0.4.0 -> 0.4.1. The orchestrator moves them"),
            "{text}"
        );
    }
    let (text, _) = start(d.path(), &reg, Some("0"));
    assert!(!text.contains("tool pins behind"), "{text}");
}

#[test]
fn nothing_behind_or_a_channel_consumer_adds_nothing() {
    let (d, reg) = (repo(), registry("0.4.0"));
    assert!(!start(d.path(), &reg, None).0.contains("tool pins behind"));
    let reg = registry("0.4.1");
    let ch = d.path().join(".splice/channels/bosun.toml");
    std::fs::create_dir_all(ch.parent().unwrap()).unwrap();
    std::fs::write(ch, "").unwrap();
    let (text, err) = start(d.path(), &reg, None);
    assert!(!text.contains("tool pins behind"));
    // A channel repo makes no npm request at SessionStart (spec 04).
    assert!(reg.log.lock().unwrap().is_empty(), "{:?}", reg.log);
    assert!(!err.contains("pin check"), "{err}");
}

#[test]
fn an_npm_error_adds_nothing_and_logs_to_stderr() {
    let d = repo();
    let reg = Registry {
        base: "http://127.0.0.1:9".into(),
        ..registry("0.4.0")
    };
    let (text, err) = start(d.path(), &reg, None);
    assert!(!text.contains("tool pins behind"), "{text}");
    assert!(err.contains("pin check"), "{err}");
}

#[test]
fn a_stalled_registry_is_cut_off_by_the_deadline() {
    let server = tiny_http::Server::http("127.0.0.1:0").unwrap();
    let base = format!(
        "http://127.0.0.1:{}",
        server.server_addr().to_ip().unwrap().port()
    );
    // Accept every request and never answer.
    std::thread::spawn(move || {
        let mut held = Vec::new();
        for req in server.incoming_requests() {
            held.push(req);
        }
    });
    let d = repo();
    let reg = Registry {
        base,
        ..registry("0.4.0")
    };
    let t = std::time::Instant::now();
    let (text, err) = start(d.path(), &reg, None);
    assert!(
        t.elapsed() < std::time::Duration::from_secs(20),
        "{:?}",
        t.elapsed()
    );
    assert!(!text.contains("tool pins behind"), "{text}");
    assert!(err.contains("pin check"), "{err}");
}
