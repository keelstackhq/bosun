mod common;

use common::fake::FakeGithub;
use common::run;

const NOW: &str = "2026-10-22T11:00:00Z";

fn verdict_comment(head: &str, minor: &str) -> String {
    format!(
        "Review verdict.\n\n<!-- bosun:verdict -->\n```json\n{{\"pr\":20,\"head_sha\":\"{head}\",\"verdict\":\"approve\",\"confidence\":0.9,\"findings\":[{{\"severity\":\"minor\",\"file\":\"src/a.rs\",\"line\":4,\"claim\":\"{minor}\",\"evidence\":\"e\"}},{{\"severity\":\"nit\",\"file\":\"src/b.rs\",\"claim\":\"a nit\",\"evidence\":\"e\"}}]}}\n```"
    )
}

fn pr_with_note(fake: &FakeGithub) {
    fake.pull(
        20,
        "feat: x",
        "claude/x",
        "Closes #10\n\n<!-- bosun:risk -->\n## Risk: high\n\n- **gov**: why\n  - `.claude/settings.json`\n  - `src/hooks/edit.rs`\n\nOwner: review.\n<!-- /bosun:risk -->",
    );
}

#[test]
fn handoff_pr_returns_a_message_to_the_registered_parent() {
    let fake = FakeGithub::start();
    fake.issue(1, "bosun registry", &["bosun:registry"], "");
    fake.comment(1, r#"{"bosun":"join","session":"child_c","origin":"spawned","parent":"root_a","depth":2,"expires":"2026-10-22T11:20:00Z"}"#);
    pr_with_note(&fake);
    fake.comment(20, &verdict_comment("sha20", "name is vague"));
    fake.comment(20, &verdict_comment("old", "stale minor"));

    let (code, v) = run(&fake, "child_c", NOW, &["handoff-pr", "20"]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["command"], "handoff-pr");
    assert_eq!(v["data"]["head_sha"], "sha20");
    assert_eq!(
        v["data"]["risk_paths"],
        serde_json::json!([".claude/settings.json", "src/hooks/edit.rs"])
    );
    assert_eq!(
        v["data"]["review_minors"],
        serde_json::json!(["src/a.rs:4 `name is vague`"])
    );
    let next = &v["next"][0];
    assert_eq!(next["tool"], "send_message", "{v}");
    let args = &next["args"];
    assert_eq!(args["session_id"], "root_a");
    let msg = args["message"].as_str().unwrap();
    assert!(msg.contains("PR #20") && msg.contains("sha20"), "{msg}");
    assert!(
        msg.contains("src/hooks/edit.rs") && msg.contains("name is vague"),
        "{msg}"
    );
    assert!(!msg.contains("stale minor"), "{msg}");
}

#[test]
fn a_wave_agent_id_names_its_orchestrator_and_to_overrides() {
    let fake = FakeGithub::start();
    pr_with_note(&fake);
    let (_, v) = run(&fake, "root_a/T3", NOW, &["handoff-pr", "20"]);
    assert_eq!(v["next"][0]["args"]["session_id"], "root_a", "{v}");
    assert_eq!(v["data"]["review_minors"], serde_json::json!([]));
    let (_, v) = run(
        &fake,
        "root_a/T3",
        NOW,
        &["handoff-pr", "20", "--to", "other"],
    );
    assert_eq!(v["next"][0]["args"]["session_id"], "other", "{v}");
}

#[test]
fn without_a_parent_it_is_a_usage_error_with_a_fix() {
    let fake = FakeGithub::start();
    pr_with_note(&fake);
    let (code, v) = run(&fake, "lone", NOW, &["handoff-pr", "20"]);
    assert_eq!(code, 2, "{v}");
    assert!(v["error"]["fix"].as_str().unwrap().contains("--to"));
}

#[test]
fn a_missing_pr_is_an_error() {
    let fake = FakeGithub::start();
    let (code, v) = run(&fake, "root_a/T3", NOW, &["handoff-pr", "99"]);
    assert_eq!(code, 1, "{v}");
}
