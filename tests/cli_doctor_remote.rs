mod common;

use common::fake::FakeGithub;
use common::{TOKEN_CANARY, bosun, one_object};
use serde_json::Value;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;

/// A fake `hoist`: logs its argv, prints `json`, exits `code`.
fn fake_hoist(dir: &Path, json: &str, code: i32) -> std::path::PathBuf {
    let bin = dir.join("fake-hoist");
    let log = dir.join("hoist-argv.log");
    std::fs::write(
        &bin,
        format!(
            "#!/bin/sh\necho \"$@\" >> '{}'\ncat <<'EOF_JSON'\n{json}\nEOF_JSON\nexit {code}\n",
            log.display()
        ),
    )
    .unwrap();
    std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
    bin
}

/// A repo whose local step 3 check passes.
fn repo_with_checks() -> tempfile::TempDir {
    let repo = tempfile::tempdir().unwrap();
    std::fs::write(
        repo.path().join("hoist.toml"),
        "[github]\nrequired_checks = [\"ci-ok\", \"risk-check\", \"review-verdict\", \"main-open\"]\n",
    )
    .unwrap();
    std::fs::create_dir_all(repo.path().join(".github/workflows")).unwrap();
    std::fs::write(
        repo.path().join(".github/workflows/ci.yml"),
        "jobs:\n  ci-ok:\n    runs-on: x\n",
    )
    .unwrap();
    repo
}

fn run(hoist: &Path, repo: &Path, remote: bool) -> (i32, Value) {
    let fake = FakeGithub::start();
    let mut cmd = bosun();
    cmd.env("BOSUN_GITHUB_API", fake.url())
        .env("BOSUN_HOIST", hoist)
        .arg("-C")
        .arg(repo)
        .arg("doctor");
    if remote {
        cmd.arg("--remote");
    }
    let out = cmd.output().unwrap();
    let v = one_object(&out.stdout);
    if v["ok"] == false {
        assert!(!v["error"]["fix"].as_str().unwrap().trim().is_empty());
    }
    (out.status.code().unwrap(), v)
}

fn step(v: &Value, n: usize) -> &Value {
    &v["data"]["items"][n - 1]
}

fn report(items: &str, ok: bool, code: i32) -> String {
    format!(r#"{{"version":1,"command":"check","ok":{ok},"exit_code":{code},"items":[{items}]}}"#)
}

const OK_ITEM: &str = r#"{"area":"github","kind":"branch_protection","key":"main","action":"ok","reason":"","source":"hoist.toml","verify":null}"#;
const CHECK_DRIFT: &str = r#"{"area":"github","kind":"required_check","key":"risk-check","action":"create","reason":"not required","source":"hoist.toml","verify":null}"#;
const ENV_DRIFT: &str = r#"{"area":"release","kind":"environment","key":"release","action":"create","reason":"absent","source":"hoist.toml","verify":null}"#;
const RULESET_DRIFT: &str = r#"{"area":"github","kind":"ruleset","key":"main","action":"update","reason":"differs","source":"hoist.toml","verify":null}"#;
const NPM_DRIFT: &str = r#"{"area":"npm","kind":"first_publish","key":"@keelstackhq/bosun","action":"create","reason":"not on the registry","source":"npm/bosun/package.json","verify":null}"#;
const PERMS_UNKNOWN: &str = r#"{"area":"github","kind":"actions_permissions","key":"default","action":"unknown","reason":"proxy refused","source":"default","verify":null}"#;
const VERIFY_BAD: &str = r#"{"area":"github","kind":"ruleset","key":"main","action":"ok","reason":"","source":"hoist.toml","verify":{"ok":false,"detail":"bypass actor present"}}"#;
const UNKNOWN: &str = r#"{"area":"github","kind":"secret","key":"TOKEN","action":"unknown","reason":"proxy refused","source":"hoist.toml","verify":null}"#;

#[test]
fn no_drift_mentions_hoist_and_lists_unknown() {
    let tmp = tempfile::tempdir().unwrap();
    let h = fake_hoist(
        tmp.path(),
        &report(&format!("{OK_ITEM},{UNKNOWN}"), true, 0),
        0,
    );
    let repo = repo_with_checks();
    let (code, v) = run(&h, repo.path(), true);
    assert_eq!(code, 5);
    let s3 = step(&v, 3);
    assert_eq!(s3["status"], "ok", "{s3}");
    assert!(
        s3["detail"]
            .as_str()
            .unwrap()
            .contains("hoist check: no drift")
    );
    assert_eq!(v["data"]["hoist"]["exit_code"], 0);
    assert_eq!(v["data"]["hoist"]["drift"].as_array().unwrap().len(), 0);
    assert_eq!(
        v["data"]["hoist"]["unknown"][0],
        "github/secret/TOKEN: unknown (proxy refused)"
    );
    let log = std::fs::read_to_string(tmp.path().join("hoist-argv.log")).unwrap();
    assert!(log.starts_with("check --json -C "), "{log}");
    assert!(!log.contains(TOKEN_CANARY));
}

#[test]
fn a_local_problem_still_fails_the_step_when_hoist_agrees() {
    let tmp = tempfile::tempdir().unwrap();
    let h = fake_hoist(tmp.path(), &report(OK_ITEM, true, 0), 0);
    let repo = tempfile::tempdir().unwrap();
    let (_, v) = run(&h, repo.path(), true);
    assert_eq!(step(&v, 3)["status"], "missing");
    assert!(
        step(&v, 3)["detail"]
            .as_str()
            .unwrap()
            .contains("hoist.toml")
    );
}

#[test]
fn required_check_drift_makes_step_three_missing() {
    let tmp = tempfile::tempdir().unwrap();
    let h = fake_hoist(
        tmp.path(),
        &report(&format!("{OK_ITEM},{CHECK_DRIFT}"), false, 1),
        1,
    );
    let repo = repo_with_checks();
    let (code, v) = run(&h, repo.path(), true);
    assert_eq!(code, 5);
    let s3 = step(&v, 3);
    assert_eq!(s3["status"], "missing");
    assert!(
        s3["detail"]
            .as_str()
            .unwrap()
            .contains("github/required_check/risk-check: create (not required)"),
        "{s3}"
    );
    assert_eq!(v["data"]["hoist"]["exit_code"], 1);
    assert_eq!(v["data"]["hoist"]["drift"].as_array().unwrap().len(), 1);
}

#[test]
fn environment_drift_makes_step_five_missing() {
    let tmp = tempfile::tempdir().unwrap();
    let h = fake_hoist(tmp.path(), &report(ENV_DRIFT, false, 1), 1);
    let repo = repo_with_checks();
    let (_, v) = run(&h, repo.path(), true);
    let s5 = step(&v, 5);
    assert_eq!(s5["status"], "missing");
    let (d5, f5) = (s5["detail"].as_str().unwrap(), s5["fix"].as_str().unwrap());
    assert!(d5.contains("bosun:halt"), "local problem kept: {d5}");
    assert!(
        f5.contains("gh label create") && f5.contains("hoist apply --area release"),
        "{f5}"
    );
    assert!(
        s5["detail"]
            .as_str()
            .unwrap()
            .contains("release/environment/release: create")
    );
    assert_eq!(step(&v, 3)["status"], "ok");
}

#[test]
fn missing_hoist_binary_marks_steps_three_and_five() {
    let repo = repo_with_checks();
    let (code, v) = run(Path::new("/nonexistent/hoist"), repo.path(), true);
    assert_eq!(code, 5);
    for n in [3, 5] {
        let s = step(&v, n);
        assert_eq!(s["status"], "missing");
        assert!(
            s["detail"]
                .as_str()
                .unwrap()
                .contains("hoist is not installed")
        );
        assert!(
            s["fix"]
                .as_str()
                .unwrap()
                .contains("npm install -g @keelstackhq/hoist")
        );
    }
}

#[test]
fn garbage_output_marks_both_steps() {
    let tmp = tempfile::tempdir().unwrap();
    let h = fake_hoist(tmp.path(), "not json", 0);
    let repo = repo_with_checks();
    let (_, v) = run(&h, repo.path(), true);
    assert_eq!(step(&v, 3)["status"], "missing");
    assert_eq!(step(&v, 5)["status"], "missing");
}

#[test]
fn without_remote_hoist_is_never_called() {
    let tmp = tempfile::tempdir().unwrap();
    let h = fake_hoist(tmp.path(), &report(CHECK_DRIFT, false, 1), 1);
    let repo = repo_with_checks();
    let (_, v) = run(&h, repo.path(), false);
    assert!(v["data"].get("hoist").is_none());
    assert!(!tmp.path().join("hoist-argv.log").exists());
}

#[test]
fn npm_drift_goes_to_step_ten_not_step_three() {
    let tmp = tempfile::tempdir().unwrap();
    let h = fake_hoist(
        tmp.path(),
        &report(&format!("{OK_ITEM},{NPM_DRIFT}"), false, 1),
        1,
    );
    let repo = repo_with_checks();
    let (_, v) = run(&h, repo.path(), true);
    assert_eq!(step(&v, 3)["status"], "ok");
    let s10 = step(&v, 10);
    assert!(
        s10["detail"]
            .as_str()
            .unwrap()
            .contains("npm/first_publish")
    );
    assert!(
        s10["fix"]
            .as_str()
            .unwrap()
            .contains("hoist apply --area npm")
    );
}

#[test]
fn ruleset_drift_and_failed_verify_are_step_three_drift() {
    let tmp = tempfile::tempdir().unwrap();
    let h = fake_hoist(tmp.path(), &report(RULESET_DRIFT, false, 1), 1);
    let repo = repo_with_checks();
    let (_, v) = run(&h, repo.path(), true);
    assert_eq!(step(&v, 3)["status"], "missing");
    assert!(
        step(&v, 3)["fix"]
            .as_str()
            .unwrap()
            .contains("hoist apply --area github")
    );
    let h = fake_hoist(tmp.path(), &report(VERIFY_BAD, false, 1), 1);
    let (_, v) = run(&h, repo.path(), true);
    assert!(
        step(&v, 3)["detail"]
            .as_str()
            .unwrap()
            .contains("bypass actor present")
    );
}

#[test]
fn unknown_only_does_not_claim_no_drift() {
    let tmp = tempfile::tempdir().unwrap();
    let h = fake_hoist(tmp.path(), &report(PERMS_UNKNOWN, false, 1), 1);
    let repo = repo_with_checks();
    let (_, v) = run(&h, repo.path(), true);
    let s3 = step(&v, 3);
    assert_eq!(s3["status"], "ok");
    let d = s3["detail"].as_str().unwrap();
    assert!(
        d.contains("hoist could not read 1 item(s): actions_permissions/default"),
        "{d}"
    );
    assert!(!d.contains("no drift"));
}

#[test]
fn exit_two_with_json_says_so() {
    let tmp = tempfile::tempdir().unwrap();
    let h = fake_hoist(tmp.path(), &report(OK_ITEM, true, 0), 2);
    let repo = repo_with_checks();
    let (_, v) = run(&h, repo.path(), true);
    assert!(
        step(&v, 3)["detail"]
            .as_str()
            .unwrap()
            .contains("exited with 2")
    );
}

#[test]
fn a_hung_hoist_is_killed_after_the_timeout() {
    let tmp = tempfile::tempdir().unwrap();
    let bin = tmp.path().join("slow-hoist");
    std::fs::write(&bin, "#!/bin/sh\nexec sleep 30\n").unwrap();
    std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
    let repo = repo_with_checks();
    let fake = FakeGithub::start();
    let started = std::time::Instant::now();
    let out = bosun()
        .env("BOSUN_GITHUB_API", fake.url())
        .env("BOSUN_HOIST", &bin)
        .env("BOSUN_HOIST_TIMEOUT_SECS", "1")
        .arg("-C")
        .arg(repo.path())
        .args(["doctor", "--remote"])
        .output()
        .unwrap();
    assert!(started.elapsed().as_secs() < 15);
    let v = one_object(&out.stdout);
    for n in [3, 5] {
        assert!(
            step(&v, n)["detail"]
                .as_str()
                .unwrap()
                .contains("hoist check timed out after 1s")
        );
    }
}
