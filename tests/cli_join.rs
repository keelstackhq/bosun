mod common;

use common::fake::FakeGithub;
use common::{lease_note as lease, run};

const T0: &str = "2026-10-22T10:00:00Z";
const T1: &str = "2026-10-22T10:10:00Z";

fn registry(fake: &FakeGithub) -> u64 {
    let s = fake.state.lock().unwrap();
    let regs: Vec<_> = s
        .issues
        .iter()
        .filter(|i| i.labels.iter().any(|l| l == "bosun:registry"))
        .collect();
    assert_eq!(regs.len(), 1, "exactly one registry issue");
    assert_eq!(regs[0].title, "bosun registry");
    regs[0].number
}

#[test]
fn join_creates_the_registry_once_and_edits_in_place() {
    let fake = FakeGithub::start();
    let (code, v) = run(&fake, "session_a", T0, &["join", "--area", "lifecycle"]);
    assert_eq!(code, 0, "{v}");
    let reg = registry(&fake);
    assert_eq!(v["data"]["registry_issue"], reg);
    assert_eq!(v["data"]["session"], "session_a");
    assert_eq!(v["data"]["origin"], "local");
    assert_eq!(v["data"]["area"], "lifecycle");
    assert_eq!(v["data"]["depth"], 1);
    assert_eq!(v["data"]["expires"], "2026-10-22T10:30:00Z");

    let (code, v) = run(&fake, "session_a", T1, &["join"]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["expires"], "2026-10-22T10:40:00Z");
    assert_eq!(v["data"]["area"], "lifecycle", "kept on rejoin");
    registry(&fake);
    let comments = fake.comments_of(reg);
    assert_eq!(
        comments.len(),
        1,
        "a live member never gets a second comment"
    );
    assert!(comments[0].body.contains("2026-10-22T10:40:00Z"));
    assert!(fake.requests().iter().any(|r| {
        r.method == "PATCH"
            && r.path
                .ends_with(&format!("/issues/comments/{}", comments[0].id))
    }));

    let (_, v) = run(
        &fake,
        "session_b",
        T1,
        &[
            "join",
            "--origin",
            "spawned",
            "--parent",
            "session_a",
            "--depth",
            "2",
        ],
    );
    assert_eq!(v["data"]["origin"], "spawned");
    assert_eq!(fake.comments_of(reg).len(), 2);
    assert!(
        fake.comments_of(reg)[1]
            .body
            .contains(r#""parent":"session_a""#)
    );
}

#[test]
fn join_and_heartbeat_are_halted_by_the_kill_switch() {
    let fake = FakeGithub::start();
    fake.variable("BOSUN_HALT", "1");
    for cmd in ["join", "heartbeat"] {
        let (code, v) = run(&fake, "session_a", T0, &[cmd]);
        assert_eq!(code, 4, "{cmd}");
        assert_eq!(v["error"]["kind"], "halted");
    }
}

#[test]
fn bad_origin_is_an_error_with_a_fix() {
    let fake = FakeGithub::start();
    let (code, _) = run(&fake, "session_a", T0, &["join", "--origin", "mars"]);
    assert_eq!(code, 1);
}

#[test]
fn heartbeat_edits_leases_and_the_join_record_in_place() {
    let fake = FakeGithub::start();
    run(&fake, "session_a", T0, &["join"]);
    let reg = registry(&fake);
    fake.issue(5, "mine", &["bosun:state/claimed"], "");
    fake.issue(6, "expired", &["bosun:state/building"], "");
    fake.issue(7, "theirs", &["bosun:state/claimed"], "");
    let mine = fake.comment(5, &lease("session_a", "2026-10-22T10:20:00Z", "src/a.rs"));
    fake.comment(6, &lease("session_a", "2026-10-22T09:00:00Z", "src/b.rs"));
    let theirs = fake.comment(7, &lease("session_z", "2026-10-22T10:20:00Z", "src/c.rs"));
    let before = fake.state.lock().unwrap().comments.len();
    let posts = fake
        .requests()
        .iter()
        .filter(|r| r.method == "POST")
        .count();

    let (code, v) = run(&fake, "session_a", T1, &["heartbeat"]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["leases"], serde_json::json!([5]));
    assert_eq!(v["data"]["registry"], true);
    let s = fake.state.lock().unwrap();
    assert_eq!(s.comments.len(), before, "no new comments");
    let body = |id: u64| s.comments.iter().find(|c| c.id == id).unwrap().body.clone();
    assert!(body(mine).contains("2026-10-22T10:40:00Z"));
    assert!(body(mine).starts_with("Claimed."), "human text kept");
    assert!(body(theirs).contains("2026-10-22T10:20:00Z"));
    assert!(body(s.comments.iter().find(|c| c.issue == reg).unwrap().id).contains("10:40:00Z"));
    drop(s);
    let posts_after = fake
        .requests()
        .iter()
        .filter(|r| r.method == "POST")
        .count();
    assert_eq!(posts, posts_after, "heartbeat only PATCHes");
}

#[test]
fn heartbeat_without_a_registry_reports_false() {
    let fake = FakeGithub::start();
    let (code, v) = run(&fake, "session_a", T0, &["heartbeat"]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["registry"], false);
    assert_eq!(v["data"]["leases"], serde_json::json!([]));
}

#[test]
fn join_never_lowers_the_depth_or_changes_the_parent() {
    let fake = FakeGithub::start();
    run(
        &fake,
        "session_c",
        T0,
        &[
            "join",
            "--origin",
            "spawned",
            "--parent",
            "session_a",
            "--depth",
            "2",
        ],
    );
    let (code, v) = run(
        &fake,
        "session_c",
        T1,
        &["join", "--parent", "session_x", "--depth", "1"],
    );
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["depth"], 2);
    let reg = registry(&fake);
    let body = &fake.comments_of(reg)[0].body;
    assert!(
        body.contains(r#""parent":"session_a""#) && body.contains(r#""depth":2"#),
        "{body}"
    );
    let (_, v) = run(&fake, "session_c", T1, &["join", "--depth", "3"]);
    assert_eq!(v["data"]["depth"], 3, "a join may raise the depth");
}

// ---- Claude Tag seats (spec 10-slack) ----

fn seat_run(
    fake: &FakeGithub,
    dir: &std::path::Path,
    envs: &[(&str, &str)],
    args: &[&str],
) -> (i32, serde_json::Value) {
    let mut cmd = common::bosun();
    cmd.current_dir(dir)
        .env("BOSUN_GITHUB_API", fake.url())
        .env("BOSUN_SESSION", "session_tag")
        .env("BOSUN_NOW", T0);
    for (k, v) in envs {
        cmd.env(k, v);
    }
    let out = cmd.arg("join").args(args).output().unwrap();
    (out.status.code().unwrap(), common::one_object(&out.stdout))
}

fn slack_dir(config: &str) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join(".bosun")).unwrap();
    std::fs::write(dir.path().join(".bosun/bosun.toml"), config).unwrap();
    dir
}

const SLACK_CFG: &str = "[slack]\nchannel = \"C0123ABCD\"\n";

fn registry_bodies(fake: &FakeGithub) -> Vec<String> {
    let s = fake.state.lock().unwrap();
    let n = s
        .issues
        .iter()
        .find(|i| i.labels.iter().any(|l| l == "bosun:registry"))
        .map(|i| i.number);
    drop(s);
    n.map(|n| fake.comments_of(n).into_iter().map(|c| c.body).collect())
        .unwrap_or_default()
}

#[test]
fn join_writes_channel_and_thread_seats_and_keeps_them() {
    let fake = FakeGithub::start();
    let d = slack_dir(SLACK_CFG);
    let (code, v) = seat_run(
        &fake,
        d.path(),
        &[],
        &["--origin", "slack", "--slack-channel", "C0123ABCD"],
    );
    assert_eq!(code, 0, "{v}");
    assert_eq!(
        v["data"]["slack"],
        serde_json::json!({"role": "channel", "channel": "C0123ABCD"})
    );
    let bodies = registry_bodies(&fake);
    assert!(bodies[0].contains(r#""slack":{"role":"channel","channel":"C0123ABCD"}"#));

    let (code, v) = seat_run(
        &fake,
        d.path(),
        &[],
        &[
            "--origin",
            "slack",
            "--slack-channel",
            "C0123ABCD",
            "--slack-thread",
            "1700000000.000200",
        ],
    );
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["slack"]["role"], "thread");
    assert_eq!(v["data"]["slack"]["thread_ts"], "1700000000.000200");

    let (code, v) = seat_run(&fake, d.path(), &[], &["--origin", "slack"]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["slack"]["thread_ts"], "1700000000.000200", "kept");
    assert_eq!(registry_bodies(&fake).len(), 1);

    let (_, v) = run(&fake, "session_plain", T0, &["join"]);
    assert!(v["data"]["slack"].is_null());
}

#[test]
fn join_seat_usage_errors_exit_2() {
    let fake = FakeGithub::start();
    let d = slack_dir(SLACK_CFG);
    for args in [
        vec!["--origin", "slack", "--slack-thread", "1700000000.000200"],
        vec!["--slack-channel", "C0123ABCD"],
        vec!["--origin", "slack", "--slack-channel", "c0123"],
        vec![
            "--origin",
            "slack",
            "--slack-channel",
            "C0123ABCD",
            "--slack-thread",
            "17000",
        ],
    ] {
        let (code, v) = seat_run(&fake, d.path(), &[], &args);
        assert_eq!(code, 2, "{args:?}: {v}");
    }
    assert!(registry_bodies(&fake).is_empty());
}

#[test]
fn join_seat_needs_matching_slack_config_and_non_child_origin() {
    let fake = FakeGithub::start();
    let ok = ["--origin", "slack", "--slack-channel", "C0123ABCD"];
    let other = slack_dir("[slack]\nchannel = \"C9999ZZZ\"\n");
    let none = slack_dir("");
    let empty = slack_dir("[slack]\nchannel = \"\"\n");
    for d in [&other, &none, &empty] {
        let (code, v) = seat_run(&fake, d.path(), &[], &ok);
        assert_eq!(code, 5, "{v}");
    }
    let good = slack_dir(SLACK_CFG);
    let (code, v) = seat_run(
        &fake,
        good.path(),
        &[("CLAUDE_CODE_CHILD_SESSION", "1")],
        &ok,
    );
    assert_eq!(code, 5, "spawned: {v}");
    let (code, v) = seat_run(&fake, good.path(), &[("BOSUN_ORIGIN", "routine")], &ok);
    assert_eq!(code, 5, "routine: {v}");
    assert!(registry_bodies(&fake).is_empty(), "nothing written");
}

fn seat_args() -> [&'static str; 4] {
    ["--origin", "slack", "--slack-channel", "C0123ABCD"]
}

#[test]
fn a_plain_rejoin_keeps_origin_slack_and_the_seat_and_another_origin_drops_it() {
    let fake = FakeGithub::start();
    let d = slack_dir(SLACK_CFG);
    let (code, v) = seat_run(&fake, d.path(), &[], &seat_args());
    assert_eq!(code, 0, "{v}");
    let (code, v) = seat_run(&fake, d.path(), &[], &[]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["origin"], "slack");
    assert_eq!(v["data"]["slack"]["channel"], "C0123ABCD");
    let (code, v) = seat_run(&fake, d.path(), &[], &["--origin", "local"]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["origin"], "local");
    assert!(v["data"]["slack"].is_null());
    assert!(!registry_bodies(&fake)[0].contains("\"slack\""));
}

#[test]
fn heartbeat_and_resume_keep_the_seat_and_origin() {
    let fake = FakeGithub::start();
    let d = slack_dir(SLACK_CFG);
    seat_run(&fake, d.path(), &[], &seat_args());
    for cmd in ["heartbeat", "resume"] {
        let mut c = common::bosun();
        let out = c
            .current_dir(d.path())
            .env("BOSUN_GITHUB_API", fake.url())
            .env("BOSUN_SESSION", "session_tag")
            .env("BOSUN_NOW", T1)
            .arg(cmd)
            .output()
            .unwrap();
        assert_eq!(out.status.code(), Some(0), "{cmd}");
        let b = &registry_bodies(&fake)[0];
        assert!(b.contains(r#""origin":"slack""#), "{cmd}: {b}");
        assert!(b.contains(r#""channel":"C0123ABCD""#), "{cmd}: {b}");
        assert!(b.contains("2026-10-22T10:40:00Z"), "{cmd}: {b}");
    }
}

#[test]
fn a_spawned_or_routine_record_cannot_join_as_slack() {
    for origin in ["spawned", "routine"] {
        let fake = FakeGithub::start();
        fake.issue(1, "bosun registry", &["bosun:registry"], "");
        fake.comment(
            1,
            &format!(
                r#"{{"bosun":"join","session":"session_tag","origin":"{origin}","depth":2,"parent":"p","expires":"2026-10-22T10:20:00Z"}}"#
            ),
        );
        let d = slack_dir(SLACK_CFG);
        let (code, v) = seat_run(&fake, d.path(), &[], &seat_args());
        assert_eq!(code, 5, "{origin}: {v}");
        let b = registry_bodies(&fake);
        assert_eq!(b.len(), 1);
        assert!(
            !b[0].contains("\"slack\"") && b[0].contains(origin),
            "{b:?}"
        );
    }
}

#[test]
fn a_plain_rejoin_keeps_origin_slack_without_a_seat() {
    let fake = FakeGithub::start();
    let d = slack_dir(SLACK_CFG);
    let (code, v) = seat_run(&fake, d.path(), &[], &["--origin", "slack"]);
    assert_eq!(code, 0, "{v}");
    let (code, v) = seat_run(&fake, d.path(), &[], &[]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["origin"], "slack");
}
