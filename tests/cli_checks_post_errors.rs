//! The gate writer when the App's check-run write fails (spec §15): `dual` only warns, `checks`
//! fails and never leaves a second, stale run behind.

mod common;

use common::fake::FakeGithub;
use common::{bosun, one_object};
use serde_json::Value;

const HEAD: &str = "sha1";

fn repo(mode: &str) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join(".bosun")).unwrap();
    let text = format!("[checks]\nmode = \"{mode}\"\napp_id = 4242\napp_slug = \"gate\"\n");
    std::fs::write(dir.path().join(".bosun/bosun.toml"), text).unwrap();
    dir
}

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

fn risk_check(fake: &FakeGithub, dir: &tempfile::TempDir) -> (i32, Value, String) {
    let out = bosun()
        .env("BOSUN_GITHUB_API", fake.url())
        .env("BOSUN_APP_TOKEN", "app-token-canary")
        .env("BOSUN_HTTP_MAX_SLEEP_MS", "0")
        .arg("-C")
        .arg(dir.path())
        .args(["risk-check", "1"])
        .output()
        .unwrap();
    let stderr = String::from_utf8_lossy(&out.stderr).to_string();
    assert!(!stderr.contains("app-token-canary"));
    (out.status.code().unwrap(), one_object(&out.stdout), stderr)
}

#[test]
fn dual_keeps_the_status_and_exit_code_when_the_check_run_write_fails() {
    let fake = pr();
    fake.fail_path("/check-runs", 403);
    let (code, v, stderr) = risk_check(&fake, &repo("dual"));
    assert_eq!(code, 0, "{v}");
    let last = fake.statuses_of(HEAD).last().cloned().unwrap();
    assert_eq!(
        (last["context"].as_str(), last["state"].as_str()),
        (Some("risk-check"), Some("success"))
    );
    assert!(fake.check_runs_of(HEAD).is_empty());
    assert!(
        stderr.contains("risk-check check run not posted (dual mode)"),
        "{stderr}"
    );
}

#[test]
fn checks_mode_fails_on_a_failed_patch_and_leaves_no_second_run() {
    let fake = pr();
    // The first run the client creates is 1001; its verdict PATCH fails.
    fake.fail_path("/check-runs/1001", 422);
    let (code, v, _) = risk_check(&fake, &repo("checks"));
    assert_eq!(code, 1, "{v}");
    assert!(!v["error"]["fix"].as_str().unwrap().is_empty());
    let runs = fake.check_runs_of(HEAD);
    assert_eq!(
        runs.len(),
        1,
        "the error post retried the same run: {runs:?}"
    );
    assert!(fake.statuses_of(HEAD).is_empty());
}
