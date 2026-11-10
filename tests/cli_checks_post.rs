//! The gate writer: `[checks] mode` decides whether risk-check, verdict post and main-open post
//! a commit status, a check run of the gate App, or both (spec §15, UC-B2-1, -2, -5).

mod common;

use common::fake::FakeGithub;
use common::{bosun, one_object};
use serde_json::Value;

const APP_TOKEN: &str = "app-token-canary";
const HEAD: &str = "sha1";
const APP: &str =
    "[checks]\nmode = \"MODE\"\napp_id = 4242\napp_slug = \"keelstackhq-bosun-gate\"\n";

/// A repo dir whose `.bosun/bosun.toml` sets `mode` (`None`: no `[checks]`).
fn repo(mode: Option<&str>) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join(".bosun")).unwrap();
    let text = mode.map_or(String::new(), |m| APP.replace("MODE", m));
    std::fs::write(dir.path().join(".bosun/bosun.toml"), text).unwrap();
    dir
}

/// A normal-risk PR: one docs file, no rules at base.
fn pr() -> FakeGithub {
    let fake = FakeGithub::start();
    fake.pull(1, "docs", "feature", "No link.");
    fake.pull_head_sha(1, HEAD);
    fake.pull_file(
        1,
        "docs/a.md",
        "modified",
        1,
        0,
        Some("@@ -0,0 +1,1 @@\n+x"),
    );
    fake
}

fn bosun_in(
    fake: &FakeGithub,
    dir: &tempfile::TempDir,
    token: bool,
    args: &[&str],
) -> (i32, Value, String) {
    let mut cmd = bosun();
    cmd.env("BOSUN_GITHUB_API", fake.url())
        .env_remove("BOSUN_APP_TOKEN")
        .arg("-C")
        .arg(dir.path())
        .args(args);
    if token {
        cmd.env("BOSUN_APP_TOKEN", APP_TOKEN);
    }
    let out = cmd.output().unwrap();
    let stderr = String::from_utf8_lossy(&out.stderr).to_string();
    let all = format!("{}{stderr}", String::from_utf8_lossy(&out.stdout));
    assert!(!all.contains(APP_TOKEN), "the App token leaked: {all}");
    let v = one_object(&out.stdout);
    if v["ok"] == false {
        assert!(!v["error"]["fix"].as_str().unwrap().trim().is_empty());
    }
    (out.status.code().unwrap(), v, stderr)
}

fn statuses(fake: &FakeGithub, ctx: &str) -> Vec<Value> {
    fake.statuses_of(HEAD)
        .into_iter()
        .filter(|s| s["context"] == ctx)
        .collect()
}

fn runs(fake: &FakeGithub, name: &str) -> Vec<Value> {
    fake.check_runs_of(HEAD)
        .into_iter()
        .filter(|r| r["name"] == name)
        .collect()
}

#[test]
fn statuses_mode_posts_no_check_run_even_with_a_token() {
    for mode in [None, Some("statuses")] {
        let fake = pr();
        let dir = repo(mode);
        let (code, v, _) = bosun_in(&fake, &dir, true, &["risk-check", "1"]);
        assert_eq!(code, 0, "{v}");
        let s = statuses(&fake, "risk-check");
        assert_eq!(s.last().unwrap()["state"], "success");
        assert!(fake.check_runs_of(HEAD).is_empty());
        assert!(v["data"].get("checks_skipped").is_none());
    }
}

#[test]
fn uc_b2_2_dual_posts_a_status_and_one_check_run_with_the_same_verdict() {
    let fake = pr();
    let dir = repo(Some("dual"));
    let (code, v, _) = bosun_in(&fake, &dir, true, &["risk-check", "1"]);
    assert_eq!(code, 0, "{v}");
    let s = statuses(&fake, "risk-check");
    assert_eq!(
        s.iter()
            .map(|s| s["state"].as_str().unwrap())
            .collect::<Vec<_>>(),
        ["pending", "success"]
    );
    let r = runs(&fake, "risk-check");
    assert_eq!(r.len(), 1, "the verdict completes the pending run: {r:?}");
    assert_eq!(r[0]["status"], "completed");
    assert_eq!(r[0]["conclusion"], "success");
    assert_eq!(r[0]["head_sha"], HEAD);
    assert_eq!(r[0]["output"]["title"], s[1]["description"]);
    // The check run is written with the App token, the status with GITHUB_TOKEN.
    let auth = |method: &str, suffix: &str| {
        fake.requests()
            .into_iter()
            .filter(|l| l.method == method && l.path.ends_with(suffix))
            .map(|l| l.headers[0].clone().unwrap_or_default())
            .collect::<Vec<_>>()
    };
    assert!(
        auth("POST", "/check-runs")
            .iter()
            .all(|a| a.contains(APP_TOKEN))
    );
    assert!(
        auth("PATCH", &format!("/check-runs/{}", r[0]["id"]))
            .iter()
            .all(|a| a.contains(APP_TOKEN))
    );
    assert!(
        auth("POST", &format!("/statuses/{HEAD}"))
            .iter()
            .all(|a| !a.contains(APP_TOKEN))
    );
}

#[test]
fn uc_b2_1_checks_mode_posts_only_the_check_run() {
    let fake = pr();
    let dir = repo(Some("checks"));
    let (code, v, _) = bosun_in(&fake, &dir, true, &["risk-check", "1"]);
    assert_eq!(code, 0, "{v}");
    assert!(statuses(&fake, "risk-check").is_empty());
    let r = runs(&fake, "risk-check");
    assert_eq!(r.len(), 1);
    assert_eq!(r[0]["conclusion"], "success");
    assert!(
        r[0]["output"]["summary"]
            .as_str()
            .unwrap()
            .starts_with("normal risk")
    );
}

#[test]
fn uc_b2_5_checks_mode_without_the_token_fails_and_names_the_environment() {
    let fake = pr();
    let dir = repo(Some("checks"));
    let (code, v, _) = bosun_in(&fake, &dir, false, &["risk-check", "1"]);
    assert_eq!(code, 1, "{v}");
    let fix = v["error"]["fix"].as_str().unwrap();
    assert!(fix.contains("bosun-app"), "{fix}");
    assert!(fix.contains("BOSUN_APP_TOKEN"), "{fix}");
    assert!(statuses(&fake, "risk-check").is_empty());
    assert!(fake.check_runs_of(HEAD).is_empty());
}

#[test]
fn dual_without_the_token_posts_the_status_and_says_the_run_was_skipped() {
    let fake = pr();
    let dir = repo(Some("dual"));
    let (code, v, stderr) = bosun_in(&fake, &dir, false, &["risk-check", "1"]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["checks_skipped"], true);
    assert_eq!(
        statuses(&fake, "risk-check").last().unwrap()["state"],
        "success"
    );
    assert!(fake.check_runs_of(HEAD).is_empty());
    assert_eq!(
        stderr.matches("BOSUN_APP_TOKEN is not set").count(),
        1,
        "{stderr}"
    );
}

#[test]
fn uc_b2_2_dual_main_open_posts_both() {
    let fake = pr();
    let dir = repo(Some("dual"));
    let (code, v, _) = bosun_in(&fake, &dir, true, &["gate", "main-open", "--pr", "1"]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(
        statuses(&fake, "main-open").last().unwrap()["state"],
        "success"
    );
    let r = runs(&fake, "main-open");
    assert_eq!((r.len(), r[0]["conclusion"].as_str()), (1, Some("success")));
    assert_eq!(r[0]["output"]["title"], "main is open");
}

#[test]
fn a_red_main_is_a_failed_run_never_neutral() {
    // UC-B2-6 for main-open: a failing gate is conclusion failure.
    let fake = pr();
    fake.issue(3, "main is red", &["bosun:main-red"], "ci-ok failed");
    let dir = repo(Some("checks"));
    let (code, v, _) = bosun_in(&fake, &dir, true, &["gate", "main-open", "--pr", "1"]);
    assert_eq!(code, 4, "{v}");
    let r = runs(&fake, "main-open");
    assert_eq!(r[0]["conclusion"], "failure");
}

#[test]
fn uc_b2_2_dual_verdict_post_posts_both() {
    let fake = pr();
    fake.pull_as_issue(1, "docs");
    let dir = repo(Some("dual"));
    let json = format!(r#"{{"pr":1,"head_sha":"{HEAD}","verdict":"approve","confidence":0.95}}"#);
    let file = dir.path().join("v.json");
    std::fs::write(&file, json).unwrap();
    let (code, v, _) = bosun_in(
        &fake,
        &dir,
        true,
        &["verdict", "post", "--from", file.to_str().unwrap()],
    );
    assert_eq!(code, 0, "{v}");
    assert_eq!(
        statuses(&fake, "review-verdict").last().unwrap()["state"],
        "success"
    );
    let r = runs(&fake, "review-verdict");
    assert_eq!((r.len(), r[0]["conclusion"].as_str()), (1, Some("success")));
}
