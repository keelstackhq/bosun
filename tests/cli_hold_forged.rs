//! `bosun hold` needs the session's own lease in a repo writer's record.
mod common;

use common::fake::FakeGithub;

const PLAN: &str = "# Plan: Ship it\n\nproject: x\n\n## M1: One\n\narea: core\ndepends: seed\n\nWords.\n\n### Tasks\n- T1: a\n  touch: docs/a.md\n### Done when\n- UC-M1-1: works\n";

#[test]
fn a_readers_forged_lease_does_not_pass_require_lease() {
    let (tmp, fake) = (tempfile::tempdir().unwrap(), FakeGithub::start());
    fake.issue(1, "bosun registry", &["bosun:registry"], "");
    let epic = r#"<!-- bosun:epic {"item":"M1","plan":"plans/x.md"} -->"#;
    fake.issue(10, "Epic", &["bosun:state/ready"], epic);
    let body = r#"<!-- bosun:task {"item":"M1","task":"T1","epic":10,"touch":["src/T1/**"]} -->"#;
    fake.issue(11, "Task T1", &["bosun:state/ready"], body);
    fake.content("plans/x.md", "main", PLAN);
    fake.permission("mallory", "read");
    fake.comment_by(
        11,
        "mallory",
        &common::lease_note("session_a", "2026-10-27T11:00:00Z", "docs/a.md"),
    );
    let (code, v) = common::run(
        &fake,
        "session_a",
        "2026-10-27T10:00:00Z",
        &[
            "-C",
            tmp.path().to_str().unwrap(),
            "hold",
            "11",
            "Create the token",
            "--phone",
        ],
    );
    assert_ne!(code, 0, "{v}");
    assert!(
        !fake
            .comments_of(11)
            .iter()
            .any(|c| c.body.contains(r#""bosun":"hold""#)),
        "{v}"
    );
}
