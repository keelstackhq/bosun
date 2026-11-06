//! `ci-minutes` runs in CI and in the local pre-push loop, with the gates that run no PR code.

fn read(p: &str) -> String {
    std::fs::read_to_string(format!("{}/{p}", env!("CARGO_MANIFEST_DIR"))).unwrap()
}

#[test]
fn bosun_ci_runs_ci_minutes_before_any_pr_code() {
    let ci = read(".github/workflows/ci.yml");
    assert!(
        ci.contains("for g in ${ONLY:-test-integrity ci-minutes deps fmt lint test}; do"),
        "ci.yml gate loop"
    );
}

#[test]
fn the_channel_runs_ci_minutes_and_skips_it_on_an_older_pin() {
    let wf = read("channel/pieces/core/files/.github/workflows/bosun.yml.jinja");
    assert!(wf.contains("GATES: test-integrity ci-minutes deps fmt lint test"));
    assert!(
        wf.contains(
            "'test-integrity ci-minutes deps fmt lint test' || 'test-integrity ci-minutes'"
        )
    );
    // ts-pnpm: it runs no PR code, so it stays in the trusted job.
    assert!(wf.contains("case \"$g\" in test-integrity|ci-minutes|deps) ;;"));
    // A consumer pinned below 0.7.0 has no such gate: skipped with a notice, not failed.
    assert!(wf.contains("has no ci-minutes gate; skipped"));
}

#[test]
fn the_local_loop_runs_ci_minutes() {
    let line = "`for g in fmt lint test deps test-integrity ci-minutes; do bosun gate \"$g\" || break; done`";
    assert!(read("AGENTS.md").contains(line), "AGENTS.md");
    assert!(
        read("channel/pieces/core/files/sections/agents.md").contains(line),
        "channel agents.md"
    );
}
