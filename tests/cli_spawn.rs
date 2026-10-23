mod common;

use common::fake::FakeGithub;
use common::{lease_note, run};
use serde_json::Value;

const NOW: &str = "2026-10-22T10:00:00Z";
const LATER: &str = "2026-10-22T10:20:00Z";

fn member(session: &str, parent: Option<&str>, depth: u32, area: Option<&str>) -> String {
    let opt = |k: &str, v: Option<&str>| v.map_or(String::new(), |v| format!(r#","{k}":"{v}""#));
    format!(
        r#"{{"bosun":"join","session":"{session}","origin":"spawned"{}{},"depth":{depth},"expires":"{LATER}"}}"#,
        opt("area", area),
        opt("parent", parent),
    )
}

/// A fake with a registry (#1), the spawner `session_test` registered at depth 1, and a ready
/// epic (#10) in area `lifecycle`.
fn setup() -> FakeGithub {
    let fake = unregistered();
    fake.comment(1, &member("session_test", None, 1, None));
    fake
}

/// Like `setup`, but the spawner never joined.
fn unregistered() -> FakeGithub {
    let fake = FakeGithub::start();
    fake.issue(1, "bosun registry", &["bosun:registry"], "");
    fake.issue(
        10,
        "Lifecycle epic",
        &["bosun:state/ready", "bosun:area/lifecycle"],
        "",
    );
    fake
}

fn spawn(fake: &FakeGithub, extra: &[&str]) -> (i32, Value) {
    let mut args = vec!["spawn", "10"];
    args.extend_from_slice(extra);
    run(fake, "session_test", NOW, &args)
}

fn refused(fake: &FakeGithub, exit: i32, check: u64, name: &str) {
    let before = fake.comments_of(10).len();
    let (code, v) = spawn(fake, &[]);
    assert_eq!(code, exit, "{v}");
    assert_eq!(v["data"]["check"], check);
    assert_eq!(v["data"]["name"], name);
    assert!(v["next"].as_array().unwrap().is_empty());
    assert_eq!(
        fake.comments_of(10).len(),
        before,
        "a refusal claims nothing"
    );
}

#[test]
fn check_1_main_red_exits_4() {
    let fake = setup();
    fake.issue(2, "main is red", &["bosun:main-red"], "");
    refused(&fake, 4, 1, "main-red");
}

#[test]
fn check_2_budget_exits_6() {
    let fake = setup();
    fake.comment(
        1,
        r#"{"bosun":"spend","session":"s","usd":125.0,"at":"2026-10-22T03:00:00Z"}"#,
    );
    // Yesterday's spend does not count.
    fake.comment(
        1,
        r#"{"bosun":"spend","session":"s","usd":900.0,"at":"2026-10-21T23:00:00Z"}"#,
    );
    refused(&fake, 6, 2, "budget");
}

#[test]
fn check_3_depth_exits_6() {
    let fake = unregistered();
    fake.comment(1, &member("session_test", Some("root"), 2, None));
    refused(&fake, 6, 3, "depth");
}

#[test]
fn an_unregistered_spawner_is_refused_with_the_join_fix() {
    let fake = unregistered();
    let (code, v) = spawn(&fake, &[]);
    assert_eq!(code, 6, "{v}");
    assert_eq!(v["data"]["check"], 3);
    assert_eq!(v["data"]["name"], "depth");
    assert!(v["error"]["fix"].as_str().unwrap().contains("bosun join"));
    assert!(fake.comments_of(10).is_empty());
}

#[test]
fn a_negative_spend_record_does_not_lower_todays_spend() {
    let fake = setup();
    for usd in ["200.0", "-150.0"] {
        fake.comment(
            1,
            &format!(
                r#"{{"bosun":"spend","session":"s","usd":{usd},"at":"2026-10-22T03:00:00Z"}}"#
            ),
        );
    }
    refused(&fake, 6, 2, "budget");
}

#[test]
fn limits_come_from_the_default_branch_not_the_working_tree() {
    let fake = setup();
    fake.comment(1, &member("a", None, 1, None));
    fake.content(
        ".bosun/limits.toml",
        "main",
        "[sessions]\nmax_live_sessions = 2\n",
    );
    let dir = std::env::temp_dir().join(format!("bosun-spawn-limits-{}", std::process::id()));
    std::fs::create_dir_all(dir.join(".bosun")).unwrap();
    std::fs::write(
        dir.join(".bosun/limits.toml"),
        "[sessions]\nmax_live_sessions = 99\n",
    )
    .unwrap();
    let out = common::bosun()
        .env("BOSUN_GITHUB_API", fake.url())
        .arg("-C")
        .arg(&dir)
        .args(["spawn", "10"])
        .output()
        .unwrap();
    let v = common::one_object(&out.stdout);
    assert_eq!(out.status.code(), Some(6), "{v}");
    assert_eq!(v["data"]["name"], "live-sessions");
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(
        err.contains(".bosun/limits.toml") && err.contains("default branch"),
        "{err}"
    );
}

#[test]
fn check_4_live_sessions_exits_6() {
    let fake = setup();
    for i in 0..4 {
        fake.comment(1, &member(&format!("s{i}"), None, 1, None));
    }
    refused(&fake, 6, 4, "live-sessions");
}

#[test]
fn check_5_children_exits_6() {
    let fake = setup();
    // The spawner itself is a live session, so lift the live cap to reach check 5.
    fake.content(
        ".bosun/limits.toml",
        "main",
        "[sessions]\nmax_live_sessions = 10\n",
    );
    for i in 0..3 {
        fake.comment(1, &member(&format!("c{i}"), Some("session_test"), 2, None));
    }
    refused(&fake, 6, 5, "children");
}

#[test]
fn check_6_conflict_exits_3_for_a_lease_and_for_an_area_owner() {
    let fake = setup();
    fake.comment(10, &lease_note("other", LATER, "x/**"));
    refused(&fake, 3, 6, "conflict");

    let fake = setup();
    fake.comment(1, &member("owner", None, 1, Some("lifecycle")));
    refused(&fake, 3, 6, "conflict");
}

#[test]
fn mainfix_passes_checks_1_and_2() {
    let fake = setup();
    fake.issue(
        11,
        "Fix main",
        &["bosun:state/ready"],
        r#"<!-- bosun:task {"kind":"mainfix"} -->"#,
    );
    fake.issue(2, "main is red", &["bosun:main-red"], "");
    fake.comment(
        1,
        r#"{"bosun":"spend","session":"s","usd":1000.0,"at":"2026-10-22T03:00:00Z"}"#,
    );
    let (code, v) = run(&fake, "session_test", NOW, &["spawn", "11"]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["checks_passed"], 6);
}

#[test]
fn success_claims_the_epic_and_returns_one_create_session() {
    let fake = setup();
    let (code, v) = spawn(&fake, &[]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["epic"], 10);
    let next = v["next"].as_array().unwrap();
    assert_eq!(next.len(), 1);
    assert_eq!(next[0]["tool"], "create_session");
    let args = &next[0]["args"];
    assert_eq!(
        args["tags"],
        serde_json::json!([
            "bosun:widget",
            "bosun:issue/10",
            "bosun:parent/session_test"
        ])
    );
    assert_eq!(args["title"], "widget: #10 Lifecycle epic");
    assert_eq!(args["model"], "sonnet");
    assert!(!args["model"].as_str().unwrap().contains("[1m]"));
    let prompt = args["prompt"].as_str().unwrap();
    assert!(prompt.contains("#10"));
    assert!(prompt.contains("bosun join --area lifecycle --parent session_test --depth 2"));
    assert!(prompt.contains("bosun wave plan 10"));
    assert!(prompt.contains("bosun result 10 --outcome merged --pr <N>"));

    assert!(
        fake.labels_of(10)
            .contains(&"bosun:state/claimed".to_string())
    );
    let comments = fake.comments_of(10);
    assert_eq!(comments.len(), 1);
    assert!(comments[0].body.contains(r#""session":"session_test""#));

    // Idempotent: a second spawn does not stack a second lease.
    let (code, _) = spawn(&fake, &[]);
    assert_eq!(code, 0);
    assert_eq!(fake.comments_of(10).len(), 1);
}

#[test]
fn model_flag_prompt_file_and_the_1m_ban() {
    let fake = setup();
    let (code, v) = spawn(&fake, &["--model", "opus[1m]"]);
    assert_eq!(code, 2, "{v}");
    assert!(fake.comments_of(10).is_empty());

    let dir = std::env::temp_dir().join(format!("bosun-spawn-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("prompt.txt");
    std::fs::write(&file, "custom prompt").unwrap();
    let (code, v) = spawn(
        &fake,
        &["--model", "opus", "--prompt-file", file.to_str().unwrap()],
    );
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["next"][0]["args"]["model"], "opus");
    assert_eq!(v["next"][0]["args"]["prompt"], "custom prompt");
}

#[test]
fn record_registers_the_child_and_moves_the_lease() {
    let fake = setup();
    let (code, _) = spawn(&fake, &[]);
    assert_eq!(code, 0);
    for result in [
        r#"{"id":"session_child"}"#,
        r#"{"session_id":"session_child"}"#,
        r#"{"session":{"id":"session_child"}}"#,
    ] {
        let fake = setup();
        spawn(&fake, &[]);
        let out = common::bosun()
            .env("BOSUN_GITHUB_API", fake.url())
            .args(["spawn", "10", "--record", "--from", "-"])
            .write_stdin(result)
            .output()
            .unwrap();
        let v = common::one_object(&out.stdout);
        assert_eq!(out.status.code(), Some(0), "{v}");
        assert_eq!(v["data"]["child"], "session_child");
        let join = fake
            .comments_of(1)
            .into_iter()
            .find(|c| c.body.contains(r#""session":"session_child""#))
            .expect("child join");
        for part in [
            r#""bosun":"join""#,
            r#""origin":"spawned""#,
            r#""parent":"session_test""#,
            r#""depth":2"#,
            r#""area":"lifecycle""#,
            r#""expires":"2026-10-22T10:30:00Z""#,
        ] {
            assert!(join.body.contains(part), "{part} in {}", join.body);
        }
        let bodies: Vec<String> = fake.comments_of(10).into_iter().map(|c| c.body).collect();
        assert_eq!(bodies.len(), 3, "{bodies:?}");
        assert!(bodies[1].contains(r#""bosun":"lease-end""#));
        assert!(bodies[1].contains(r#""reason":"spawned""#));
        assert!(bodies[2].contains(r#""bosun":"lease""#));
        assert!(bodies[2].contains(r#""session":"session_child""#));
    }
}

#[test]
fn record_without_an_id_exits_1_with_a_fix() {
    let fake = setup();
    let out = common::bosun()
        .env("BOSUN_GITHUB_API", fake.url())
        .args(["spawn", "10", "--record", "--from", "-"])
        .write_stdin(r#"{"title":"no id"}"#)
        .output()
        .unwrap();
    let v = common::one_object(&out.stdout);
    assert_eq!(out.status.code(), Some(1));
    assert!(!v["error"]["fix"].as_str().unwrap().is_empty());
}

#[test]
fn the_kill_switch_halts_spawn() {
    let fake = setup();
    fake.variable("BOSUN_HALT", "1");
    let (code, _) = spawn(&fake, &[]);
    assert_eq!(code, 4);
}

#[test]
fn losing_the_epic_lease_race_ends_the_lease_and_exits_3() {
    let fake = setup();
    fake.after_requests("/issues/10/comments", 2, |s| {
        s.comments.push(common::fake::FakeComment {
            id: 50,
            issue: 10,
            body: lease_note("session_w", LATER, "x"),
            user: "octocat".into(),
            created_at: "2026-10-22T09:59:59Z".into(),
            updated_at: "2026-10-22T09:59:59Z".into(),
            via_app: None,
            no_app_field: false,
            association: "MEMBER".into(),
        });
    });
    let (code, v) = spawn(&fake, &[]);
    assert_eq!(code, 3, "{v}");
    assert!(
        v["error"]["message"]
            .as_str()
            .unwrap()
            .contains("session_w")
    );
    assert!(v["next"].as_array().unwrap().is_empty());
    let ends: Vec<_> = fake
        .comments_of(10)
        .into_iter()
        .filter(|c| c.body.contains(r#""bosun":"lease-end""#) && c.body.contains("lost-race"))
        .collect();
    assert_eq!(ends.len(), 1);
    assert!(fake.labels_of(10).contains(&"bosun:state/ready".into()));
}
