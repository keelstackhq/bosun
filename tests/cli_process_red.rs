mod common;

use common::fake::FakeGithub;
use common::run;

const NOW: &str = "2026-10-22T10:00:00Z";
const LATER: &str = "2026-10-22T10:20:00Z";

fn task(kind: &str) -> String {
    format!(
        r#"Do it.

<!-- bosun:task {{"item":"M1","task":"T1","epic":10,"touch":["src/a.rs"],"kind":"{kind}"}} -->"#
    )
}

fn red(fake: &FakeGithub) {
    fake.issue(
        99,
        "lesson",
        &["bosun:lesson", "bosun:lesson/blocking", "bosun:process-red"],
        "",
    );
}

#[test]
fn claim_is_refused_for_features_and_allowed_for_process_fix() {
    let fake = FakeGithub::start();
    fake.issue(11, "feature", &["bosun:state/ready"], &task("feature"));
    fake.issue(12, "fix", &["bosun:state/ready"], &task("process-fix"));
    red(&fake);
    let (code, v) = run(&fake, "session_a", NOW, &["claim", "11"]);
    assert_eq!(code, 4, "{v}");
    assert!(v["error"]["message"].as_str().unwrap().contains("#99"));
    assert!(!v["error"]["fix"].as_str().unwrap().is_empty());
    assert_eq!(run(&fake, "session_a", NOW, &["claim", "12"]).0, 0);
}

#[test]
fn a_closed_process_red_issue_does_not_pause() {
    let fake = FakeGithub::start();
    fake.issue(11, "feature", &["bosun:state/ready"], &task("feature"));
    red(&fake);
    fake.state.lock().unwrap().issues[1].state = "closed".into();
    assert_eq!(run(&fake, "session_a", NOW, &["claim", "11"]).0, 0);
}

#[test]
fn spawn_is_refused_except_for_a_process_fix_epic() {
    let fake = FakeGithub::start();
    fake.issue(1, "bosun registry", &["bosun:registry"], "");
    fake.comment(
        1,
        &format!(
            r#"{{"bosun":"join","session":"session_test","origin":"spawned","depth":1,"expires":"{LATER}"}}"#
        ),
    );
    fake.issue(
        10,
        "Epic",
        &["bosun:state/ready", "bosun:area/lifecycle"],
        "",
    );
    red(&fake);
    let (code, v) = run(&fake, "session_test", NOW, &["spawn", "10"]);
    assert_eq!(code, 4, "{v}");
    assert_eq!(v["data"]["check"], 1);

    fake.issue(
        13,
        "Fix epic",
        &[
            "bosun:state/ready",
            "bosun:area/lifecycle",
            "bosun:kind/process-fix",
        ],
        "",
    );
    let (code, v) = run(&fake, "session_test", NOW, &["spawn", "13"]);
    assert_eq!(code, 0, "{v}");
}

#[test]
fn wave_plan_pauses_except_for_a_process_fix_epic() {
    let fake = FakeGithub::start();
    fake.issue(10, "Epic", &["bosun:state/claimed"], "");
    fake.issue(11, "task", &["bosun:state/ready"], &task("feature"));
    fake.issue(14, "Fix", &["bosun:kind/process-fix"], "");
    red(&fake);
    let (code, v) = run(&fake, "session_a", NOW, &["wave", "plan", "10"]);
    assert_eq!(code, 4, "{v}");
    assert!(v["error"]["message"].as_str().unwrap().contains("#99"));
    assert_eq!(run(&fake, "session_a", NOW, &["wave", "plan", "14"]).0, 0);
}

#[test]
fn a_mainfix_task_is_claimed_under_process_red() {
    let fake = FakeGithub::start();
    fake.issue(11, "fix main", &["bosun:state/ready"], &task("mainfix"));
    red(&fake);
    assert_eq!(run(&fake, "session_a", NOW, &["claim", "11"]).0, 0);
}

#[test]
fn wave_plan_pauses_while_main_is_red_except_for_a_mainfix_epic() {
    let fake = FakeGithub::start();
    fake.issue(10, "Epic", &["bosun:state/claimed"], "");
    fake.issue(11, "task", &["bosun:state/ready"], &task("feature"));
    fake.issue(15, "Fix", &["bosun:kind/mainfix"], "");
    fake.issue(98, "main is red", &["bosun:main-red"], "");
    let (code, v) = run(&fake, "session_a", NOW, &["wave", "plan", "10"]);
    assert_eq!(code, 4, "{v}");
    assert!(v["error"]["message"].as_str().unwrap().contains("#98"));
    assert_eq!(run(&fake, "session_a", NOW, &["wave", "plan", "15"]).0, 0);
}

#[test]
fn wave_plan_under_process_red_runs_mainfix_epics_and_only_process_fix_tasks() {
    let fake = FakeGithub::start();
    fake.issue(10, "Epic", &["bosun:state/claimed"], "");
    fake.issue(11, "feature", &["bosun:state/ready"], &task("feature"));
    fake.issue(15, "Fix", &["bosun:kind/mainfix"], "");
    red(&fake);
    assert_eq!(run(&fake, "session_a", NOW, &["wave", "plan", "15"]).0, 0);

    let body = r#"<!-- bosun:task {"item":"M1","task":"T2","epic":10,"touch":["src/b.rs"],"kind":"process-fix"} -->"#;
    fake.issue(12, "process fix", &["bosun:state/ready"], body);
    let (code, v) = run(&fake, "session_a", NOW, &["wave", "plan", "10"]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["run"][0]["issue"], 12);
    assert_eq!(v["data"]["run"].as_array().unwrap().len(), 1);
    assert_eq!(v["data"]["waiting"][0]["issue"], 11);
    assert_eq!(
        v["data"]["waiting"][0]["reason"],
        "process is red (#99): only process-fix tasks run"
    );
}

fn in_flight_feature(fake: &FakeGithub) {
    fake.issue(10, "Epic", &["bosun:state/claimed"], "");
    let body = task("feature").replace(r#""T1""#, r#""T0""#);
    fake.issue(11, "running", &["bosun:state/building"], &body);
    fake.comment(11, &common::lease_note("other", LATER, "src/a.rs"));
    red(fake);
}

#[test]
fn process_fix_tasks_respect_max_parallel_of_in_flight_work() {
    let fake = FakeGithub::start();
    in_flight_feature(&fake);
    let body = r#"<!-- bosun:task {"item":"M1","task":"T2","epic":10,"touch":["src/b.rs"],"kind":"process-fix"} -->"#;
    fake.issue(12, "process fix", &["bosun:state/ready"], body);
    // wave plan reads the limits from the default branch (spec §5).
    fake.content(".bosun/limits.toml", "main", "[waves]\nmax_parallel = 1\n");
    let dir = std::env::temp_dir().join(format!("bosun-pr-wave-{}", std::process::id()));
    std::fs::create_dir_all(dir.join(".bosun")).unwrap();
    std::fs::write(
        dir.join(".bosun/limits.toml"),
        "[waves]\nmax_parallel = 1\n",
    )
    .unwrap();
    let out = common::bosun()
        .env("BOSUN_GITHUB_API", fake.url())
        .arg("-C")
        .arg(&dir)
        .args(["wave", "plan", "10"])
        .output()
        .unwrap();
    let v = common::one_object(&out.stdout);
    assert_eq!(out.status.code(), Some(0), "{v}");
    assert!(v["data"]["run"].as_array().unwrap().is_empty(), "{v}");
    let waiting = v["data"]["waiting"].to_string();
    assert!(waiting.contains("waves.max_parallel is 1"), "{waiting}");
}

#[test]
fn wave_plan_stays_halted_when_no_process_fix_task_is_ready() {
    let fake = FakeGithub::start();
    in_flight_feature(&fake);
    let (code, v) = run(&fake, "session_a", NOW, &["wave", "plan", "10"]);
    assert_eq!(code, 4, "{v}");
}
