mod common;

use common::fake::FakeGithub;
use common::{bosun, one_object};
use serde_json::{Value, json};

const T0: &str = "2026-10-22T10:00:00Z";

fn budget(fake: &FakeGithub, stdin: &str, args: &[&str]) -> (i32, Value) {
    let out = bosun()
        .env("BOSUN_GITHUB_API", fake.url())
        .env("BOSUN_NOW", T0)
        .args(["budget", "record"])
        .args(args)
        .write_stdin(stdin.to_string())
        .output()
        .unwrap();
    let v = one_object(&out.stdout);
    if v["ok"] == false {
        assert!(!v["error"]["fix"].as_str().unwrap().trim().is_empty());
    }
    (out.status.code().unwrap(), v)
}

fn spends(fake: &FakeGithub, issue: u64) -> usize {
    fake.comments_of(issue)
        .iter()
        .filter(|c| c.body.contains(r#""bosun":"spend""#))
        .count()
}

#[test]
fn claude_sessions_records_on_task_and_registry() {
    let fake = FakeGithub::start();
    fake.issue(5, "task", &["bosun:state/building"], "");
    let (code, v) = budget(
        &fake,
        r#"{"session":{"usage":{"cost_usd":3.25}}}"#,
        &["5", "--provider", "claude-sessions", "--from", "-"],
    );
    assert_eq!(code, 0, "{v}");
    assert_eq!(
        v["data"],
        json!({"issue": 5, "usd": 3.25, "provider": "claude-sessions",
               "task_total_usd": 3.25, "over_task_cap": false, "blocked": false})
    );
    assert_eq!(spends(&fake, 5), 1);
    let reg = fake
        .state
        .lock()
        .unwrap()
        .issues
        .iter()
        .find(|i| i.labels.contains(&"bosun:registry".to_string()))
        .map(|i| i.number)
        .expect("registry created");
    assert_eq!(spends(&fake, reg), 1);
    assert!(fake.comments_of(5)[0].body.contains(r#""usd":3.25"#));
}

#[test]
fn missing_cost_is_exit_1_with_the_generic_fix() {
    let fake = FakeGithub::start();
    fake.issue(5, "task", &[], "");
    let (code, v) = budget(
        &fake,
        r#"{"usage":{}}"#,
        &["5", "--provider", "claude-sessions", "--from", "-"],
    );
    assert_eq!(code, 1, "{v}");
    assert!(
        v["error"]["fix"]
            .as_str()
            .unwrap()
            .contains("--provider generic")
    );
    assert_eq!(spends(&fake, 5), 0);
}

#[test]
fn generic_estimates_from_turns_and_model() {
    let fake = FakeGithub::start();
    fake.issue(5, "task", &[], "");
    let args = [
        "5",
        "--provider",
        "generic",
        "--turns",
        "10",
        "--model",
        "sonnet",
    ];
    let (code, v) = budget(&fake, "", &args);
    assert_eq!(code, 0, "{v}");
    assert!((v["data"]["usd"].as_f64().unwrap() - 0.55).abs() < 1e-9);
}

#[test]
fn over_the_task_cap_blocks_the_task() {
    let fake = FakeGithub::start();
    fake.issue(5, "task", &["bosun:state/building"], "");
    let (code, v) = budget(
        &fake,
        r#"{"usage":{"cost_usd":20.0}}"#,
        &["5", "--provider", "claude-sessions", "--from", "-"],
    );
    assert_eq!((code, &v["data"]["over_task_cap"]), (0, &json!(false)));
    let (code, v) = budget(
        &fake,
        r#"{"usage":{"cost_usd":6.0}}"#,
        &["5", "--provider", "claude-sessions", "--from", "-"],
    );
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["task_total_usd"], 26.0);
    assert_eq!(v["data"]["over_task_cap"], true);
    assert_eq!(v["data"]["blocked"], true);
    assert!(fake.labels_of(5).contains(&"bosun:state/blocked".into()));
}

#[test]
fn kill_switch_applies() {
    let fake = FakeGithub::start();
    fake.issue(5, "task", &[], "");
    fake.variable("BOSUN_HALT", "1");
    let args = [
        "5",
        "--provider",
        "generic",
        "--turns",
        "1",
        "--model",
        "haiku",
    ];
    let (code, _) = budget(&fake, "", &args);
    assert_eq!(code, 4);
}

#[test]
fn a_negative_cost_is_refused_and_records_nothing() {
    let fake = FakeGithub::start();
    fake.issue(5, "task", &["bosun:state/building"], "");
    let (code, v) = budget(
        &fake,
        r#"{"usage":{"cost_usd":-5.0}}"#,
        &["5", "--provider", "claude-sessions", "--from", "-"],
    );
    assert_eq!(code, 1, "{v}");
    assert_eq!(spends(&fake, 5), 0);
}

#[test]
fn the_task_cap_comes_from_the_default_branch_and_ignores_negative_records() {
    let fake = FakeGithub::start();
    fake.issue(5, "task", &["bosun:state/building"], "");
    fake.content(
        ".bosun/limits.toml",
        "main",
        "[budget]\nper_task_usd = 10.0\n",
    );
    fake.comment(
        5,
        r#"{"bosun":"spend","session":"s","usd":-100.0,"at":"2026-10-22T03:00:00Z"}"#,
    );
    let (code, v) = budget(
        &fake,
        r#"{"usage":{"cost_usd":12.0}}"#,
        &["5", "--provider", "claude-sessions", "--from", "-"],
    );
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["task_total_usd"], 12.0);
    assert_eq!(v["data"]["over_task_cap"], true);
}
