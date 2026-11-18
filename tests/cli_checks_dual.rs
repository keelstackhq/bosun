//! In `[checks] mode = "dual"` a reader takes, per gate, the worse of the newest status and the
//! trusted App's newest check run, so a status forged by any workflow cannot outvote the App
//! (spec §15, "Verifying the poster").

mod common;

use common::fake::FakeGithub;
use common::{bosun, one_object};
use serde_json::Value;

const SLUG: &str = "keelstackhq-bosun-gate";
const APP_ID: u64 = 4242;
const WAITING: &str = "high risk, owner approval needed: R-gov";
const BASE: &str = "base0000";
const HEAD: &str = "sha1";

fn repo(mode: &str) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join(".bosun")).unwrap();
    let text = format!("[checks]\nmode = \"{mode}\"\napp_id = {APP_ID}\napp_slug = \"{SLUG}\"\n");
    std::fs::write(dir.path().join(".bosun/bosun.toml"), text).unwrap();
    dir
}

fn list(fake: &FakeGithub, dir: &tempfile::TempDir) -> Value {
    let out = bosun()
        .env("BOSUN_GITHUB_API", fake.url())
        .env("BOSUN_OWNER_TOKEN", "canary-owner-token-7f3a")
        .env("BOSUN_HTTP_MAX_SLEEP_MS", "0")
        .arg("-C")
        .arg(dir.path())
        .args(["approve", "--list"])
        .output()
        .unwrap();
    let v = one_object(&out.stdout);
    assert_eq!(out.status.code(), Some(0), "{v}");
    v
}

fn ready(v: &Value) -> Vec<u64> {
    (v["data"]["ready"].as_array().unwrap().iter())
        .map(|r| r["pr"].as_u64().unwrap())
        .collect()
}

/// PR 1 waits for the owner by status; review-verdict is green by status; ci-ok green.
fn waiting() -> FakeGithub {
    let fake = FakeGithub::start();
    fake.pull(1, "gov change", "b1", "");
    fake.pull_as_issue(1, "gov change");
    fake.pull_head_sha(1, HEAD);
    fake.compare_file(BASE, HEAD, "governance/f.toml", "@@");
    fake.status(HEAD, "risk-check", "failure", WAITING);
    fake.status(HEAD, "review-verdict", "success", "approved");
    fake.check_run(HEAD, "ci-ok", "completed", Some("success"));
    fake
}

#[test]
fn dual_a_forged_green_status_loses_to_the_apps_run() {
    let fake = waiting();
    let dual = repo("dual");
    assert_eq!(ready(&list(&fake, &dual)), [1], "no App run: statuses");

    // The App says request changes; the green status (any workflow can post one) loses.
    fake.app_run(
        HEAD,
        "review-verdict",
        "failure",
        "request changes",
        SLUG,
        APP_ID,
    );
    assert!(ready(&list(&fake, &dual)).is_empty(), "App run wins");

    // A newer App run that agrees makes it ready again.
    fake.app_run(HEAD, "review-verdict", "success", "approved", SLUG, APP_ID);
    assert_eq!(ready(&list(&fake, &dual)), [1]);
}

#[test]
fn dual_a_red_status_still_counts_over_a_green_app_run() {
    let fake = waiting();
    let dual = repo("dual");
    fake.app_run(HEAD, "review-verdict", "success", "approved", SLUG, APP_ID);
    fake.status(HEAD, "review-verdict", "failure", "request changes");
    assert!(ready(&list(&fake, &dual)).is_empty(), "worse of the two");
}

#[test]
fn dual_other_apps_runs_never_count() {
    let fake = waiting();
    let dual = repo("dual");
    fake.app_run(
        HEAD,
        "review-verdict",
        "failure",
        "x",
        "github-actions",
        15368,
    );
    fake.app_run(HEAD, "review-verdict", "failure", "x", "other-app", 7);
    assert_eq!(ready(&list(&fake, &dual)), [1], "only the trusted App");
}
