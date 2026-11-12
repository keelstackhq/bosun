//! `bosun heal`: red PR heads get a notify or a fixer, at most 2 rounds, then needs-human.
mod common;

use common::fake::FakeGithub;
use common::{bosun, lease_note, one_object};
use serde_json::Value;

const NOW: &str = "2026-11-12T12:00:00Z";
const LIVE: &str = "2026-11-12T12:20:00Z";
const DEAD: &str = "2026-11-12T11:00:00Z";
const ACTIONS: &str = "github-actions";

/// Task #7 with a lease (live or dead) and its PR #30 at head `sha30`, failing `ci-ok`.
fn red_pr(lease: &str) -> FakeGithub {
    let fake = FakeGithub::start();
    fake.issue(1, "bosun registry", &["bosun:registry"], "");
    fake.comment(
        1,
        &format!(
            r#"{{"bosun":"join","session":"session_test","origin":"local","depth":1,"expires":"{LIVE}"}}"#
        ),
    );
    fake.issue(7, "Task seven", &["bosun:state/review"], "");
    fake.comment(7, &lease_note("session_w", lease, "src/x.rs"));
    fake.pull(30, "PR 30", "feat/7", "Closes #7");
    fake.pull_as_issue(30, "PR 30");
    fake.pull_commit_shas(30, &["sha30"]);
    fake.check_run_by("sha30", "ci", "failure", ACTIONS);
    fake.check_run_by("sha30", "ci-ok", "failure", ACTIONS);
    fake.run_with_job("sha30", "ci", "failure", "error[E0308]: mismatched types");
    fake
}

fn heal_in(fake: &FakeGithub, toml: Option<&str>, args: &[&str]) -> (i32, Value) {
    let dir = tempfile::tempdir().unwrap();
    if let Some(t) = toml {
        std::fs::create_dir_all(dir.path().join(".bosun")).unwrap();
        std::fs::write(dir.path().join(".bosun/bosun.toml"), t).unwrap();
        fake.content(".bosun/bosun.toml", "main", t);
    }
    let out = bosun()
        .current_dir(dir.path())
        .env("BOSUN_GITHUB_API", fake.url())
        .env("BOSUN_NOW", NOW)
        .arg("heal")
        .args(args)
        .output()
        .unwrap();
    (out.status.code().unwrap(), one_object(&out.stdout))
}

const STEADY: &str = "[mode]\nname = \"steady\"\nbrake = \"off\"\n";
const BOOSTER: &str = "[mode]\nname = \"booster\"\nbrake = \"off\"\n";
const BOOSTER_BRAKE: &str = "[mode]\nname = \"booster\"\n";

fn actions(v: &Value) -> Vec<String> {
    let rows = v["data"]["prs"].as_array().unwrap();
    rows.iter()
        .map(|r| r["action"].as_str().unwrap().to_string())
        .collect()
}

#[test]
fn gatekeeper_is_a_no_op_that_reads_nothing() {
    let fake = red_pr(LIVE);
    let before = fake.requests().len();
    let (code, v) = heal_in(&fake, None, &[]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["knob"], "off");
    assert_eq!(v["next"], serde_json::json!([]));
    assert_eq!(fake.requests().len(), before);
}

#[test]
fn notify_sends_the_holder_the_failing_gate_once_per_head() {
    let fake = red_pr(LIVE);
    let (code, v) = heal_in(&fake, Some(STEADY), &[]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(actions(&v), ["notify"]);
    let next = &v["next"][0];
    assert_eq!(next["tool"], "send_message");
    assert_eq!(next["args"]["session_id"], "session_w");
    let msg = next["args"]["message"].as_str().unwrap();
    assert!(
        msg.contains("#30") && msg.contains("bosun ci-log 30") && msg.contains("ci"),
        "{msg}"
    );
    assert!(msg.chars().count() <= 500);
    let marks = fake.comments_of(30);
    assert!(
        marks
            .iter()
            .any(|c| c.body.contains("bosun:heal head=sha30 round=1")),
        "{marks:?}"
    );
    let (_, again) = heal_in(&fake, Some(STEADY), &[]);
    assert_eq!(actions(&again), ["skip"]);
    assert_eq!(again["next"], serde_json::json!([]));
}

#[test]
fn a_dead_holder_gets_a_fixer_only_with_spawn() {
    let fake = red_pr(DEAD);
    let (_, v) = heal_in(&fake, Some(STEADY), &[]);
    assert_eq!(actions(&v), ["skip"]);
    assert_eq!(v["next"], serde_json::json!([]));
    let (code, v) = heal_in(&fake, Some(BOOSTER), &[]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(actions(&v), ["spawn"]);
    let next = &v["next"][0];
    assert_eq!(next["tool"], "create_session");
    let prompt = next["args"]["prompt"].as_str().unwrap();
    assert!(
        prompt.contains("#30") && prompt.contains("Do not claim"),
        "{prompt}"
    );
    assert!(
        prompt.contains("bosun join --parent session_test --depth 2"),
        "{prompt}"
    );
    // This session took the dead lease for the fixer, once. The same head is not healed again;
    // a new red head is round 2 (a new fixer, no second lease), then needs-human.
    let mine = |f: &FakeGithub| {
        let c = f.comments_of(7);
        c.iter()
            .filter(|c| c.body.contains("\"session\":\"session_test\""))
            .count()
    };
    assert_eq!(mine(&fake), 1);
    let (_, v) = heal_in(&fake, Some(BOOSTER), &[]);
    assert_eq!(actions(&v), ["skip"], "{v}");
    for (head, want) in [("sha30b", "spawn"), ("sha30c", "needs-human")] {
        fake.pull_head_sha(30, head);
        fake.check_run_by(head, "ci", "failure", ACTIONS);
        fake.check_run_by(head, "ci-ok", "failure", ACTIONS);
        let (_, v) = heal_in(&fake, Some(BOOSTER), &[]);
        assert_eq!(actions(&v), [want], "{v}");
    }
    assert_eq!(mine(&fake), 1);
}

#[test]
fn a_local_edit_cannot_lift_the_brake() {
    let fake = red_pr(DEAD);
    fake.content(".bosun/bosun.toml", "main", BOOSTER_BRAKE);
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join(".bosun")).unwrap();
    std::fs::write(dir.path().join(".bosun/bosun.toml"), BOOSTER).unwrap();
    let out = bosun()
        .current_dir(dir.path())
        .env("BOSUN_GITHUB_API", fake.url())
        .env("BOSUN_NOW", NOW)
        .arg("heal")
        .output()
        .unwrap();
    let v = one_object(&out.stdout);
    assert_eq!(v["data"]["knob"], "notify", "{v}");
    assert_eq!(v["next"], serde_json::json!([]));
}

#[test]
fn the_kill_switch_halts_heal() {
    let fake = red_pr(LIVE);
    fake.variable("BOSUN_HALT", "1");
    let (code, v) = heal_in(&fake, Some(STEADY), &[]);
    assert_eq!(code, 4, "{v}");
}

#[test]
fn one_failing_pr_keeps_the_calls_of_the_others() {
    let fake = red_pr(LIVE);
    fake.issue(8, "Task eight", &["bosun:state/review"], "");
    fake.pull(31, "PR 31", "feat/8", "Closes #8");
    fake.pull_as_issue(31, "PR 31");
    fake.fail_path("commits/sha31/check-runs", 500);
    let (code, v) = heal_in(&fake, Some(STEADY), &[]);
    assert_eq!(code, 0, "{v}");
    let mut a = actions(&v);
    a.sort();
    assert_eq!(a, ["error", "notify"]);
    assert_eq!(v["next"][0]["tool"], "send_message");
}

#[test]
fn a_pr_closing_a_non_task_is_left_alone() {
    let fake = red_pr(LIVE);
    fake.issue(9, "not a task", &[], "");
    fake.pull(32, "PR 32", "feat/9", "Closes #9");
    let (_, v) = heal_in(&fake, Some(STEADY), &["--pr", "32"]);
    assert_eq!(v["data"]["prs"], serde_json::json!([]), "{v}");
}

#[test]
fn the_brake_caps_spawn_at_notify() {
    // One red head of one pushed: 100 % is over booster's 20 % target, so the brake is on.
    let fake = red_pr(DEAD);
    let (_, v) = heal_in(&fake, Some(BOOSTER_BRAKE), &[]);
    assert_eq!(v["data"]["knob"], "notify", "{v}");
    assert_eq!(v["data"]["brake"]["on"], true);
    assert_eq!(actions(&v), ["skip"]);
    assert_eq!(v["next"], serde_json::json!([]));
    // A live holder is still notified under the brake.
    let fake = red_pr(LIVE);
    let (_, v) = heal_in(&fake, Some(BOOSTER_BRAKE), &[]);
    assert_eq!(actions(&v), ["notify"], "{v}");
}

#[test]
fn after_two_rounds_the_task_goes_to_needs_human() {
    let fake = red_pr(LIVE);
    fake.comment(30, "<!-- bosun:heal head=old1 round=1 action=notify -->");
    fake.comment(30, "<!-- bosun:heal head=old2 round=2 action=notify -->");
    let (code, v) = heal_in(&fake, Some(STEADY), &[]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(actions(&v), ["needs-human"]);
    assert_eq!(v["next"], serde_json::json!([]));
    assert!(
        fake.labels_of(7)
            .iter()
            .any(|l| l == "bosun:state/needs-human")
    );
    let (_, v) = heal_in(&fake, Some(STEADY), &[]);
    assert_eq!(actions(&v), ["skip"], "{v}");
}

#[test]
fn a_forged_marker_does_not_count_and_a_stop_rule_blocks() {
    let fake = red_pr(LIVE);
    fake.permission("mallory", "read");
    fake.comment_by(
        30,
        "mallory",
        "<!-- bosun:heal head=sha30 round=1 action=notify -->",
    );
    let (_, v) = heal_in(&fake, Some(STEADY), &[]);
    assert_eq!(actions(&v), ["notify"], "{v}");

    let fake = red_pr(LIVE);
    for _ in 0..3 {
        fake.comment(
            7,
            r#"{"bosun":"failure","session":"s","signature":"E0308","changed_lines":4}"#,
        );
    }
    let (_, v) = heal_in(&fake, Some(STEADY), &[]);
    assert_eq!(actions(&v), ["skip"], "{v}");
    assert!(
        v["data"]["prs"][0]["why"]
            .as_str()
            .unwrap()
            .contains("same-failure")
    );
}

#[test]
fn green_heads_and_other_prs_are_left_alone() {
    let fake = red_pr(LIVE);
    fake.pull(31, "PR 31", "feat/8", "Closes #8");
    fake.check_run_by("sha31", "ci-ok", "success", ACTIONS);
    let (_, v) = heal_in(&fake, Some(STEADY), &["--pr", "31"]);
    assert_eq!(v["data"]["prs"], serde_json::json!([]), "{v}");
}
