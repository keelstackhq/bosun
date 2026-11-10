//! Check runs of the gate App: create, update, read back with the App's id (spec §15).

mod common;

use bosun::github::{GateRun, Github, LiveGithub, RunState};
use common::TOKEN_CANARY;
use common::fake::FakeGithub;

fn setup() -> (FakeGithub, LiveGithub) {
    let fake = FakeGithub::start();
    let gh = LiveGithub::with(&fake.url(), TOKEN_CANARY, "acme", "widget", 0);
    (fake, gh)
}

fn run<'a>(state: RunState, title: &'a str, summary: &'a str) -> GateRun<'a> {
    GateRun {
        name: "risk-check",
        head_sha: "sha1",
        state,
        title,
        summary,
    }
}

#[test]
fn create_posts_an_in_progress_run_and_returns_its_id() {
    let (fake, gh) = setup();
    let id = gh
        .create_check_run(&run(RunState::Pending, "judging", "risk rules of main"))
        .unwrap();
    let posted = fake.check_runs_of("sha1");
    assert_eq!(posted.len(), 1);
    let r = &posted[0];
    assert_eq!(r["id"], id);
    assert_eq!(r["name"], "risk-check");
    assert_eq!(r["head_sha"], "sha1");
    assert_eq!(r["status"], "in_progress");
    assert!(r["conclusion"].is_null());
    assert_eq!(r["output"]["title"], "judging");
    assert_eq!(r["output"]["summary"], "risk rules of main");
    let req = fake
        .requests()
        .into_iter()
        .find(|l| l.method == "POST")
        .unwrap();
    assert_eq!(req.path, "/repos/acme/widget/check-runs");
}

#[test]
fn success_and_failure_complete_the_run() {
    let (fake, gh) = setup();
    let ok = gh
        .create_check_run(&run(RunState::Success, "low: R-docs", "s"))
        .unwrap();
    let bad = gh
        .create_check_run(&run(RunState::Failure, "error: no base", "s"))
        .unwrap();
    assert_ne!(ok, bad);
    let posted = fake.check_runs_of("sha1");
    assert_eq!(
        (
            posted[0]["status"].as_str(),
            posted[0]["conclusion"].as_str()
        ),
        (Some("completed"), Some("success"))
    );
    assert_eq!(
        (
            posted[1]["status"].as_str(),
            posted[1]["conclusion"].as_str()
        ),
        (Some("completed"), Some("failure"))
    );
}

#[test]
fn update_patches_the_same_run_to_its_verdict() {
    let (fake, gh) = setup();
    let id = gh
        .create_check_run(&run(RunState::Pending, "judging", "s"))
        .unwrap();
    gh.update_check_run(id, &run(RunState::Failure, "high: needs the owner", "R-ci"))
        .unwrap();
    let posted = fake.check_runs_of("sha1");
    assert_eq!(posted.len(), 1, "an update never adds a run");
    assert_eq!(posted[0]["status"], "completed");
    assert_eq!(posted[0]["conclusion"], "failure");
    assert_eq!(posted[0]["output"]["title"], "high: needs the owner");
    assert_eq!(posted[0]["output"]["summary"], "R-ci");
    let req = fake
        .requests()
        .into_iter()
        .find(|l| l.method == "PATCH")
        .unwrap();
    assert_eq!(req.path, format!("/repos/acme/widget/check-runs/{id}"));
    let body: serde_json::Value = serde_json::from_str(req.body.as_deref().unwrap()).unwrap();
    assert!(body.get("head_sha").is_none(), "PATCH takes no head_sha");
}

#[test]
fn update_of_an_unknown_run_is_an_error_with_a_fix() {
    let (_fake, gh) = setup();
    let e = gh
        .update_check_run(999_999, &run(RunState::Success, "t", "s"))
        .unwrap_err();
    assert!(!e.fix.is_empty());
}

#[test]
fn no_conclusion_but_success_or_failure_is_ever_sent() {
    // UC-B2-6: neutral and skipped both satisfy a required check.
    let (fake, gh) = setup();
    for s in [RunState::Pending, RunState::Success, RunState::Failure] {
        gh.create_check_run(&run(s, "t", "s")).unwrap();
    }
    for r in fake.check_runs_of("sha1") {
        let c = r["conclusion"].as_str();
        assert!(
            matches!(c, None | Some("success") | Some("failure")),
            "{c:?}"
        );
    }
}

#[test]
fn a_long_summary_is_cut_to_what_github_accepts() {
    let (fake, gh) = setup();
    let summary = "é".repeat(70_000);
    gh.create_check_run(&run(RunState::Success, "t", &summary))
        .unwrap();
    let r = &fake.check_runs_of("sha1")[0];
    assert_eq!(
        r["output"]["summary"].as_str().unwrap().chars().count(),
        65_535
    );
}

#[test]
fn runs_carry_the_app_id_and_slug_of_their_creator() {
    let (fake, gh) = setup();
    fake.check_runs_as_app("keelstackhq-bosun-gate", 4242);
    gh.create_check_run(&run(RunState::Success, "t", "s"))
        .unwrap();
    fake.check_run("sha1", "risk-check", "completed", Some("success"));
    let runs = gh.check_runs_all("sha1").unwrap();
    assert_eq!(runs.len(), 2);
    let app: Vec<(&str, u64)> = runs.iter().map(|r| (r.app.as_str(), r.app_id)).collect();
    assert!(app.contains(&("keelstackhq-bosun-gate", 4242)), "{app:?}");
    assert!(app.contains(&("github-actions", 15368)), "{app:?}");
}

#[test]
fn check_runs_all_reads_every_run_of_every_page() {
    // GitHub's default filter (latest) hides superseded runs; readers of a gate need them all.
    let (fake, gh) = setup();
    for c in ["failure", "success", "failure"] {
        fake.check_run("sha1", "risk-check", "completed", Some(c));
    }
    gh.create_check_run(&run(RunState::Pending, "t", "s"))
        .unwrap();
    let all = gh.check_runs_all("sha1").unwrap();
    assert_eq!(all.len(), 4);
    assert_eq!(gh.check_runs("sha1").unwrap().len(), 2, "latest per app");
    assert!(gh.check_runs_all("nosuchsha").unwrap().is_empty());
}
