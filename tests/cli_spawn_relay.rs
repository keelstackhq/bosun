mod common;

use common::fake::FakeGithub;
use common::run;
use serde_json::Value;

const NOW: &str = "2026-11-03T11:00:00Z";
const LIVE: &str = "2026-11-03T11:20:00Z";

fn join_note(session: &str, parent: Option<&str>) -> String {
    let parent = parent.map_or(String::new(), |p| format!(",\"parent\":\"{p}\""));
    format!(
        "{{\"bosun\":\"join\",\"session\":\"{session}\",\"origin\":\"local\"{parent},\"depth\":1,\"expires\":\"{LIVE}\"}}"
    )
}

fn handoff_note(session: &str, line: &str) -> String {
    format!(
        "{{\"bosun\":\"handoff\",\"session\":\"{session}\",\"at\":\"{NOW}\",\"lines\":[\"{line}\"]}}"
    )
}

fn setup() -> FakeGithub {
    let fake = FakeGithub::start();
    fake.issue(1, "bosun registry", &["bosun:registry"], "");
    fake.comment(1, &join_note("root_a", None));
    fake
}

fn bodies(fake: &FakeGithub, issue: u64) -> Vec<String> {
    fake.comments_of(issue)
        .into_iter()
        .map(|c| c.body)
        .collect()
}

fn file(name: &str, text: &str) -> String {
    let path = std::env::temp_dir().join(format!("bosun-relay-{}-{name}", std::process::id()));
    std::fs::write(&path, text).unwrap();
    path.to_string_lossy().into_owned()
}

#[test]
fn uc_zh_4_a_root_with_a_handoff_gets_the_relay_calls() {
    let fake = setup();
    fake.comment(1, &handoff_note("root_a", "next: go on"));
    fake.content(
        ".bosun/bosun.toml",
        "main",
        "project = \"widget\"\n[cloud]\nenvironment_id = \"env_abc\"\n",
    );
    let before = bodies(&fake, 1);
    let (code, v) = run(&fake, "root_a", NOW, &["spawn", "--root", "--handoff"]);
    assert_eq!(code, 0, "{v}");
    let next = v["next"].as_array().unwrap();
    assert_eq!(
        next.len(),
        1,
        "archive_session comes only from --record: {v}"
    );
    assert_eq!(next[0]["tool"], "create_session");
    let a = &next[0]["args"];
    assert_eq!(a["prompt"], "bosun resume --handoff");
    assert_eq!(a["environment_id"], "env_abc");
    assert_eq!(a["tags"][1], "bosun:relay/root_a");
    assert_eq!(a["source_url"], "https://github.com/acme/widget");
    assert_eq!(a["source_revision"], "main");
    assert_eq!(v["data"]["from"], "root_a");
    assert_eq!(v["data"]["relay_waves"], 6);
    let then = v["data"]["then"].as_str().unwrap();
    assert!(then.contains("--record") && then.contains("archive_session"));
    assert_eq!(bodies(&fake, 1), before, "no comment");
    assert_eq!(fake.state.lock().unwrap().issues.len(), 1);
}

#[test]
fn without_a_cloud_table_there_is_no_environment_id() {
    let fake = setup();
    fake.comment(1, &handoff_note("root_a", "next: go on"));
    let (code, v) = run(&fake, "root_a", NOW, &["spawn", "--root", "--handoff"]);
    assert_eq!(code, 0, "{v}");
    let a = v["next"][0]["args"].as_object().unwrap();
    assert!(!a.contains_key("environment_id"), "{a:?}");
    assert!(v["data"]["environment_id"].is_null());
}

#[test]
fn refusals_and_usage_errors() {
    let fake = setup();
    // No handoff yet.
    let (code, v) = run(&fake, "root_a", NOW, &["spawn", "--root", "--handoff"]);
    assert_eq!(code, 1, "{v}");
    assert!(
        v["error"]["fix"]
            .as_str()
            .unwrap()
            .contains("bosun handoff")
    );
    // A handoff of another root does not count.
    fake.comment(1, &join_note("root_b", None));
    fake.comment(1, &handoff_note("root_b", "next: x"));
    let (code, _) = run(&fake, "root_a", NOW, &["spawn", "--root", "--handoff"]);
    assert_eq!(code, 1);
    // A child and a wave agent.
    fake.comment(1, &join_note("child_c", Some("root_a")));
    fake.comment(1, &join_note("root_a/T7", None));
    for s in ["child_c", "root_a/T7"] {
        fake.comment(1, &handoff_note(s, "next: y"));
        let (code, v) = run(&fake, s, NOW, &["spawn", "--root", "--handoff"]);
        assert_eq!(code, 1, "{s}: {v}");
    }
    // Usage errors.
    for args in [
        &["spawn", "--handoff"][..],
        &["spawn", "--root", "5", "--handoff"],
        &["spawn", "--root"],
        &["spawn", "--root", "--handoff", "--slack-thread", "1.2"],
    ] {
        let (code, v) = run(&fake, "root_a", NOW, args);
        assert_eq!(code, 2, "{args:?}: {v}");
    }
}

#[test]
fn record_joins_the_new_root_and_wires_the_lineage() {
    let fake = setup();
    fake.comment(1, &handoff_note("root_a", "next: old root"));
    fake.issue(10, "t", &["bosun:state/building"], "");
    fake.comment(10, &common::lease_note("root_a", LIVE, "src/**"));
    // A wave agent of the old root dies with it; a lookalike session keeps its lease.
    fake.issue(11, "w", &["bosun:state/building"], "");
    fake.comment(11, &common::lease_note("root_a/T7", LIVE, "docs/**"));
    fake.issue(12, "x", &["bosun:state/building"], "");
    fake.comment(12, &common::lease_note("root_ab", LIVE, "x/**"));
    fake.comment(1, &join_note("root_z", None));
    fake.comment(1, &handoff_note("root_z", "next: newer, other lineage"));
    let f = file("rec", r#"{"id":"session_new"}"#);
    let (code, v) = run(
        &fake,
        "root_a",
        NOW,
        &["spawn", "--root", "--record", "--from", &f],
    );
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["root"], "session_new");
    assert_eq!(v["next"][0]["tool"], "archive_session");
    assert_eq!(v["next"][0]["args"]["session_id"], "root_a");
    let all = bodies(&fake, 1);
    let join = all
        .iter()
        .find(|b| b.contains(r#""session":"session_new""#) && b.contains(r#""bosun":"join""#))
        .expect("join");
    let j: Value =
        serde_json::from_str(join.lines().rev().find(|l| l.starts_with('{')).unwrap()).unwrap();
    assert_eq!(j["origin"], "relay");
    assert_eq!(j["depth"], 1);
    assert!(j.get("parent").is_none_or(Value::is_null));
    assert!(all.iter().any(|b| b.contains(
        r#"{"bosun":"relay","session":"root_a","root":"session_new","at":"2026-11-03T11:00:00Z"}"#
    )));
    assert!(!all.iter().any(|b| b.contains(r#""bosun":"lease""#)));

    // The old root left and its lease ended; the lineage still works.
    assert!(
        all.iter()
            .any(|b| b.contains(r#""bosun":"leave","session":"root_a","reason":"relayed""#))
    );
    assert!(
        bodies(&fake, 10)
            .iter()
            .any(|b| b.contains(r#""bosun":"lease-end""#) && b.contains("root_a"))
    );
    assert!(
        bodies(&fake, 11)
            .iter()
            .any(|b| b.contains(r#""bosun":"lease-end""#) && b.contains("root_a/T7"))
    );
    assert!(!bodies(&fake, 12).iter().any(|b| b.contains("lease-end")));
    let (code, v) = run(&fake, "session_new", NOW, &["resume", "--handoff"]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["handoff"]["session"], "root_a");
    assert_eq!(v["data"]["handoff"]["lines"][0], "next: old root");
    // Once the new root hands off itself, its own newer handoff wins.
    let (code, v) = run(&fake, "session_new", NOW, &["handoff"]);
    assert_eq!(code, 0, "{v}");
    let (_, v) = run(&fake, "session_new", NOW, &["resume", "--handoff"]);
    assert_eq!(v["data"]["handoff"]["session"], "session_new");
}

#[test]
fn record_refuses_to_promote_an_existing_session() {
    let fake = setup();
    fake.comment(1, &handoff_note("root_a", "next: x"));
    fake.comment(1, &join_note("child_c", Some("root_a")));
    fake.comment(1, &join_note("root_z", None));
    fake.comment(1, &join_note("gone", Some("root_a")));
    fake.comment(1, r#"{"bosun":"leave","session":"gone","reason":"done"}"#);
    let before = bodies(&fake, 1);
    for s in ["child_c", "root_z", "gone"] {
        let f = file(s, &format!(r#"{{"id":"{s}"}}"#));
        let (code, v) = run(
            &fake,
            "root_a",
            NOW,
            &["spawn", "--root", "--record", "--from", &f],
        );
        assert_eq!(code, 1, "{s}: {v}");
        assert!(
            v["error"]["fix"]
                .as_str()
                .unwrap()
                .contains("create_session")
        );
    }
    assert_eq!(bodies(&fake, 1), before);
    let (code, _) = run(&fake, "child_c", NOW, &["handoff"]);
    assert_eq!(code, 1, "child_c is still no root");
}

#[test]
fn record_needs_the_callers_handoff_and_relays_once() {
    let fake = setup();
    let f = file("once", r#"{"id":"n1"}"#);
    let args = ["spawn", "--root", "--record", "--from", &f];
    let (code, v) = run(&fake, "root_a", NOW, &args);
    assert_eq!(code, 1, "{v}");
    assert!(
        v["error"]["fix"]
            .as_str()
            .unwrap()
            .contains("bosun handoff")
    );
    assert_eq!(bodies(&fake, 1).len(), 1);
    fake.comment(1, &handoff_note("root_a", "next: x"));
    assert_eq!(run(&fake, "root_a", NOW, &args).0, 0);
    let f2 = file("twice", r#"{"id":"n2"}"#);
    let (code, v) = run(
        &fake,
        "root_a",
        NOW,
        &["spawn", "--root", "--record", "--from", &f2],
    );
    assert_eq!(code, 1, "{v}");
    assert!(
        !bodies(&fake, 1)
            .iter()
            .any(|b| b.contains(r#""session":"n2""#))
    );
}

#[test]
fn record_refuses_self_slash_non_root_and_missing_id() {
    let fake = setup();
    fake.comment(1, &handoff_note("root_a", "next: x"));
    let go = |s: &str, text: &str| {
        let f = file("bad", text);
        run(
            &fake,
            s,
            NOW,
            &["spawn", "--root", "--record", "--from", &f],
        )
    };
    assert_eq!(go("root_a", r#"{"id":"root_a"}"#).0, 1);
    assert_eq!(go("root_a", r#"{"id":"x/y"}"#).0, 1);
    assert_eq!(go("root_a", r#"{}"#).0, 1);
    fake.comment(1, &join_note("child_c", Some("root_a")));
    assert_eq!(go("child_c", r#"{"id":"s2"}"#).0, 1);
    assert_eq!(bodies(&fake, 1).len(), 3);
}

#[test]
fn uc_zh_5_relay_waves_defaults_to_6_and_comes_from_the_default_branch() {
    let fake = setup();
    let (_, v) = run(&fake, "root_a", NOW, &["resume", "--handoff"]);
    assert_eq!(v["data"]["relay_waves"], 6);
    let (_, v) = run(&fake, "root_a", NOW, &["resume"]);
    assert!(!v["data"].as_object().unwrap().contains_key("relay_waves"));
    fake.content(".bosun/limits.toml", "main", "[waves]\nrelay_waves = 4\n");
    let (_, v) = run(&fake, "root_a", NOW, &["resume", "--handoff"]);
    assert_eq!(v["data"]["relay_waves"], 4);
}
