mod common;

use common::fake::FakeGithub;
use common::{lease_note, one_object, run};
use serde_json::Value;

const NOW: &str = "2026-10-22T10:00:00Z";
const LATER: &str = "2026-10-22T10:20:00Z";

fn task(fake: &FakeGithub, n: u64, id: &str, state: &str, epic: u64, depends: &str, touch: &str) {
    let (item, t) = id.split_once('/').unwrap();
    let body = format!(
        r#"<!-- bosun:task {{"item":"{item}","task":"{t}","epic":{epic},"depends":[{depends}],"touch":[{touch}]}} -->"#
    );
    fake.issue(
        n,
        &format!("Task {id}"),
        &[&format!("bosun:state/{state}")],
        &body,
    );
}

fn epic() -> FakeGithub {
    let fake = FakeGithub::start();
    fake.issue(10, "Epic", &["bosun:state/claimed"], "");
    fake
}

fn plan(fake: &FakeGithub) -> Value {
    let (code, v) = run(fake, "session_test", NOW, &["wave", "plan", "10"]);
    assert_eq!(code, 0, "{v}");
    v
}

fn issues(list: &Value, key: &str) -> Vec<u64> {
    list.as_array()
        .unwrap()
        .iter()
        .map(|t| t[key].as_u64().unwrap())
        .collect()
}

#[test]
fn dependencies_overlap_and_merged_tasks_decide_the_wave() {
    let fake = epic();
    task(&fake, 11, "M1/T1", "ready", 10, "", r#""src/a/**""#);
    task(
        &fake,
        12,
        "M1/T2",
        "ready",
        10,
        r#""M1/T1""#,
        r#""src/b/**""#,
    );
    task(&fake, 13, "M1/T3", "ready", 10, "", r#""src/a/x.rs""#);
    task(
        &fake,
        14,
        "M1/T4",
        "ready",
        10,
        r#""M0/T1""#,
        r#""docs/**""#,
    );
    task(&fake, 15, "M1/T5", "ready", 11, "", r#""other/**""#);
    // M0/T1 is merged (closed, in another epic), so #14 may run.
    task(&fake, 16, "M0/T1", "merged", 9, "", r#""x/**""#);
    fake.state.lock().unwrap().issues[5].state = "closed".into();

    let v = plan(&fake);
    assert_eq!(v["data"]["epic"], 10);
    let run = &v["data"]["run"];
    assert_eq!(issues(run, "issue"), vec![11, 14]);
    assert_eq!(run[0]["title"], "Task M1/T1");
    assert_eq!(run[0]["plan_id"], "M1/T1");
    assert_eq!(run[0]["touch"][0], "src/a/**");
    assert_eq!(run[0]["workspace"], "berth up m1-t1");
    let waiting = v["data"]["waiting"].as_array().unwrap();
    assert_eq!(issues(&v["data"]["waiting"], "issue"), vec![12, 13]);
    assert_eq!(waiting[0]["reason"], "waits for M1/T1");
    assert!(waiting[1]["reason"].as_str().unwrap().contains("#11"));
}

#[test]
fn a_live_lease_of_another_session_blocks_a_task() {
    let fake = epic();
    task(&fake, 11, "M1/T1", "ready", 10, "", r#""src/a/**""#);
    task(&fake, 12, "M1/T2", "building", 10, "", r#""src/b/**""#);
    fake.comment(12, &lease_note("other", LATER, "src/a/**"));
    let v = plan(&fake);
    assert!(v["data"]["run"].as_array().unwrap().is_empty());
    let why = v["data"]["waiting"][0]["reason"].as_str().unwrap();
    assert!(why.contains("leased on #12 by other"), "{why}");
    assert_eq!(v["data"]["waiting"][1]["reason"], "state is building");
}

#[test]
fn max_parallel_comes_from_limits() {
    let fake = epic();
    for n in 11..14 {
        task(
            &fake,
            n,
            &format!("M1/T{}", n - 10),
            "ready",
            10,
            "",
            &format!(r#""d{n}/**""#),
        );
    }
    let dir = std::env::temp_dir().join(format!("bosun-wave-{}", std::process::id()));
    std::fs::create_dir_all(dir.join(".bosun")).unwrap();
    std::fs::write(
        dir.join(".bosun/limits.toml"),
        "[waves]\nmax_parallel = 2\n",
    )
    .unwrap();
    let out = common::bosun()
        .env("BOSUN_GITHUB_API", fake.url())
        .arg("-C")
        .arg(&dir)
        .args(["wave", "plan", "10"])
        .output()
        .unwrap();
    let v = one_object(&out.stdout);
    assert_eq!(out.status.code(), Some(0), "{v}");
    assert_eq!(issues(&v["data"]["run"], "issue"), vec![11, 12]);
    assert_eq!(v["data"]["waiting"][0]["reason"], "waves.max_parallel is 2");
}

#[test]
fn a_missing_epic_is_exit_1() {
    let fake = FakeGithub::start();
    let (code, v) = run(&fake, "session_test", NOW, &["wave", "plan", "99"]);
    assert_eq!(code, 1, "{v}");
}
