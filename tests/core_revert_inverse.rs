//! `core::risk` revert helpers: the markers and the exact-inverse check (spec §8, R-revert).
use bosun::core::heal::{capped_main, since_green};
use bosun::core::mode::HealMain;
use bosun::core::risk::{PatchFile, inverse_problems, revert_targets};

fn f<'a>(path: &'a str, status: &'a str, patch: Option<&'a str>) -> PatchFile<'a> {
    let changes = patch.map_or(0, |p| p.lines().count() as u64);
    PatchFile {
        path,
        previous_path: None,
        status,
        patch,
        changes,
    }
}

#[test]
fn targets_are_distinct_digits_in_order() {
    let body = "x\n<!-- bosun:revert 5 -->\n<!-- bosun:revert 7 -->\n<!-- bosun:revert 5 -->\n\
                <!-- bosun:revert x1 -->\n<!-- bosun:revert  -->";
    assert_eq!(revert_targets(body), [5, 7]);
    assert!(revert_targets("none").is_empty());
}

#[test]
fn the_exact_inverse_is_ok() {
    let m = f(
        "a",
        "modified",
        Some("@@ -1,2 +1,2 @@ fn x\n ctx\n-old\n+new"),
    );
    let r = f("a", "modified", Some("@@ -1,2 +1,2 @@\n ctx\n-new\n+old"));
    assert!(inverse_problems(&[m], &[r]).is_empty());
}

#[test]
fn the_same_lines_at_another_place_are_a_problem() {
    let m = f("a", "modified", Some("@@ -10,2 +10,3 @@\n ctx\n+x\n ctx"));
    let r = f("a", "modified", Some("@@ -200,3 +200,2 @@\n ctx\n-x\n ctx"));
    assert!(!inverse_problems(&[m], &[r]).is_empty());
}

#[test]
fn a_binary_change_without_a_patch_fails_closed() {
    for (ms, rs) in [
        ("modified", "modified"),
        ("removed", "added"),
        ("added", "removed"),
    ] {
        let (m, r) = (f("x.png", ms, None), f("x.png", rs, None));
        assert!(!inverse_problems(&[m], &[r]).is_empty(), "{ms}");
    }
}

#[test]
fn run_order_of_changed_lines_does_not_matter() {
    let m = f("a", "modified", Some("@@ -1 +1 @@\n-a\n+b\n+c"));
    let r = f("a", "modified", Some("@@ -1 +1 @@\n+a\n-b\n-c"));
    assert!(inverse_problems(&[m], &[r]).is_empty());
}

#[test]
fn a_modified_line_swapped_for_another_is_a_problem() {
    let m = f("a", "modified", Some("@@ -1 +1 @@\n-old\n+new"));
    let r = f("a", "modified", Some("@@ -1 +1 @@\n-new\n+other"));
    let p = inverse_problems(&[m], &[r]);
    assert!(p.iter().any(|s| s.contains('a')), "{p:?}");
}

#[test]
fn added_pairs_with_removed_only() {
    let m = f("n", "added", Some("@@ -0,0 +1,2 @@\n+x\n+y"));
    let r = f("n", "removed", Some("@@ -1,2 +0,0 @@\n-x\n-y"));
    assert!(inverse_problems(&[m], &[r]).is_empty());
    let m = f("n", "added", Some("@@ -0,0 +1 @@\n+x"));
    let r = f("n", "modified", Some("@@ -1 +1 @@\n-x\n+y"));
    assert!(!inverse_problems(&[m], &[r]).is_empty());
}

#[test]
fn a_rename_pairs_through_the_previous_path() {
    let mut m = f("new", "renamed", None);
    m.previous_path = Some("old");
    let mut r = f("old", "renamed", None);
    r.previous_path = Some("new");
    assert!(inverse_problems(&[m], &[r]).is_empty());
    let mut m = f("new", "renamed", None);
    m.previous_path = Some("old");
    let mut r = f("elsewhere", "renamed", None);
    r.previous_path = Some("new");
    assert!(!inverse_problems(&[m], &[r]).is_empty());
}

#[test]
fn an_extra_or_missing_file_is_a_problem() {
    let p = "@@ -1 +1 @@\n-a\n+b";
    let q = "@@ -1 +1 @@\n-b\n+a";
    let (m, r) = (f("a", "modified", Some(p)), f("a", "modified", Some(q)));
    let extra = f("z", "modified", Some(q));
    assert!(!inverse_problems(&[f("a", "modified", Some(p))], &[r, extra]).is_empty());
    assert!(
        !inverse_problems(
            &[m, f("z", "modified", Some(p))],
            &[f("a", "modified", Some(q))]
        )
        .is_empty()
    );
}

#[test]
fn the_wrong_pr_has_a_different_diff() {
    let m = f("a", "modified", Some("@@ -1 +1 @@\n-1\n+2"));
    let r = f("a", "modified", Some("@@ -1 +1 @@\n-3\n+1"));
    assert!(!inverse_problems(&[m], &[r]).is_empty());
}

#[test]
fn a_missing_patch_fails_closed() {
    let m = f("a", "modified", None);
    let r = f("a", "modified", Some("@@ -1 +1 @@\n-b\n+a"));
    let p = inverse_problems(&[m], &[r]);
    assert!(p.iter().any(|s| s.contains("no patch for a")), "{p:?}");
    let mut r = f("a", "modified", None);
    r.changes = 4;
    assert!(!inverse_problems(&[f("a", "modified", None)], &[r]).is_empty());
}

#[test]
fn the_no_newline_marker_belongs_to_the_line_before_it() {
    let m = "@@ -1 +1 @@\n-a\n\\ No newline at end of file\n+a";
    let r = "@@ -1 +1 @@\n-a\n+a\n\\ No newline at end of file";
    assert!(
        inverse_problems(
            &[f("a", "modified", Some(m))],
            &[f("a", "modified", Some(r))]
        )
        .is_empty()
    );
    let r_plain = "@@ -1 +1 @@\n-a\n+a";
    assert!(
        !inverse_problems(
            &[f("a", "modified", Some(m))],
            &[f("a", "modified", Some(r_plain))]
        )
        .is_empty()
    );
}

#[test]
fn problems_are_capped() {
    let files: Vec<String> = (0..20).map(|i| format!("f{i}")).collect();
    let r: Vec<PatchFile> = files
        .iter()
        .map(|n| f(n, "modified", Some("@@ -1 +1 @@\n-a\n+b")))
        .collect();
    assert!(inverse_problems(&[], &r).len() <= 6);
}

#[test]
fn brake_caps_revert_auto_at_revert_pr() {
    assert_eq!(capped_main(HealMain::RevertAuto, true), HealMain::RevertPr);
    assert_eq!(
        capped_main(HealMain::RevertAuto, false),
        HealMain::RevertAuto
    );
    assert_eq!(capped_main(HealMain::Off, true), HealMain::Off);
}

#[test]
fn since_green_names_the_one_bad_pr() {
    assert_eq!(since_green(&[(9, false), (8, true), (7, true)]), Ok(9));
    let e = since_green(&[(9, true)]).unwrap_err();
    assert!(e.contains("newest merge is green"), "{e}");
    let e = since_green(&[(9, false), (8, false), (7, true)]).unwrap_err();
    assert!(e.starts_with("2 PRs merged since green"), "{e}");
    assert!(e.contains("bosun main-triage --good <sha>"), "{e}");
    let e = since_green(&[(9, false), (8, false)]).unwrap_err();
    assert!(e.contains("no green merge in the window"), "{e}");
    assert!(since_green(&[]).is_err());
}
