//! Pure parts of `bosun metrics` (plan PR 5): CI facts of a head, the revert marker, p50, risk rules.
use bosun::core::metrics::*;
use bosun::time::Timestamp;

fn run(name: &str, app: &str, done: bool, conclusion: &str, at: Option<i64>) -> CiRun {
    CiRun {
        name: name.into(),
        app: app.into(),
        completed: done,
        conclusion: Some(conclusion.into()),
        at: at.map(Timestamp),
    }
}
fn ok(c: &str, at: i64) -> CiRun {
    run("ci-ok", "github-actions", true, c, Some(at))
}

fn job(c: &str) -> CiRun {
    run("ci", "github-actions", true, c, None)
}

#[test]
fn a_later_green_rerun_does_not_erase_a_failure() {
    let h = ci_head(
        &[
            job("failure"),
            ok("failure", 10),
            job("success"),
            ok("success", 20),
        ],
        true,
    );
    let h = h.unwrap();
    assert!(h.failing);
    assert_eq!(h.at, Some(Timestamp(20)));
    for c in ["timed_out", "startup_failure"] {
        assert!(ci_head(&[job(c), ok(c, 1)], true).unwrap().failing, "{c}");
    }
}

#[test]
fn a_lone_ci_ok_failure_is_red_unless_excused() {
    assert!(ci_head(&[job("cancelled"), ok("failure", 1)], true).is_none());
    assert!(ci_head(&[ok("failure", 1)], true).is_none());
    assert!(
        ci_head(&[job("cancelled"), ok("failure", 1)], false)
            .unwrap()
            .failing
    );
    assert!(ci_head(&[ok("failure", 1)], false).unwrap().failing);
    let other = run("ci", "other-app", true, "failure", None);
    assert!(ci_head(&[other, ok("failure", 1)], true).is_none());
}

#[test]
fn only_completed_ci_ok_runs_of_actions_count() {
    assert!(ci_head(&[], false).is_none());
    assert!(
        ci_head(
            &[run("ci-ok", "other-app", true, "failure", Some(1))],
            false
        )
        .is_none()
    );
    assert!(
        ci_head(
            &[run("lint", "github-actions", true, "failure", Some(1))],
            false
        )
        .is_none()
    );
    assert!(
        ci_head(
            &[run("ci-ok", "github-actions", false, "failure", Some(1))],
            false
        )
        .is_none()
    );
    assert!(
        !ci_head(&[job("failure"), ok("success", 1)], false)
            .unwrap()
            .failing
    );
    for c in ["cancelled", "skipped", "neutral"] {
        assert!(ci_head(&[job("failure"), ok(c, 1)], false).is_none(), "{c}");
    }
}

#[test]
fn revert_marker_and_p50() {
    assert_eq!(revert_of("Reverts #4.\n\n<!-- bosun:revert 4 -->"), Some(4));
    assert_eq!(revert_of("bosun:revert 4"), None);
    assert_eq!(revert_of("<!-- bosun:revert x -->"), None);
    assert_eq!(p50(&mut []), None);
    assert_eq!(p50(&mut [3.0, 1.0, 2.0]), Some(2.0));
    assert_eq!(p50(&mut [4.0, 1.0, 2.0, 3.0]), Some(2.5));
}

#[test]
fn owner_rules_need_an_owner_ask() {
    let note = |ask: &str| {
        format!(
            "intro\n<!-- bosun:risk -->\n## Risk: high\n\n- **R-gov**: why\n  - `a.md`\n- **R-gate**: why\n\n{ask}\n<!-- /bosun:risk -->"
        )
    };
    let owner = note("Owner: review the head SHA abc and approve.");
    assert_eq!(owner_rules(&owner), ["R-gov", "R-gate"]);
    assert!(owner_rules(&note("Needed: two independent review verdicts")).is_empty());
    assert!(owner_rules("- **R-gov**: loose bullet\nOwner: review").is_empty());
}
