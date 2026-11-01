//! Wave-agent identity (F12): a per-task id `<owner>/<task>` claims inside its owner's area,
//! the spawn prompt names the child's own id, and `join` cannot point a session at itself
//! nor keep a root session stuck at a deeper depth.

mod common;

use common::fake::FakeGithub;
use common::run;

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

fn task(fake: &FakeGithub, n: u64) {
    fake.issue(
        n,
        "task",
        &["bosun:state/ready", "bosun:area/cli"],
        r#"Do it.

<!-- bosun:task {"item":"M1","task":"T1","touch":["src/a.rs"],"kind":"feature"} -->"#,
    );
}

#[test]
fn a_per_task_id_claims_inside_its_owners_area() {
    let fake = FakeGithub::start();
    fake.issue(1, "bosun registry", &["bosun:registry"], "");
    fake.comment(1, &member("session_root", None, 1, None));
    fake.comment(
        1,
        &member("session_child", Some("session_root"), 2, Some("cli")),
    );
    task(&fake, 10);
    task(&fake, 11);

    let (code, v) = run(&fake, "session_other/T1", NOW, &["claim", "10"]);
    assert_eq!(code, 3, "another session's wave agent is refused: {v}");
    assert!(
        v["error"]["message"]
            .as_str()
            .unwrap()
            .contains("session_child")
    );

    let (code, v) = run(&fake, "session_child/T1", NOW, &["claim", "10"]);
    assert_eq!(code, 0, "the owner's wave agent claims: {v}");
    assert_eq!(v["data"]["lease"]["session"], "session_child/T1");
}

#[test]
fn the_spawn_prompt_names_the_childs_own_id() {
    let fake = FakeGithub::start();
    fake.issue(1, "bosun registry", &["bosun:registry"], "");
    fake.comment(1, &member("session_test", None, 1, None));
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
        !prompt.contains("BOSUN_SESSION=session_test/"),
        "not the spawner's id: {prompt}"
    );
    assert!(
        prompt.contains("BOSUN_SESSION=<session>/<task>"),
        "{prompt}"
    );
    assert!(prompt.contains("`data.session`"), "{prompt}");
}

#[test]
fn join_refuses_itself_as_parent() {
    let fake = FakeGithub::start();
    let (code, v) = run(
        &fake,
        "session_a",
        NOW,
        &["join", "--parent", "session_a", "--depth", "2"],
    );
    assert_eq!(code, 1, "{v}");
    assert!(!v["error"]["fix"].as_str().unwrap().is_empty());
    assert!(
        !fake
            .state
            .lock()
            .unwrap()
            .comments
            .iter()
            .any(|c| c.body.contains("session_a")),
        "nothing is written"
    );
}

#[test]
fn join_depth_lowers_a_root_record_but_not_below_its_parent() {
    let fake = FakeGithub::start();
    // A root session that joined with a wrong depth gets it back down.
    run(
        &fake,
        "session_root",
        NOW,
        &["join", "--area", "cli", "--depth", "2"],
    );
    let (code, v) = run(&fake, "session_root", NOW, &["join", "--depth", "1"]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["depth"], 1);
    assert_eq!(v["data"]["area"], "cli");

    // A spawned child stays below its parent: depth 2 under a depth-1 parent.
    run(
        &fake,
        "session_child",
        NOW,
        &["join", "--parent", "session_root", "--depth", "2"],
    );
    let (code, v) = run(&fake, "session_child", NOW, &["join", "--depth", "1"]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["depth"], 2, "never above the parent's depth + 1");

    // Without --depth the depth is kept.
    let (_, v) = run(&fake, "session_root", NOW, &["join"]);
    assert_eq!(v["data"]["depth"], 1);
}

#[test]
fn a_record_that_names_itself_as_parent_keeps_its_depth() {
    let fake = FakeGithub::start();
    fake.issue(1, "bosun registry", &["bosun:registry"], "");
    fake.comment(1, &member("session_a", Some("session_a"), 2, Some("cli")));
    let (code, v) = run(&fake, "session_a", NOW, &["join", "--depth", "1"]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["depth"], 2, "not reset to a root's depth");
    assert!(!fake.comments_of(1)[0].body.contains(r#""parent""#));
}

#[test]
fn an_expired_parent_keeps_the_old_depth_as_the_floor() {
    let fake = FakeGithub::start();
    fake.issue(1, "bosun registry", &["bosun:registry"], "");
    fake.comment(
        1,
        r#"{"bosun":"join","session":"session_p","origin":"local","depth":1,"expires":"2026-10-22T08:00:00Z"}"#,
    );
    fake.comment(1, &member("session_c", Some("session_p"), 3, None));
    let (code, v) = run(&fake, "session_c", NOW, &["join", "--depth", "1"]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["depth"], 3);
}

#[test]
fn spawn_record_refuses_the_spawner_as_its_own_child() {
    let fake = FakeGithub::start();
    fake.issue(1, "bosun registry", &["bosun:registry"], "");
    fake.comment(1, &member("session_test", None, 1, None));
    fake.issue(
        10,
        "Epic",
        &["bosun:state/ready", "bosun:area/lifecycle"],
        "",
    );
    let (code, v) = run(&fake, "session_test", NOW, &["spawn", "10"]);
    assert_eq!(code, 0, "{v}");
    let before = fake.comments_of(1).len();
    let out = common::bosun()
        .env("BOSUN_GITHUB_API", fake.url())
        .env("BOSUN_SESSION", "session_test")
        .env("BOSUN_NOW", NOW)
        .args(["spawn", "10", "--record", "--from", "-"])
        .write_stdin(r#"{"id":"session_test"}"#)
        .output()
        .unwrap();
    let v = common::one_object(&out.stdout);
    assert_eq!(out.status.code(), Some(1), "{v}");
    assert!(!v["error"]["fix"].as_str().unwrap().is_empty());
    assert_eq!(fake.comments_of(1).len(), before, "no join is written");
}

#[test]
fn spoofed_per_task_ids_do_not_claim_in_the_owners_area() {
    let fake = FakeGithub::start();
    fake.issue(1, "bosun registry", &["bosun:registry"], "");
    fake.comment(1, &member("session_root", None, 1, None));
    fake.comment(
        1,
        &member("session_child", Some("session_root"), 2, Some("cli")),
    );
    task(&fake, 10);
    for id in [
        "session_childX/T1",
        "session_other/../session_child",
        "session_chil/d",
        "/session_child",
    ] {
        let (code, v) = run(&fake, id, NOW, &["claim", "10"]);
        assert_eq!(code, 3, "{id} is not a wave agent of session_child: {v}");
    }
}
