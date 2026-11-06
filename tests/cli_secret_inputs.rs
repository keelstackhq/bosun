//! Every free-text input bosun stores or posts refuses secret-shaped text (spec §6): exit 2,
//! the error names the field, never the secret, and nothing is written to GitHub.

mod common;

use common::bosun;
use common::fake::FakeGithub;
use common::one_object;

fn secret() -> String {
    format!("ghp_{}", "a1B2".repeat(9))
}

/// Run `args` with `stdin`; assert the refusal and that the fake saw no write.
fn refused(args: &[&str], stdin: &str, field: &str) {
    let fake = FakeGithub::start();
    let out = bosun()
        .env("BOSUN_GITHUB_API", fake.url())
        .env("BOSUN_SESSION", "session_test")
        .env("BOSUN_NOW", "2026-10-22T10:00:00Z")
        .args(args)
        .write_stdin(stdin.to_string())
        .output()
        .unwrap();
    let all =
        String::from_utf8_lossy(&out.stdout).into_owned() + &String::from_utf8_lossy(&out.stderr);
    let v = one_object(&out.stdout);
    assert_eq!(out.status.code(), Some(2), "{args:?}: {v}");
    let message = v["error"]["message"].as_str().unwrap();
    assert!(message.contains(field), "{args:?}: {message}");
    assert!(message.contains("GitHub token"), "{args:?}: {message}");
    assert!(v["error"]["fix"].as_str().unwrap().contains("redact"));
    assert!(!all.contains(&secret()), "{args:?} echoed the secret");
    let writes: Vec<_> = fake
        .requests()
        .into_iter()
        .filter(|r| r.method != "GET")
        .collect();
    assert!(writes.is_empty(), "{args:?} wrote to GitHub");
}

#[test]
fn lesson_add_scope_and_evidence() {
    let s = secret();
    refused(
        &["lesson", "add", "rule", "--scope", &s, "--evidence", "#1"],
        "",
        "--scope",
    );
    refused(
        &["lesson", "add", "rule", "--evidence", &s],
        "",
        "--evidence",
    );
}

#[test]
fn lesson_capture_title_and_failure_text() {
    let s = secret();
    refused(
        &["lesson", "capture", "--kind", "flake", "--title", &s],
        "boom\n",
        "--title",
    );
    refused(
        &["lesson", "capture", "--kind", "flake"],
        &format!("boom {s}\n"),
        "the failure text",
    );
}

#[test]
fn lesson_triage_rule_and_scope() {
    let s = secret();
    let args = ["lesson", "triage", "5", "--as", "advice"];
    refused(&[&args[..], &["--rule", &s]].concat(), "", "--rule");
    refused(
        &[&args[..], &["--rule", "r", "--scope", &s]].concat(),
        "",
        "--scope",
    );
}

#[test]
fn ask_hold_and_answer_text() {
    let s = secret();
    refused(&["ask", "5", &s], "", "the question");
    refused(&["hold", "5", &s], "", "the hold text");
    refused(
        &["answer", "5", "--slack-user", "U01ABC23"],
        &s,
        "the answer",
    );
}

#[test]
fn intake_text_and_title() {
    let s = secret();
    refused(&["intake", &s, "--source", "human"], "", "the intake text");
    refused(
        &["intake", "ok", "--source", "human", "--title", &s],
        "",
        "--title",
    );
}

#[test]
fn claim_touch() {
    let s = secret();
    refused(&["claim", "5", "--touch", &s], "", "--touch");
}

#[test]
fn pr_title() {
    let s = secret();
    let dir = tempfile::tempdir().unwrap();
    let git = |args: &[&str]| {
        let st = std::process::Command::new("git")
            .arg("-C")
            .arg(dir.path())
            .args(["-c", "user.name=t", "-c", "user.email=t@t"])
            .args(args)
            .status()
            .unwrap();
        assert!(st.success());
    };
    git(&["init", "-q", "-b", "claude/t7"]);
    git(&["commit", "-q", "--allow-empty", "-m", "x"]);
    let fake = FakeGithub::start();
    let label = format!("bosun:state/{}", "building");
    fake.issue(7, "the task", &[label.as_str()], "Do it.");
    let out = bosun()
        .env("BOSUN_GITHUB_API", fake.url())
        .arg("-C")
        .arg(dir.path())
        .args(["pr", "7", "--title", &s])
        .output()
        .unwrap();
    let all =
        String::from_utf8_lossy(&out.stdout).into_owned() + &String::from_utf8_lossy(&out.stderr);
    let v = one_object(&out.stdout);
    assert_eq!(out.status.code(), Some(2), "{v}");
    assert!(v["error"]["message"].as_str().unwrap().contains("title"));
    assert!(!all.contains(&s), "echoed the secret");
    assert!(fake.requests().iter().all(|r| r.method == "GET"));
}
