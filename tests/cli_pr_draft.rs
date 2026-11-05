//! Draft-first PRs: `bosun pr` opens a draft; `--ready` (alias `--auto-merge`) needs the head to
//! be the local HEAD and every ready gate stamped for its tree, then marks it ready and turns on
//! auto-merge; `hold` and `ask` put the task's open PRs back to draft.

mod common;

use std::path::Path;
use std::process::Command;

use common::fake::FakeGithub;
use common::stamps;
use common::{bosun, one_object};
use serde_json::Value;

const CCR_MSG: &str = "GitHub GraphQL is not available from Claude Code sessions; use the REST API (gh api repos/{owner}/{repo}/...).";
const FOUR: [&str; 4] = ["ci-ok", "risk-check", "review-verdict", "main-open"];
const OWNERS: &str = "[[owner]]\ngithub = \"ada\"\n";

fn repo(branch: &str) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    for args in [
        vec!["init", "-q", "-b", branch],
        vec![
            "-c",
            "user.name=t",
            "-c",
            "user.email=t@t",
            "commit",
            "-q",
            "--allow-empty",
            "-m",
            "x",
        ],
    ] {
        let st = Command::new("git")
            .arg("-C")
            .arg(dir.path())
            .args(&args)
            .status()
            .unwrap();
        assert!(st.success());
    }
    dir
}

fn bosun_in(fake: &FakeGithub, dir: &Path, args: &[&str]) -> (i32, Value) {
    let out = bosun()
        .env("BOSUN_GITHUB_API", fake.url())
        .arg("-C")
        .arg(dir)
        .args(args)
        .output()
        .unwrap();
    (out.status.code().unwrap(), one_object(&out.stdout))
}

/// Task 7 in review with PR 20 (a draft, as `bosun pr` leaves it), checks green.
fn setup() -> FakeGithub {
    let fake = FakeGithub::start();
    fake.issue(7, "the task", &["bosun:state/review"], "Do it.");
    fake.enforce_checks(&FOUR);
    fake.pull(20, "t", "claude/t7", "Closes #7");
    fake.pull_set(20, "draft", true.into());
    fake
}

fn graphql_calls(fake: &FakeGithub) -> usize {
    fake.requests()
        .iter()
        .filter(|r| r.path.ends_with("/graphql"))
        .count()
}

#[test]
fn a_new_pr_opens_as_a_draft() {
    let fake = FakeGithub::start();
    fake.issue(7, "the task", &["bosun:state/building"], "Do it.");
    let dir = repo("claude/t7");
    let (code, v) = bosun_in(&fake, dir.path(), &["pr", "7"]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["created"], true);
    assert_eq!(v["data"]["draft"], true);
    let n = v["data"]["pr"].as_u64().unwrap();
    assert!(fake.pull_is_draft(n));
    let create = fake
        .requests()
        .into_iter()
        .find(|r| r.method == "POST" && r.path.ends_with("/pulls"))
        .unwrap();
    let body: Value = serde_json::from_str(create.body.as_deref().unwrap()).unwrap();
    assert_eq!(body["draft"], true);
    // A rerun finds the same draft and leaves it one.
    let (code, v) = bosun_in(&fake, dir.path(), &["pr", "7"]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["draft"], true);
    assert_eq!(v["data"]["auto_merge"], false);
}

#[test]
fn ready_without_stamps_names_the_missing_gates_and_changes_nothing() {
    let fake = setup();
    let dir = repo("claude/t7");
    fake.pull_head_sha(20, &stamps::head_sha(dir.path()));
    stamps::stamp(dir.path(), &["test-integrity", "fmt"]);
    let (code, v) = bosun_in(&fake, dir.path(), &["pr", "7", "--ready"]);
    assert_eq!(code, 1, "{v}");
    assert_eq!(
        v["data"]["missing_gates"],
        serde_json::json!(["lint", "test", "deps"])
    );
    let fix = v["error"]["fix"].as_str().unwrap();
    assert!(
        fix.contains("for g in lint test deps; do bosun gate $g || break; done"),
        "{fix}"
    );
    assert!(fake.pull_is_draft(20));
    assert_eq!(graphql_calls(&fake), 0);
    assert_eq!(fake.auto_merge_enabled(20), None);
    assert!(fake.labels_of(7).contains(&"bosun:state/review".into()));
}

#[test]
fn ready_with_a_head_that_is_not_local_head_is_a_conflict() {
    let fake = setup();
    let dir = repo("claude/t7");
    stamps::stamp_all(dir.path());
    // The fake PR head is `sha20`, not the local commit.
    let (code, v) = bosun_in(&fake, dir.path(), &["pr", "7", "--ready"]);
    assert_eq!(code, 3, "{v}");
    let fix = v["error"]["fix"].as_str().unwrap();
    assert!(fix.contains("push") && fix.contains("pull"), "{fix}");
    assert!(fake.pull_is_draft(20));
    assert_eq!(graphql_calls(&fake), 0);
}

#[test]
fn ready_with_every_stamp_marks_ready_and_turns_on_auto_merge() {
    let fake = setup();
    let dir = repo("claude/t7");
    stamps::ready(&fake, dir.path(), 20);
    let (code, v) = bosun_in(&fake, dir.path(), &["pr", "7", "--ready"]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["ready"], true);
    assert_eq!(v["data"]["draft"], false);
    assert_eq!(v["data"]["auto_merge"], true);
    assert_eq!(v["data"]["task_state"], "queued");
    assert!(!fake.pull_is_draft(20));
    assert_eq!(fake.auto_merge_enabled(20).as_deref(), Some("SQUASH"));
}

#[test]
fn auto_merge_is_an_alias_of_ready() {
    let fake = setup();
    let dir = repo("claude/t7");
    let (code, v) = bosun_in(&fake, dir.path(), &["pr", "7", "--auto-merge"]);
    assert_eq!(code, 3, "{v}");
    stamps::ready(&fake, dir.path(), 20);
    let (code, v) = bosun_in(&fake, dir.path(), &["pr", "7", "--auto-merge"]);
    assert_eq!(code, 0, "{v}");
    assert!(!fake.pull_is_draft(20));
    assert_eq!(fake.auto_merge_enabled(20).as_deref(), Some("SQUASH"));
}

#[test]
fn a_ready_pr_skips_the_graphql_draft_call_but_still_needs_the_stamps() {
    let fake = setup();
    fake.pull_set(20, "draft", false.into());
    let dir = repo("claude/t7");
    let (code, v) = bosun_in(&fake, dir.path(), &["pr", "7", "--ready"]);
    assert_eq!(code, 3, "{v}");
    stamps::ready(&fake, dir.path(), 20);
    let (code, v) = bosun_in(&fake, dir.path(), &["pr", "7", "--ready"]);
    assert_eq!(code, 0, "{v}");
    let drafts = fake
        .requests()
        .iter()
        .filter(|r| {
            r.path.ends_with("/graphql")
                && r.body
                    .as_deref()
                    .is_some_and(|b| b.contains("markPullRequestReadyForReview"))
        })
        .count();
    assert_eq!(drafts, 0);
    assert_eq!(fake.auto_merge_enabled(20).as_deref(), Some("SQUASH"));
}

#[test]
fn a_ccr_refusal_leaves_the_pr_draft_and_hands_back_the_mcp_call() {
    let fake = setup();
    fake.graphql_status(403, CCR_MSG);
    let dir = repo("claude/t7");
    stamps::ready(&fake, dir.path(), 20);
    let (code, v) = bosun_in(&fake, dir.path(), &["pr", "7", "--ready"]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["ready"], "pending");
    assert_eq!(v["data"]["auto_merge"], false);
    assert_eq!(fake.auto_merge_enabled(20), None);
    assert!(fake.pull_is_draft(20));
    assert!(fake.labels_of(7).contains(&"bosun:state/review".into()));
    let next = &v["next"][0];
    assert_eq!(next["tool"], "mcp__github__update_pull_request");
    assert_eq!(next["args"]["pullNumber"], 20);
    assert_eq!(next["args"]["draft"], false);
    assert_eq!(next["args"]["owner"], "acme");
    assert_eq!(next["args"]["repo"], "widget");
    assert!(
        v["data"]["note"]
            .as_str()
            .unwrap()
            .contains("bosun pr 7 --ready")
    );
}

#[test]
fn hold_and_ask_put_the_task_pr_back_to_draft() {
    let fake = setup();
    fake.pull_set(20, "draft", false.into());
    fake.pull(21, "other", "claude/other", "Closes #99");
    fake.content("governance/owners.toml", "main", OWNERS);
    let dir = repo("claude/t7");
    let (code, v) = bosun_in(&fake, dir.path(), &["ask", "7", "May I?"]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["drafted"], serde_json::json!([20]));
    assert!(fake.pull_is_draft(20));
    assert!(!fake.pull_is_draft(21));
    // Already a draft: nothing to do.
    let (code, v) = bosun_in(&fake, dir.path(), &["ask", "7", "Again?"]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["drafted"], serde_json::json!([]));
}

#[test]
fn ask_under_a_ccr_refusal_reports_the_mcp_call_and_does_not_fail() {
    let fake = setup();
    fake.pull_set(20, "draft", false.into());
    fake.content("governance/owners.toml", "main", OWNERS);
    fake.graphql_status(403, CCR_MSG);
    let dir = repo("claude/t7");
    let (code, v) = bosun_in(&fake, dir.path(), &["ask", "7", "May I?"]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["drafted"], serde_json::json!([]));
    assert!(v["data"]["pr_draft_error"].is_string(), "{v}");
    let next = &v["next"][0];
    assert_eq!(next["tool"], "mcp__github__update_pull_request");
    assert_eq!(next["args"]["pullNumber"], 20);
    assert_eq!(next["args"]["draft"], true);
    assert!(
        fake.labels_of(7)
            .contains(&"bosun:state/needs-human".into())
    );
}

const PLAN: &str = "# Plan: Ship it\n\nproject: x\n\n## M1: One\n\narea: core\ndepends: seed\n\nWords.\n\n\
### Tasks\n- T1: a\n  touch: docs/a.md\n### Done when\n- UC-M1-1: works\n";

#[test]
fn hold_puts_the_task_pr_back_to_draft() {
    let fake = FakeGithub::start();
    fake.issue(1, "bosun registry", &["bosun:registry"], "");
    let epic = r#"<!-- bosun:epic {"item":"M1","plan":"plans/x.md"} -->"#;
    fake.issue(10, "Epic", &["bosun:state/ready"], epic);
    let task = r#"<!-- bosun:task {"item":"M1","task":"T1","epic":10,"touch":["src/T1/**"]} -->"#;
    fake.issue(11, "Task T1", &["bosun:state/ready"], task);
    fake.content("plans/x.md", "main", PLAN);
    fake.pull(30, "t", "claude/t11", "Closes #11");
    let tmp = tempfile::tempdir().unwrap();
    let d = tmp.path();
    let (code, v) = bosun_in(&fake, d, &["claim", "11"]);
    assert_eq!(code, 0, "{v}");
    let (code, v) = bosun_in(&fake, d, &["hold", "11", "Create the npm token"]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["drafted"], serde_json::json!([30]));
    assert!(fake.pull_is_draft(30));
}
