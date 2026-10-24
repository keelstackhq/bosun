//! Use cases of M8 (spec §12): review mode (b).
mod common;

use common::anthropic::FakeAnthropic;
use common::bosun;
use common::fake::FakeGithub;
use serde_json::json;

/// UC-M8-1: with review.mode = "ci", the review-verdict check runs with no session.
#[test]
fn uc_m8_1() {
    let gh = FakeGithub::start();
    gh.pull_as_issue(42, "the pr");
    gh.pull(42, "the pr", "claude/x", "no task");
    gh.pull_file(
        42,
        "src/a.rs",
        "modified",
        1,
        0,
        Some("@@ -1 +1 @@\n+fn a() {}"),
    );
    let ai = FakeAnthropic::start(
        &json!({"pr": 42, "head_sha": "sha42", "verdict": "approve", "confidence": 0.93,
            "findings": [], "lessons_hit": [], "escalate_risk": false})
        .to_string(),
    );
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join(".bosun")).unwrap();
    std::fs::write(
        dir.path().join(".bosun/bosun.toml"),
        "[review]\nmode = \"ci\"\n",
    )
    .unwrap();

    // No BOSUN_SESSION and no Claude Code variables: only the CI environment.
    let out = bosun()
        .current_dir(dir.path())
        .env_remove("BOSUN_SESSION")
        .env_remove("CLAUDE_CODE_REMOTE_SESSION_ID")
        .env_remove("CLAUDE_CODE_CHILD_SESSION")
        .env_remove("CLAUDE_CODE_ENTRYPOINT")
        .env("BOSUN_GITHUB_API", gh.url())
        .env("BOSUN_ANTHROPIC_API", ai.url())
        .env("ANTHROPIC_API_KEY", "sk-ant-uc-canary")
        .args(["review-ci", "42"])
        .output()
        .unwrap();
    let v = common::one_object(&out.stdout);
    assert_eq!(out.status.code(), Some(0), "{v}");
    assert_eq!(v["data"]["approved"], true);
    let st = gh.statuses_of("sha42");
    assert_eq!(st.len(), 1);
    assert_eq!(st[0]["context"], "review-verdict");
    assert_eq!(st[0]["state"], "success");
    assert_eq!(ai.requests().len(), 1);
}
