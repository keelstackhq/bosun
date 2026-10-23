//! Use cases of M6 (spec §12): Slack.
mod common;

use common::fake::FakeGithub;
use common::{bosun, one_object};
use serde_json::json;

/// UC-M6-1: notify refuses a shared channel or a channel with guests.
#[test]
fn uc_m6_1() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join(".bosun")).unwrap();
    std::fs::write(
        dir.path().join(".bosun/bosun.toml"),
        "[slack]\nchannel = \"C1\"\n",
    )
    .unwrap();
    let gh = FakeGithub::start();
    gh.issue(3, "main red", &["bosun:main-red"], "");
    for (info, reason) in [
        (
            json!({"id":"C1","is_ext_shared":true,"is_org_shared":false,"is_shared":true,"guests":0}),
            "channel shared",
        ),
        (
            json!({"id":"C1","is_ext_shared":false,"is_org_shared":false,"is_shared":false,"guests":1}),
            "channel has guests",
        ),
    ] {
        let out = bosun()
            .current_dir(dir.path())
            .env("BOSUN_GITHUB_API", gh.url())
            .args(["notify", "--channel-info", "-"])
            .write_stdin(info.to_string())
            .output()
            .unwrap();
        let v = one_object(&out.stdout);
        assert_eq!(out.status.code(), Some(5), "{v}");
        assert_eq!(v["data"]["reason"], reason);
        assert_eq!(v["next"], json!([]), "nothing to post");
    }

    // The contrast: a private channel without guests gets its post.
    let good =
        json!({"id":"C1","is_ext_shared":false,"is_org_shared":false,"is_shared":false,"guests":0});
    let out = bosun()
        .current_dir(dir.path())
        .env("BOSUN_GITHUB_API", gh.url())
        .args(["notify", "--channel-info", "-"])
        .write_stdin(good.to_string())
        .output()
        .unwrap();
    let v = one_object(&out.stdout);
    assert_eq!(out.status.code(), Some(0), "{v}");
    assert_eq!(v["next"][0]["tool"], "slack_post");
    assert_eq!(v["next"][0]["args"]["channel"], "C1");
}

/// UC-M6-2: an answer from a non-owner Slack ID does not count.
#[test]
fn uc_m6_2() {
    let fake = FakeGithub::start();
    fake.issue(4, "task", &["bosun:state/needs-human"], "body");
    fake.comment(4, r#"{"bosun":"ask","session":"s","question":"Why?"}"#);
    fake.content(
        "governance/owners.toml",
        "main",
        "[[owner]]\ngithub = \"ada\"\nslack = \"U0ADA1\"\n",
    );
    let out = bosun()
        .env("BOSUN_GITHUB_API", fake.url())
        .args(["answer", "4", "--slack-user", "U0OTHER"])
        .write_stdin("Yes.")
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(5));
    assert_eq!(
        one_object(&out.stdout)["data"]["reason"],
        "not an owner Slack ID"
    );
    assert_eq!(fake.labels_of(4), ["bosun:state/needs-human"]);
    assert_eq!(fake.comments_of(4).len(), 1);
}

/// UC-M6-3: a thread request with no ready issue runs intake, then a plan task, then spawn.
#[test]
fn uc_m6_3() {
    let fake = FakeGithub::start();
    fake.issue(1, "bosun registry", &["bosun:registry"], "");
    fake.comment(
        1,
        r#"{"bosun":"join","session":"session_test","origin":"slack","depth":1,"expires":"2026-10-22T10:20:00Z"}"#,
    );
    fake.content(
        "governance/owners.toml",
        "main",
        "[[owner]]\ngithub = \"ada\"\nslack = \"U0ADA1\"\n",
    );
    fake.content(".bosun/bosun.toml", "main", "[slack]\nchannel = \"C1\"\n");
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("plans")).unwrap();
    std::fs::write(
        dir.path().join("plans/widget.md"),
        "# Plan: widget\n\nproject: widget\n\n## M1: Lifecycle\n\narea: lifecycle\n\n### Tasks\n\n- T1: State machine\n  touch: src/core/state.rs\n\n### Done when\n\n- UC-M1-1: a task moves planned -> merged.\n",
    )
    .unwrap();
    let out = bosun()
        .env("BOSUN_GITHUB_API", fake.url())
        .env("BOSUN_ORIGIN", "slack")
        .arg("-C")
        .arg(dir.path())
        .args(["spawn", "--slack-thread", "1700000000.000200"])
        .args(["--slack-user", "U0ADA1", "--request", "-"])
        .write_stdin("add a dry-run flag")
        .output()
        .unwrap();
    let v = one_object(&out.stdout);
    assert_eq!(out.status.code(), Some(0), "{v}");
    let n = v["data"]["intake"]
        .as_u64()
        .expect("intake opened an issue");
    let labels = fake.labels_of(n);
    assert!(
        labels.contains(&"bosun:kind/plan".to_string()),
        "{labels:?}"
    );
    assert!(
        labels.contains(&"bosun:source/slack".to_string()),
        "{labels:?}"
    );
    assert!(
        labels.contains(&"bosun:state/claimed".to_string()),
        "{labels:?}"
    );
    assert_eq!(v["data"]["epic"], n, "spawn took the plan task");
    assert_eq!(v["next"][0]["tool"], "create_session");
}

/// UC-M6-4: a child result produces exactly one thread reply, also when the thread ended.
#[test]
fn uc_m6_4() {
    const LIVE: &str = "1700000000.000200";
    const ENDED: &str = "1700000001.000100";
    let good =
        r#"{"id":"C1","is_ext_shared":false,"is_org_shared":false,"is_shared":false,"guests":0}"#;
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join(".bosun")).unwrap();
    std::fs::write(
        dir.path().join(".bosun/bosun.toml"),
        "[slack]\nchannel = \"C1\"\n",
    )
    .unwrap();
    let gh = FakeGithub::start();
    gh.issue(1, "bosun registry", &["bosun:registry"], "");
    // Epic 10: the thread is live. Epic 11: the thread ended without acking.
    for (n, ts) in [(10, LIVE), (11, ENDED)] {
        gh.issue(n, "epic", &["bosun:state/merged"], "");
        let lease = json!({"bosun": "lease", "session": "s", "origin": "slack",
            "expires": "2026-10-22T09:00:00Z", "touch": [], "thread_ts": ts});
        gh.comment_at(n, "ada", &lease.to_string(), "2026-10-22T09:00:00Z");
        let res = json!({"bosun": "result", "session": "s", "outcome": "merged", "pr": n});
        gh.comment_at(n, "ada", &res.to_string(), "2026-10-22T09:30:00Z");
    }
    gh.comment_at(
        11,
        "ada",
        &json!({"bosun": "lease-end", "session": "s", "reason": "result"}).to_string(),
        "2026-10-22T09:31:00Z",
    );
    let run = |args: &[&str]| {
        let out = bosun()
            .current_dir(dir.path())
            .env("BOSUN_GITHUB_API", gh.url())
            .env("BOSUN_NOW", "2026-10-22T10:00:00Z")
            .arg("notify")
            .args(args)
            .write_stdin(good)
            .output()
            .unwrap();
        (out.status.code().unwrap(), one_object(&out.stdout))
    };
    let posts = |v: &serde_json::Value| -> Vec<(String, String)> {
        v["data"]["pending"]
            .as_array()
            .unwrap()
            .iter()
            .map(|p| {
                (
                    p["key"].as_str().unwrap().to_string(),
                    p["thread_ts"].as_str().unwrap().to_string(),
                )
            })
            .collect()
    };
    // The live thread session lists and acks its own reply.
    let live = run(&["--channel-info", "-", "--thread", LIVE]).1;
    let live = posts(&live);
    assert_eq!(live.len(), 1, "{live:?}");
    assert!(live[0].0.starts_with("result:10:") && live[0].1 == LIVE);
    let (code, v) = run(&["--ack", &live[0].0]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["already"], false);
    // The Routine run lists only the ended thread's reply.
    let routine = run(&["--channel-info", "-"]).1;
    let routine = posts(&routine);
    assert_eq!(routine.len(), 1, "{routine:?}");
    assert!(routine[0].0.starts_with("result:11:") && routine[0].1 == ENDED);
    let (code, v) = run(&["--ack", &routine[0].0]);
    assert_eq!(code, 0, "{v}");
    // Nothing is left, for either run.
    for args in [
        vec!["--channel-info", "-"],
        vec!["--channel-info", "-", "--thread", LIVE],
    ] {
        assert_eq!(run(&args).1["next"], json!([]));
    }
    let acks = gh
        .requests()
        .iter()
        .filter(|r| r.method == "POST" && r.path.ends_with("/issues/1/comments"))
        .count();
    assert_eq!(acks, 2, "one ack each");
}
