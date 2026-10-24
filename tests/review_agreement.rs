//! The mode a / mode b agreement harness (`core::agreement`) on fixtures.
use ::bosun::core::agreement::{Agreement, agreement, passes};
use ::bosun::core::verdict::Verdict;
use serde::Deserialize;
use std::path::Path;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Recording {
    pr: u64,
    head_sha: String,
    #[serde(default)]
    since_sha: Option<String>,
    a: Verdict,
    b: Verdict,
}

fn load(dir: &Path, real: bool) -> Vec<Recording> {
    let mut files: Vec<_> = std::fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|e| e == "json"))
        .collect();
    files.sort();
    files
        .iter()
        .map(|p| {
            let text = std::fs::read_to_string(p).unwrap();
            let r: Recording =
                serde_json::from_str(&text).unwrap_or_else(|e| panic!("{}: {e}", p.display()));
            assert!(
                !real || r.since_sha.is_none(),
                "{}: a real recording is a full review",
                p.display()
            );
            for v in [&r.a, &r.b] {
                v.validate()
                    .unwrap_or_else(|e| panic!("{}: {e}", p.display()));
                assert_eq!((v.pr, &v.head_sha), (r.pr, &r.head_sha), "{}", p.display());
            }
            r
        })
        .collect()
}

fn compare(rs: Vec<Recording>) -> Agreement {
    let pairs: Vec<_> = rs.into_iter().map(|r| (r.a, r.b)).collect();
    agreement(&pairs)
}

fn dir(sub: &str) -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/reviews")
        .join(sub)
}

#[test]
fn synthetic_fixtures_parse_and_agree_as_expected() {
    let a = compare(load(&dir("synthetic"), false));
    assert_eq!(a.total, 6);
    assert_eq!(a.agree, 4);
    assert_eq!(a.disagree, vec![103, 105]);
    assert!(!passes(&a));
}

fn agreement_of(total: usize, agree: usize) -> Agreement {
    Agreement {
        total,
        agree,
        disagree: vec![],
    }
}

#[test]
fn passes_at_eighteen_of_twenty_only() {
    assert!(passes(&agreement_of(20, 18)));
    assert!(passes(&agreement_of(20, 20)));
    assert!(!passes(&agreement_of(20, 17)));
    assert!(!passes(&agreement_of(10, 9)));
}

/// Recordings directly in `tests/fixtures/reviews/` must parse; with 20 or more they must
/// also pass. UC-M8-3 stays open until 20 real recorded runs exist and a real `uc_m8_3` test
/// lands with them: this test does not satisfy it, and asserts nothing while there are none.
#[test]
fn recorded_reviews_parse_and_pass_once_twenty_exist() {
    let a = compare(load(&dir(""), true));
    if a.total >= 20 {
        assert!(passes(&a), "disagree on PRs {:?}", a.disagree);
    }
}
