//! `.bosun/template-tests` lists tests test-integrity waives: a line added is R-weaken
//! evidence, a line removed is not.

use bosun::core::weaken::{reads_text, text_findings};

const T: &str = ".bosun/template-tests";

#[test]
fn the_file_is_read_at_base_and_head() {
    assert!(reads_text(T));
}

#[test]
fn a_new_glob_is_found() {
    let f = text_findings(T, Some("tests/a/**\n"), Some("tests/a/**\ntests/b/**\n"));
    assert_eq!(f, [format!("{T}: template-tests (none) → tests/b/**")]);
}

#[test]
fn a_new_file_is_found_line_by_line() {
    let f = text_findings(T, None, Some("# note\n\ntests/a/**\ntests/b/**\n"));
    assert_eq!(f.len(), 2, "{f:?}");
}

#[test]
fn a_widened_glob_is_found() {
    let f = text_findings(T, Some("tests/a/x.rs\n"), Some("tests/a/**\n"));
    assert_eq!(f.len(), 1, "{f:?}");
}

#[test]
fn removed_lines_comments_and_reordering_are_not() {
    assert!(text_findings(T, Some("tests/a/**\ntests/b/**\n"), Some("tests/a/**\n")).is_empty());
    assert!(text_findings(T, Some("tests/a/**\n"), Some("# why\ntests/a/**\n")).is_empty());
    assert!(text_findings(T, Some("a\nb\n"), Some("b\na\n")).is_empty());
    assert!(text_findings(T, Some("a\n"), None).is_empty());
}
