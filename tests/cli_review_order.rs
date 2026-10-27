//! An owner's decisive reviews are ordered by `submitted_at`, not by list position (spec §8); a
//! decisive review without a usable time fails closed.
mod common;

use common::fake::FakeGithub;
use common::run;
use serde_json::{Value, json};

const HEAD: &str = "0123456789abcdef0123456789abcdef01234567";
const EARLY: &str = "2026-10-22T09:00:00Z";
const LATE: &str = "2026-10-22T09:30:00Z";

fn high_pr() -> FakeGithub {
    let fake = FakeGithub::start();
    fake.pull(1, "change", "feature", "");
    fake.pull_as_issue(1, "change");
    fake.pull_set(1, "head", json!({"sha": HEAD, "ref": "feature"}));
    fake.content(
        "governance/owners.toml",
        "base0000",
        "[[owner]]\ngithub = \"ada\"\n",
    );
    fake.pull_file(
        1,
        ".github/workflows/ci.yml",
        "modified",
        1,
        0,
        Some("@@ -0,0 +1,1 @@\n+x"),
    );
    fake
}

fn approved_by(fake: &FakeGithub) -> Value {
    let (_, v) = run(
        fake,
        "session_test",
        "2026-10-22T12:00:00Z",
        &["risk-check", "1"],
    );
    v["data"]["approved_by"].clone()
}

/// Reviews listed in this order, each with its own `submitted_at` (`""`: none).
fn listed(reviews: &[(&str, &str)]) -> Value {
    let fake = high_pr();
    for (state, at) in reviews {
        fake.review_at(1, "ada", state, HEAD, Some(at));
    }
    approved_by(&fake)
}

#[test]
fn the_newest_review_by_time_decides_whatever_the_list_order() {
    assert_eq!(
        listed(&[("CHANGES_REQUESTED", LATE), ("APPROVED", EARLY)]),
        Value::Null
    ); // CR is newer, listed first
    assert_eq!(
        listed(&[("APPROVED", LATE), ("CHANGES_REQUESTED", EARLY)]),
        json!("ada")
    ); // approval is newer, listed first
    assert_eq!(
        listed(&[("DISMISSED", LATE), ("APPROVED", EARLY)]),
        Value::Null
    );
    assert_eq!(
        listed(&[("APPROVED", EARLY), ("CHANGES_REQUESTED", LATE)]),
        Value::Null
    ); // list order agrees
    assert_eq!(
        listed(&[("APPROVED", EARLY), ("CHANGES_REQUESTED", EARLY)]),
        Value::Null
    ); // a tie goes to the later id
}

#[test]
fn a_decisive_review_without_a_usable_time_fails_closed() {
    for bad in ["", "yesterday"] {
        assert_eq!(
            listed(&[("APPROVED", LATE), ("CHANGES_REQUESTED", bad)]),
            Value::Null,
            "{bad:?}"
        );
        assert_eq!(
            listed(&[("CHANGES_REQUESTED", bad), ("APPROVED", LATE)]),
            Value::Null,
            "{bad:?}"
        );
        assert_eq!(
            listed(&[("APPROVED", bad), ("CHANGES_REQUESTED", EARLY)]),
            Value::Null,
            "{bad:?}"
        );
    }
}
