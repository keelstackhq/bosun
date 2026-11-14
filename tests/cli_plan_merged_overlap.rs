//! `plan check` gives no overlap warning for a pair of merged tasks.

mod common;

use common::fake::FakeGithub;
use serde_json::Value;
use std::path::Path;

const T0: &str = "2026-10-23T10:00:00Z";
const PLAN: &str = "# Plan: x\n\nproject: widget\n\n## M1: One\n\narea: a\n\n### Tasks\n\n- T1: t\n  touch: src/a.rs\n- T2: t\n  touch: src/a.rs\n\n### Done when\n\n- UC-M1-1: ok\n";

fn check(fake: &FakeGithub, dir: &Path) -> (i32, Value) {
    std::fs::write(dir.join("plan.md"), PLAN).unwrap();
    let file = dir.join("plan.md");
    common::run(
        fake,
        "session_test",
        T0,
        &[
            "-C",
            dir.to_str().unwrap(),
            "plan",
            "check",
            file.to_str().unwrap(),
        ],
    )
}

#[test]
fn a_pair_of_merged_tasks_gets_no_overlap_warning() {
    let (tmp, fake) = (tempfile::tempdir().unwrap(), FakeGithub::start());
    let (code, v) = check(&fake, tmp.path());
    assert_eq!(code, 0, "{v}");
    assert!(v.to_string().contains("overlapping"), "{v}");
    for (n, t) in [(40, "T1"), (41, "T2")] {
        fake.issue(
            n,
            &format!("M1/{t}: t"),
            &["bosun:state/merged"],
            &format!("<!-- bosun:plan M1/{t} -->\n"),
        );
    }
    let (code, v) = check(&fake, tmp.path());
    assert_eq!(code, 0, "{v}");
    assert!(!v.to_string().contains("overlapping"), "{v}");
}
