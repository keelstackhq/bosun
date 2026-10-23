//! Use cases of M2 (spec §12): plan import.
mod common;

use common::fake::FakeGithub;

/// UC-M2-1: importing the same plan twice creates no duplicate issues.
#[test]
fn uc_m2_1() {
    let (tmp, fake) = (tempfile::tempdir().unwrap(), FakeGithub::start());
    let file = tmp.path().join("plan.md");
    std::fs::write(
        &file,
        "# Plan\n\n## M1: One\n\narea: core\n\n### Tasks\n\n- T1: Do it\n  touch: src/a.rs\n\n### Done when\n\n- UC-M1-1: it works.\n",
    )
    .unwrap();
    let args = [
        "-C",
        tmp.path().to_str().unwrap(),
        "plan",
        "import",
        file.to_str().unwrap(),
    ];
    let posts = |f: &FakeGithub| {
        f.requests()
            .iter()
            .filter(|r| r.method == "POST" && r.path.ends_with("/issues"))
            .count()
    };
    let (code, v) = common::run(&fake, "session_test", "2026-10-23T10:00:00Z", &args);
    assert_eq!(code, 0, "{v}");
    let first = posts(&fake);
    assert_eq!(first, 3);
    let (code, v) = common::run(&fake, "session_test", "2026-10-23T10:00:00Z", &args);
    assert_eq!(code, 0, "{v}");
    assert_eq!(posts(&fake), first);
}

const ONE: &str = "# Plan\n\n## M1: One\n\narea: core\n\n### Tasks\n\n- T1: Do it\n  touch: src/a.rs\n\n### Done when\n\n- UC-M1-1: it works.\n- UC-M1-2: it stays working.\n";

/// UC-M2-2: a task becomes ready only after its plan task merged.
#[test]
fn uc_m2_2() {
    let (tmp, fake) = (tempfile::tempdir().unwrap(), FakeGithub::start());
    let file = tmp.path().join("plan.md");
    std::fs::write(&file, ONE).unwrap();
    fake.issue(
        900,
        "hand made",
        &["bosun:state/planned"],
        r#"<!-- bosun:task {"item":"M1","task":"T9","touch":["src/x.rs"]} -->"#,
    );
    let args = [
        "-C",
        tmp.path().to_str().unwrap(),
        "plan",
        "import",
        file.to_str().unwrap(),
    ];
    let state = |n: u64| -> Vec<String> {
        fake.issue_state(n)
            .labels
            .into_iter()
            .filter(|l| l.starts_with("bosun:state/"))
            .collect()
    };
    let (code, v) = common::run(&fake, "session_test", "2026-10-23T10:00:00Z", &args);
    assert_eq!(code, 0, "{v}");
    let num = |id: &str| -> u64 {
        let all = [&v["data"]["epics"], &v["data"]["tasks"]];
        all.iter()
            .flat_map(|a| a.as_array().unwrap())
            .find(|t| t["id"] == id)
            .unwrap()["issue"]
            .as_u64()
            .unwrap()
    };
    let (epic, t1, p) = (num("M1"), num("M1/T1"), num("M1/P"));
    assert_eq!(state(t1), ["bosun:state/planned"]);
    {
        let mut s = fake.state.lock().unwrap();
        let i = s.issues.iter_mut().find(|i| i.number == p).unwrap();
        i.labels = vec!["bosun:state/merged".into()];
    }
    let (code, v) = common::run(&fake, "session_test", "2026-10-23T10:00:00Z", &args);
    assert_eq!(code, 0, "{v}");
    assert_eq!(state(t1), ["bosun:state/ready"]);
    assert_eq!(state(epic), ["bosun:state/ready"]);
    assert_eq!(state(900), ["bosun:state/planned"]);
}

/// UC-M2-3: each "Done when" line has a pending UC test after import.
#[test]
fn uc_m2_3() {
    let (tmp, fake) = (tempfile::tempdir().unwrap(), FakeGithub::start());
    std::fs::create_dir_all(tmp.path().join(".bosun")).unwrap();
    std::fs::write(
        tmp.path().join(".bosun/bosun.toml"),
        "project = \"widget\"\n\n[[stack]]\npath = \".\"\nprofile = \"rust\"\n",
    )
    .unwrap();
    let file = tmp.path().join("plan.md");
    std::fs::write(&file, ONE).unwrap();
    let args = [
        "-C",
        tmp.path().to_str().unwrap(),
        "plan",
        "import",
        file.to_str().unwrap(),
    ];
    let (code, v) = common::run(&fake, "session_test", "2026-10-23T10:00:00Z", &args);
    assert_eq!(code, 0, "{v}");
    let path = tmp.path().join("tests/uc_m1.rs");
    let text = std::fs::read_to_string(&path).unwrap();
    for uc in ["UC-M1-1", "UC-M1-2"] {
        assert!(text.contains(&format!("#[ignore = \"{uc}\"]")), "{uc}");
    }
    let (code, _) = common::run(&fake, "session_test", "2026-10-23T10:00:00Z", &args);
    assert_eq!(code, 0);
    assert_eq!(std::fs::read_to_string(&path).unwrap(), text);
}

/// UC-M2-4: an amendment that removes an item closes its epic as not planned.
#[test]
fn uc_m2_4() {
    let (tmp, fake) = (tempfile::tempdir().unwrap(), FakeGithub::start());
    let file = tmp.path().join("plan.md");
    let two = format!(
        "{ONE}\n## M2: Two\n\narea: core\n\n### Tasks\n\n- T1: Other\n  touch: src/b.rs\n\n### Done when\n\n- UC-M2-1: it also works.\n"
    );
    std::fs::write(&file, two).unwrap();
    let run = |cmd: &str| {
        let args = [
            "-C",
            tmp.path().to_str().unwrap(),
            "plan",
            cmd,
            file.to_str().unwrap(),
        ];
        common::run(&fake, "session_test", "2026-10-23T10:00:00Z", &args)
    };
    let (code, v) = run("import");
    assert_eq!(code, 0, "{v}");
    let epic = v["data"]["epics"][1]["issue"].as_u64().unwrap();
    std::fs::write(&file, ONE).unwrap();
    let (code, v) = run("amend");
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["closed"][0], "M2");
    let i = fake.issue_state(epic);
    assert_eq!(i.state, "closed");
    assert_eq!(i.state_reason.as_deref(), Some("not_planned"));
}
