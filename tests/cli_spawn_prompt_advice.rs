//! The spawn prompt carries working advice from first user repos: one shared build cache,
//! what to delete when the disk is full, and the 30-minute limit of background waits.

mod common;

use common::fake::FakeGithub;
use common::run;

const NOW: &str = "2026-10-22T10:00:00Z";

#[test]
fn the_spawn_prompt_carries_the_working_advice() {
    let fake = FakeGithub::start();
    fake.issue(1, "bosun registry", &["bosun:registry"], "");
    fake.comment(
        1,
        r#"{"bosun":"join","session":"session_test","origin":"local","depth":1,"expires":"2026-10-22T10:20:00Z"}"#,
    );
    fake.issue(
        10,
        "Epic",
        &["bosun:state/ready", "bosun:area/lifecycle"],
        "",
    );
    let (code, v) = run(&fake, "session_test", NOW, &["spawn", "10"]);
    assert_eq!(code, 0, "{v}");
    let prompt = v["next"][0]["args"]["prompt"].as_str().unwrap();
    for needle in [
        "its own CARGO_TARGET_DIR",
        "sccache",
        "delete only build caches",
        "by default",
        "bosun msg session_test",
    ] {
        assert!(prompt.contains(needle), "{needle}: {prompt}");
    }
    assert!(!prompt.contains("one shared CARGO_TARGET_DIR"), "{prompt}");
}
