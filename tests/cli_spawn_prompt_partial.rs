//! The spawn prompt lists every `bosun result` outcome, `partial` included (F13).

mod common;

use common::fake::FakeGithub;
use common::run;

const NOW: &str = "2026-10-22T10:00:00Z";

#[test]
fn the_spawn_prompt_offers_partial() {
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
    assert!(
        prompt.contains("merged, partial, blocked or needs-human"),
        "{prompt}"
    );
    assert!(
        prompt.contains("`partial` when some tasks merged"),
        "{prompt}"
    );
}
