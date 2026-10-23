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
