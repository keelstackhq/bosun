mod common;

use common::bosun;
use common::fake::FakeGithub;
use serde_json::{Value, json};
use std::path::Path;

const T0: &str = "2026-10-22T10:00:00Z";
const LESSONS: &str = r#"
[[lesson]]
id = "L1"
rule = "global rule"
evidence = ["e"]
added = "2026-10-01"

[[lesson]]
id = "L2"
rule = "core rule"
scope = ["src/core/**"]
evidence = ["e"]
added = "2026-10-01"

[[lesson]]
id = "L3"
rule = "docs rule"
scope = ["docs/**"]
evidence = ["e"]
added = "2026-10-01"
"#;

fn claim(fake: &FakeGithub, dir: &Path, args: &[&str]) -> Value {
    let out = bosun()
        .env("BOSUN_GITHUB_API", fake.url())
        .env("BOSUN_SESSION", "session_a")
        .env("BOSUN_NOW", T0)
        .arg("-C")
        .arg(dir)
        .arg("claim")
        .args(args)
        .output()
        .unwrap();
    let v: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["ok"], true, "{v}");
    v
}

fn ready(fake: &FakeGithub) {
    fake.issue(
        10,
        "task",
        &["bosun:state/ready"],
        "Do it.\n\n<!-- bosun:task {\"item\":\"M1\",\"task\":\"T1\",\"touch\":[\"src/core/a.rs\"],\"kind\":\"feature\"} -->",
    );
}

fn lines(v: &Value) -> String {
    v["data"]["lessons"].to_string()
}

#[test]
fn claim_lists_global_and_matching_lessons_on_every_path() {
    let (fake, tmp) = (FakeGithub::start(), tempfile::tempdir().unwrap());
    std::fs::create_dir(tmp.path().join("docs")).unwrap();
    std::fs::write(tmp.path().join("docs/lessons.toml"), LESSONS).unwrap();
    ready(&fake);
    let v = claim(&fake, tmp.path(), &["10"]);
    let s = lines(&v);
    assert!(s.contains("global rule") && s.contains("core rule"), "{s}");
    assert!(!s.contains("docs rule"), "{s}");
    // Idempotent re-claim.
    assert_eq!(lines(&claim(&fake, tmp.path(), &["10"])), s);
    // Grow adds the docs scope.
    let v = claim(&fake, tmp.path(), &["10", "--touch", "docs/x.md"]);
    assert!(lines(&v).contains("docs rule"), "{v}");
}

#[test]
fn claim_without_a_lessons_file_has_an_empty_list() {
    let (fake, tmp) = (FakeGithub::start(), tempfile::tempdir().unwrap());
    ready(&fake);
    let v = claim(&fake, tmp.path(), &["10"]);
    assert_eq!(v["data"]["lessons"], json!([]));
}

#[test]
fn claim_with_an_unparseable_lessons_file_still_claims() {
    let (fake, tmp) = (FakeGithub::start(), tempfile::tempdir().unwrap());
    std::fs::create_dir(tmp.path().join("docs")).unwrap();
    std::fs::write(tmp.path().join("docs/lessons.toml"), "[[lesson\n").unwrap();
    ready(&fake);
    let v = claim(&fake, tmp.path(), &["10"]);
    assert_eq!(v["data"]["lessons"], json!([]));
}
