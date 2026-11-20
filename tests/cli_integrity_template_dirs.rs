//! test-integrity skips template sources (spec §7): a file under a `template*/pieces/<piece>/files/`
//! or `channel/pieces/<piece>/files/` tree is render input, not a test of this repo, so moving it
//! to `.jinja` or dropping a test there is no finding.

mod common;

use common::{bosun, one_object};
use serde_json::Value;
use std::fs;
use std::path::Path;
use std::process::Command;

fn git(root: &Path, args: &[&str]) {
    let out = Command::new("git")
        .current_dir(root)
        .args(args)
        .output()
        .expect("git");
    assert!(out.status.success(), "git {args:?}: {out:?}");
}

const TWO_TESTS: &str =
    "#[test]\nfn a() {\n    assert!(true);\n}\n\n#[test]\nfn b() {\n    let _ = 1;\n}\n";

fn put(root: &Path, rel: &str, text: &str) {
    let p = root.join(rel);
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    fs::write(p, text).unwrap();
}

/// Base has `.bosun/` (not a bootstrap PR) and the given test file; the branch moves it.
fn run(path: &str, moved: &str) -> (i32, Value) {
    run_with(path, moved, true)
}

fn run_with(path: &str, moved: &str, marker: bool) -> (i32, Value) {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().canonicalize().unwrap();
    git(&root, &["init", "-q", "-b", "main"]);
    git(&root, &["config", "user.name", "t"]);
    git(&root, &["config", "user.email", "t@example.com"]);
    put(&root, ".bosun/bosun.toml", "");
    put(&root, path, TWO_TESTS);
    if marker {
        let top = path.split('/').next().unwrap();
        put(&root, &format!("{top}/splice.toml"), "");
    }
    git(&root, &["add", "."]);
    git(&root, &["commit", "-q", "-m", "base"]);
    git(&root, &["checkout", "-q", "-b", "work"]);
    git(&root, &["mv", path, moved]);
    git(&root, &["commit", "-q", "-m", "move"]);
    let out = bosun()
        .env_remove("GITHUB_BASE_REF")
        .env_remove("BOSUN_PR_BODY")
        .env_remove("BOSUN_INTEGRITY_ALLOW")
        .arg("-C")
        .arg(&root)
        .args(["test-integrity", "--base", "main"])
        .output()
        .unwrap();
    (out.status.code().unwrap(), one_object(&out.stdout))
}

#[test]
fn a_template_test_moved_to_jinja_is_no_finding() {
    let (code, v) = run(
        "template-rust/pieces/core/files/tests/t.rs",
        "template-rust/pieces/core/files/tests/t.rs.jinja",
    );
    assert_eq!(code, 0, "{v}");
}

#[test]
fn a_channel_piece_test_moved_to_jinja_is_no_finding() {
    let (code, v) = run(
        "channel/pieces/rust/files/tests/t.rs",
        "channel/pieces/rust/files/tests/t.rs.jinja",
    );
    assert_eq!(code, 0, "{v}");
}

#[test]
fn a_real_test_moved_to_jinja_is_still_refused() {
    let (code, v) = run("tests/t.rs", "tests/t.rs.jinja");
    assert_eq!(code, 1, "{v}");
}

#[test]
fn a_look_alike_path_without_a_template_marker_is_still_refused() {
    let (code, v) = run_with(
        "template-rust/pieces/core/files/tests/t.rs",
        "template-rust/pieces/core/files/tests/t.rs.jinja",
        false,
    );
    assert_eq!(code, 1, "{v}");
}
