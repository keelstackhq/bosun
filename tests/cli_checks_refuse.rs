//! `[checks] mode = "checks"`: what a session's or the owner's token cannot post, and what
//! doctor checks (spec §15, B2/T5).

mod common;

use common::fake::FakeGithub;
use common::{bosun, one_object};
use serde_json::Value;

const HEAD: &str = "sha1";
const CHECKS: &str = "[checks]\nmode = \"checks\"\napp_id = 4242\napp_slug = \"gate\"\n";
const ALL: &str = "\"ci-ok\", \"risk-check\", \"review-verdict\", \"main-open\"";

/// A repo dir with `bosun_toml`, `hoist.toml` [github] `required_checks = [<checks>]` and a ci-ok job.
fn repo(bosun_toml: &str, checks: &str) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join(".bosun")).unwrap();
    std::fs::write(dir.path().join(".bosun/bosun.toml"), bosun_toml).unwrap();
    let hoist = format!("[github]\nrequired_checks = [{checks}]\n");
    std::fs::write(dir.path().join("hoist.toml"), hoist).unwrap();
    std::fs::create_dir_all(dir.path().join(".github/workflows")).unwrap();
    std::fs::write(
        dir.path().join(".github/workflows/ci.yml"),
        "jobs:\n  ci-ok:\n    runs-on: x\n",
    )
    .unwrap();
    dir
}

fn bosun_in(fake: &FakeGithub, dir: &tempfile::TempDir, args: &[&str]) -> (i32, Value) {
    let out = bosun()
        .env("BOSUN_GITHUB_API", fake.url())
        .env("BOSUN_OWNER_TOKEN", "owner-canary")
        .env("BOSUN_HOIST", "/nonexistent/hoist")
        .env("BOSUN_HTTP_MAX_SLEEP_MS", "0")
        .arg("-C")
        .arg(dir.path())
        .args(args)
        .output()
        .unwrap();
    let v = one_object(&out.stdout);
    if v["ok"] == false {
        assert!(!v["error"]["fix"].as_str().unwrap().trim().is_empty());
    }
    (out.status.code().unwrap(), v)
}

fn pr() -> FakeGithub {
    let fake = FakeGithub::start();
    fake.pull(1, "change", "feature", "No link.");
    fake.pull_as_issue(1, "change");
    fake.pull_head_sha(1, HEAD);
    fake.pull_file(
        1,
        ".github/workflows/main-open.yml",
        "modified",
        1,
        0,
        Some("@@"),
    );
    fake
}

#[test]
fn verdict_post_in_checks_mode_is_refused_with_verdict_comment_as_the_fix() {
    let fake = pr();
    let dir = repo(CHECKS, ALL);
    let file = dir.path().join("v.json");
    let json = format!(r#"{{"pr":1,"head_sha":"{HEAD}","verdict":"approve","confidence":0.95}}"#);
    std::fs::write(&file, json).unwrap();
    let (code, v) = bosun_in(
        &fake,
        &dir,
        &["verdict", "post", "--from", file.to_str().unwrap()],
    );
    assert_eq!(code, 1, "{v}");
    assert!(
        v["error"]["fix"]
            .as_str()
            .unwrap()
            .contains("bosun verdict comment"),
        "{v}"
    );
    assert!(fake.statuses_of(HEAD).is_empty() && fake.check_runs_of(HEAD).is_empty());
    assert!(fake.comments_of(1).is_empty());
}

#[test]
fn approve_main_open_in_checks_mode_is_a_conflict_that_names_the_workflow() {
    let fake = pr();
    let (code, v) = bosun_in(&fake, &repo(CHECKS, ALL), &["approve", "--main-open", "1"]);
    assert_eq!(code, 3, "{v}");
    assert!(
        v["error"]["fix"]
            .as_str()
            .unwrap()
            .contains("re-run the main-open workflow"),
        "{v}"
    );
    assert!(fake.statuses_of(HEAD).is_empty());
}

fn step3(v: &Value) -> Value {
    (v["data"]["items"].as_array().unwrap().iter())
        .find(|i| i["step"] == 3)
        .cloned()
        .unwrap()
}

#[test]
fn doctor_reads_the_table_form_of_required_checks() {
    let fake = FakeGithub::start();
    fake.enforce_checks(&["ci-ok", "risk-check", "review-verdict", "main-open"]);
    let tables = "\"ci-ok\", { context = \"risk-check\", app = \"gate\" }, \
        { context = \"review-verdict\", app = \"gate\" }, { context = \"main-open\", integration_id = 4242 }";
    let (_, v) = bosun_in(&fake, &repo("", tables), &["doctor"]);
    let s = step3(&v);
    assert_eq!(s["status"], "ok", "{s}");
}

#[test]
fn doctor_says_when_required_checks_cannot_be_read() {
    let fake = FakeGithub::start();
    fake.enforce_checks(&["ci-ok", "risk-check", "review-verdict", "main-open"]);
    let bad = format!("{ALL}, {{ context = \"x\", colour = \"red\" }}");
    let (_, v) = bosun_in(&fake, &repo("", &bad), &["doctor"]);
    let s = step3(&v);
    assert_eq!(s["status"], "missing", "{s}");
    assert!(
        s["detail"].as_str().unwrap().contains("cannot be read"),
        "{s}"
    );
}

#[test]
fn doctor_remote_warns_when_a_gate_is_not_pinned_to_the_app() {
    let fake = FakeGithub::start();
    fake.enforce_checks(&[
        "ci-ok@15368",
        "risk-check@4242",
        "review-verdict",
        "main-open@99",
    ]);
    let (_, v) = bosun_in(&fake, &repo(CHECKS, ALL), &["doctor", "--remote"]);
    let s = step3(&v);
    let detail = s["detail"].as_str().unwrap();
    assert!(
        detail.contains("review-verdict") && detail.contains("main-open"),
        "{s}"
    );
    assert!(!detail.contains("\"risk-check\""), "{s}");

    fake.enforce_checks(&[
        "ci-ok@15368",
        "risk-check@4242",
        "review-verdict@4242",
        "main-open@4242",
    ]);
    let (_, v) = bosun_in(&fake, &repo(CHECKS, ALL), &["doctor", "--remote"]);
    let detail = step3(&v)["detail"].as_str().unwrap().to_string();
    assert!(!detail.contains("not pinned"), "{detail}");
}

#[test]
fn doctor_remote_warns_when_the_token_can_read_the_actions_secrets() {
    let fake = FakeGithub::start();
    fake.enforce_checks(&[
        "ci-ok@15368",
        "risk-check@4242",
        "review-verdict@4242",
        "main-open@4242",
    ]);
    fake.present("actions/secrets");
    let (_, v) = bosun_in(&fake, &repo(CHECKS, ALL), &["doctor", "--remote"]);
    let detail = step3(&v)["detail"].as_str().unwrap().to_string();
    assert!(detail.contains("Actions secrets"), "{detail}");
}

#[test]
fn statuses_mode_checks_no_pins() {
    let fake = FakeGithub::start();
    fake.enforce_checks(&["ci-ok", "risk-check", "review-verdict", "main-open"]);
    let (_, v) = bosun_in(&fake, &repo("", ALL), &["doctor", "--remote"]);
    let detail = step3(&v)["detail"].as_str().unwrap().to_string();
    assert!(!detail.contains("not pinned"), "{detail}");
}
