mod common;

use common::fake::FakeGithub;
use common::npm::{Registry, repo};
use common::{bosun, one_object};
use serde_json::{Value, json};

fn pin(fake: Option<&FakeGithub>, token: bool) -> (Value, Registry, tempfile::TempDir) {
    let d = repo();
    let reg = Registry::start(
        &[
            ("splice", "0.4.0"),
            ("hoist", "0.4.1"),
            ("berth", "0.1.0"),
            ("bosun", "0.9.0"),
        ],
        None,
        None,
    );
    let mut cmd = bosun();
    cmd.env("BOSUN_NPM_REGISTRY", &reg.base).args([
        "-C",
        d.path().to_str().unwrap(),
        "pin",
        "--latest",
    ]);
    if let Some(f) = fake {
        cmd.env("BOSUN_GITHUB_API", f.url());
    }
    if !token {
        cmd.env_remove("GITHUB_TOKEN");
    }
    let out = cmd.output().unwrap();
    assert_eq!(out.status.code(), Some(0), "{out:?}");
    (one_object(&out.stdout), reg, d)
}

#[test]
fn an_open_pin_pr_is_refreshed_not_duplicated() {
    let fake = FakeGithub::start();
    fake.pull(7, "chore: pin hoist 0.4.0", "bosun/pin", "");
    let (v, _r, _d) = pin(Some(&fake), true);
    assert_eq!(v["data"]["pr"], 7, "{v}");
    assert_eq!(v["next"], json!([]));
    let steps = v["data"]["steps"].to_string();
    assert!(steps.contains("bosun/pin") && steps.contains("never force-push"));
}

#[test]
fn no_open_pin_pr_returns_the_create_call() {
    let fake = FakeGithub::start();
    let (v, _r, _d) = pin(Some(&fake), true);
    assert_eq!(v["data"]["pr"], Value::Null, "{v}");
    let n = &v["next"][0];
    assert_eq!(n["tool"], "mcp__github__create_pull_request");
    let a = &n["args"];
    assert_eq!(
        (a["owner"].as_str(), a["repo"].as_str(), a["head"].as_str()),
        (Some("acme"), Some("widget"), Some("bosun/pin"))
    );
    assert_eq!(
        (a["base"].as_str(), a["draft"].as_bool()),
        (Some("main"), Some(true))
    );
    assert_eq!(a["title"], "chore: pin hoist 0.4.1");
    assert!(a["body"].as_str().unwrap().contains("hoist 0.4.0 -> 0.4.1"));
}

#[test]
fn unreachable_github_does_not_fail_the_pin() {
    let (v, _r, _d) = pin(None, false);
    assert_eq!(v["data"]["pr"], Value::Null, "{v}");
    assert!(
        v["data"]["pr_lookup"]
            .as_str()
            .is_some_and(|s| !s.is_empty())
    );
    assert_eq!(v["next"][0]["tool"], "mcp__github__create_pull_request");
    assert_eq!(v["next"][0]["args"]["base"], "main");
}

#[test]
fn an_open_dated_pin_pr_is_refreshed_not_duplicated() {
    let fake = FakeGithub::start();
    fake.pull(9, "chore: pin hoist 0.4.0", "bosun/pin-20261001", "");
    let (v, _r, _d) = pin(Some(&fake), true);
    assert_eq!(v["data"]["pr"], 9, "{v}");
    assert_eq!(v["data"]["branch"], "bosun/pin-20261001");
    assert_eq!(v["next"], json!([]));
}
