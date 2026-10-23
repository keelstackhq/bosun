mod common;

use common::fake::FakeGithub;
use common::{bosun, one_object};
use serde_json::Value;

const T0: &str = "2026-10-23T10:00:00Z";

fn triage(dir: &std::path::Path, fake: &FakeGithub, args: &[&str]) -> (i32, Value) {
    let out = bosun()
        .env("BOSUN_GITHUB_API", fake.url())
        .env("BOSUN_NOW", T0)
        .arg("-C")
        .arg(dir)
        .args(["lesson", "triage"])
        .args(args)
        .output()
        .unwrap();
    let v = one_object(&out.stdout);
    if v["ok"] == false {
        assert!(!v["error"]["fix"].as_str().unwrap().trim().is_empty());
    }
    (out.status.code().unwrap(), v)
}

fn setup() -> (tempfile::TempDir, FakeGithub) {
    let fake = FakeGithub::start();
    fake.issue(
        5,
        "lesson: gate was slow",
        &["bosun:lesson", "bosun:source/lesson"],
        "body",
    );
    (tempfile::tempdir().unwrap(), fake)
}

fn writes(fake: &FakeGithub) -> usize {
    fake.requests().iter().filter(|r| r.method != "GET").count()
}

#[test]
fn blocking_labels_the_lesson_and_sets_process_red() {
    let (tmp, fake) = setup();
    let (code, v) = triage(tmp.path(), &fake, &["5", "--as", "blocking"]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["command"], "lesson triage");
    assert_eq!(v["data"]["as"], "blocking");
    assert_eq!(v["data"]["process_red"], 5);
    let labels = fake.labels_of(5);
    assert!(labels.contains(&"bosun:lesson/blocking".into()));
    assert!(labels.contains(&"bosun:process-red".into()));
    let comments = fake.comments_of(5);
    assert!(comments[0].body.contains("Triaged as blocking"));
}

#[test]
fn mechanizable_opens_a_process_fix_task() {
    let (tmp, fake) = setup();
    let args = [
        "5",
        "--as",
        "mechanizable",
        "--touch",
        "src/gates/**",
        "--area",
        "gates",
    ];
    let (code, v) = triage(tmp.path(), &fake, &args);
    assert_eq!(code, 0, "{v}");
    let task = v["data"]["task"].as_u64().unwrap();
    let t = fake.issue_state(task);
    assert_eq!(t.title, "process-fix: gate was slow");
    for l in [
        "bosun:kind/process-fix",
        "bosun:source/lesson",
        "bosun:state/ready",
        "bosun:area/gates",
    ] {
        assert!(t.labels.contains(&l.to_string()), "{l} in {:?}", t.labels);
    }
    assert!(t.body.contains("Mechanize lesson #5"));
    assert!(
        t.body
            .contains(&format!("`Closes #{task}` and `Closes #5`"))
    );
    let meta = bosun::core::meta::parse_task_meta(&t.body).unwrap();
    assert_eq!(meta.item, "lesson");
    assert_eq!(meta.task, "#5");
    assert_eq!(meta.touch, ["src/gates/**"]);
    assert_eq!(meta.kind, "process-fix");
    assert_eq!(meta.risk, "normal");
    assert!(
        fake.labels_of(5)
            .contains(&"bosun:lesson/mechanizable".into())
    );
    assert!(fake.comments_of(5)[0].body.contains(&format!("#{task}")));
}

#[test]
fn advice_adds_to_the_local_store() {
    let (tmp, fake) = setup();
    let args = [
        "5",
        "--as",
        "advice",
        "--rule",
        "Run it once",
        "--scope",
        "src/**",
    ];
    let (code, v) = triage(tmp.path(), &fake, &args);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["lesson"], "L-001");
    let text = std::fs::read_to_string(tmp.path().join("docs/lessons.toml")).unwrap();
    let t: toml::Table = text.parse().unwrap();
    let l = &t["lesson"][0];
    assert_eq!(l["rule"].as_str(), Some("Run it once"));
    assert_eq!(l["why"].as_str(), Some("gate was slow"));
    assert_eq!(l["evidence"][0].as_str(), Some("#5"));
    assert!(fake.labels_of(5).contains(&"bosun:lesson/advice".into()));
    let body = &fake.comments_of(5)[0].body;
    assert!(body.contains("L-001") && body.contains("Closes #5"));
}

#[test]
fn retriage_removes_the_other_labels_and_process_red() {
    let (tmp, fake) = setup();
    triage(tmp.path(), &fake, &["5", "--as", "blocking"]);
    let (code, v) = triage(tmp.path(), &fake, &["5", "--as", "advice", "--rule", "r"]);
    assert_eq!(code, 0, "{v}");
    let labels = fake.labels_of(5);
    assert!(labels.contains(&"bosun:lesson/advice".into()));
    assert!(!labels.contains(&"bosun:lesson/blocking".into()));
    assert!(!labels.contains(&"bosun:process-red".into()));
}

#[test]
fn a_non_lesson_or_closed_or_missing_issue_is_exit_1() {
    let (tmp, fake) = setup();
    fake.issue(6, "task", &["bosun:state/ready"], "");
    fake.issue(7, "lesson: old", &["bosun:lesson"], "");
    fake.close_issue(7, T0);
    for n in ["6", "7", "99"] {
        let (code, v) = triage(tmp.path(), &fake, &[n, "--as", "blocking"]);
        assert_eq!(code, 1, "{n}: {v}");
    }
}

#[test]
fn missing_touch_or_rule_is_usage() {
    let (tmp, fake) = setup();
    let (code, _) = triage(tmp.path(), &fake, &["5", "--as", "mechanizable"]);
    assert_eq!(code, 2);
    let (code, _) = triage(tmp.path(), &fake, &["5", "--as", "advice"]);
    assert_eq!(code, 2);
    assert_eq!(writes(&fake), 0);
}

#[test]
fn halted_changes_nothing() {
    let (tmp, fake) = setup();
    fake.variable("BOSUN_HALT", "1");
    let (code, _) = triage(tmp.path(), &fake, &["5", "--as", "blocking"]);
    assert_eq!(code, 4);
    assert_eq!(writes(&fake), 0);
}

#[test]
fn advice_at_the_cap_is_exit_6_and_writes_nothing_to_github() {
    let (tmp, fake) = setup();
    std::fs::create_dir_all(tmp.path().join("docs")).unwrap();
    let mut text = String::new();
    for i in 1..=40 {
        text.push_str(&format!(
            "[[lesson]]\nid = \"L-{i:03}\"\nrule = \"r{i}\"\nscope = [\"a/**\"]\n\n"
        ));
    }
    std::fs::write(tmp.path().join("docs/lessons.toml"), &text).unwrap();
    let (code, v) = triage(tmp.path(), &fake, &["5", "--as", "advice", "--rule", "x"]);
    assert_eq!(code, 6, "{v}");
    assert_eq!(writes(&fake), 0);
    assert_eq!(
        std::fs::read_to_string(tmp.path().join("docs/lessons.toml")).unwrap(),
        text
    );
}
