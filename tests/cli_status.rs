mod common;

use common::fake::FakeGithub;
use common::{lease_note, run};
use serde_json::json;

const T0: &str = "2026-10-22T10:00:00Z";

fn join_note(session: &str, expires: &str) -> String {
    format!(
        "{{\"bosun\":\"join\",\"session\":\"{session}\",\"origin\":\"spawned\",\"area\":\"gates\",\"depth\":2,\"expires\":\"{expires}\"}}"
    )
}

fn spend(usd: f64, at: &str) -> String {
    format!("{{\"bosun\":\"spend\",\"session\":\"s\",\"usd\":{usd},\"at\":\"{at}\"}}")
}

#[test]
fn status_on_an_empty_repo() {
    let fake = FakeGithub::start();
    let (code, v) = run(&fake, "session_a", T0, &["status"]);
    assert_eq!(code, 0, "{v}");
    let d = &v["data"];
    assert_eq!(d["session"], "session_a");
    assert_eq!(d["origin"], "local");
    assert_eq!(d["repo"], "acme/widget");
    assert_eq!(d["halted"], false);
    assert!(d["main_red"].is_null() && d["process_red"].is_null());
    assert_eq!(d["live_sessions"], json!([]));
    assert_eq!(d["leases"], json!([]));
    assert_eq!(d["issues_by_state"], json!({}));
    assert_eq!(d["spend_today_usd"], 0.0);
    assert_eq!(d["budget_pct"], 0.0);
    assert_eq!(d["integrity"], "full");
}

#[test]
fn status_with_a_seeded_repo() {
    let fake = FakeGithub::start();
    fake.variable("BOSUN_HALT", "1");
    fake.issue(1, "bosun registry", &["bosun:registry"], "");
    fake.comment(1, &join_note("session_b", "2026-10-22T10:20:00Z"));
    fake.comment(1, &join_note("session_old", "2026-10-22T09:00:00Z"));
    fake.comment(1, &spend(15.0, "2026-10-22T08:00:00Z"));
    fake.comment(1, &spend(99.0, "2026-10-21T23:00:00Z"));
    fake.issue(2, "red", &["bosun:main-red"], "");
    fake.issue(3, "proc", &["bosun:process-red"], "");
    fake.issue(10, "t", &["bosun:state/building"], "");
    fake.comment(
        10,
        &lease_note("session_b", "2026-10-22T10:20:00Z", "src/gates/**"),
    );
    fake.issue(11, "t", &["bosun:state/ready"], "");
    fake.issue(12, "t", &["bosun:state/ready"], "");
    let (code, v) = run(&fake, "session_a", T0, &["status"]);
    assert_eq!(code, 0, "{v}");
    let d = &v["data"];
    assert_eq!(d["halted"], true);
    assert_eq!(d["main_red"], 2);
    assert_eq!(d["process_red"], 3);
    assert_eq!(
        d["live_sessions"],
        json!([{"session": "session_b", "origin": "spawned", "area": "gates", "depth": 2,
                "expires": "2026-10-22T10:20:00Z"}])
    );
    assert_eq!(
        d["leases"],
        json!([{"issue": 10, "session": "session_b", "expires": "2026-10-22T10:20:00Z",
                "touch": ["src/gates/**"]}])
    );
    assert_eq!(d["issues_by_state"], json!({"building": 1, "ready": 2}));
    assert_eq!(d["spend_today_usd"], 15.0);
    assert_eq!(d["budget_pct"], 10.0);
}

#[test]
fn budget_pct_uses_the_daily_usd_of_the_default_branch() {
    let (tmp, fake) = (tempfile::tempdir().unwrap(), FakeGithub::start());
    std::fs::create_dir_all(tmp.path().join(".bosun")).unwrap();
    std::fs::write(
        tmp.path().join(".bosun/limits.toml"),
        "[budget]\ndaily_usd = 150\n",
    )
    .unwrap();
    fake.content(".bosun/limits.toml", "main", "[budget]\ndaily_usd = 100\n");
    fake.issue(1, "bosun registry", &["bosun:registry"], "");
    fake.comment(1, &spend(50.0, "2026-10-22T08:00:00Z"));
    let out = common::bosun()
        .env("BOSUN_GITHUB_API", fake.url())
        .env("BOSUN_SESSION", "session_a")
        .env("BOSUN_NOW", T0)
        .arg("-C")
        .arg(tmp.path())
        .arg("status")
        .output()
        .unwrap();
    let v = common::one_object(&out.stdout);
    assert_eq!(out.status.code(), Some(0), "{v}");
    assert_eq!(v["data"]["budget_pct"], 50.0);
    assert_eq!(v["data"]["daily_usd"], 100.0);
}

#[test]
fn a_custom_profile_means_reduced_integrity() {
    let (tmp, fake) = (tempfile::tempdir().unwrap(), FakeGithub::start());
    std::fs::create_dir_all(tmp.path().join(".bosun")).unwrap();
    std::fs::write(
        tmp.path().join(".bosun/bosun.toml"),
        "[[stack]]\npath = \".\"\nprofile = \"mine\"\n",
    )
    .unwrap();
    let (code, v) = run(
        &fake,
        "session_a",
        T0,
        &["-C", tmp.path().to_str().unwrap(), "status"],
    );
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["integrity"], "reduced");
}
