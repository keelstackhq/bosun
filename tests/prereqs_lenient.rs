//! `prereqs::parse_lenient`: sloppy prerequisite bullets get normalized and auto-numbered; the
//! strict `parse` stays strict.
use bosun::core::plan;
use bosun::core::prereqs as pq;

const PLAN: &str = "# Plan: x\n\nproject: x\n\n\
## M1: One\n\narea: core\ndepends: seed\n\nWords.\n\n### Tasks\n\
- T1: a\n  touch: docs/a.md\n- T2: b\n  touch: src/x.rs\n  depends: T1\n\
### Done when\n- UC-M1-1: works\n";

const F: &str = "  on: phone\n  steps: s\n  blocks: M1\n";

fn lenient(section: &str) -> pq::Lenient {
    let t = format!("{PLAN}\n## Prerequisites\n\n{section}");
    let p = plan::parse(&t).unwrap();
    pq::parse_lenient(&t, &p).unwrap()
}

#[test]
fn a_clean_section_is_unchanged() {
    let s = format!("- H1: a\n{F}- H2: b\n{F}");
    let l = lenient(&s);
    assert!(l.fixed.is_empty());
    assert_eq!(l.text, format!("{PLAN}\n## Prerequisites\n\n{s}"));
    assert_eq!(l.prereqs.len(), 2);
}

#[test]
fn bare_and_duplicate_ids_take_the_lowest_unused_number_valid_ones_stay() {
    let s = format!("- H: first\n{F}- H3: second\n{F}- H3: third\n{F}- H1: fourth\n{F}");
    let l = lenient(&s);
    let ids: Vec<&str> = l.prereqs.iter().map(|p| p.id.as_str()).collect();
    assert_eq!(ids, ["H2", "H3", "H4", "H1"]);
    let titles: Vec<&str> = l.prereqs.iter().map(|p| p.title.as_str()).collect();
    assert_eq!(titles, ["first", "second", "third", "fourth"]);
    // Only the bullets that changed are reported, each with its line.
    let rows: Vec<(usize, &str, &str)> = l
        .fixed
        .iter()
        .map(|f| (f.line, f.from.as_str(), f.to.as_str()))
        .collect();
    let base = PLAN.lines().count() + 4;
    assert_eq!(
        rows,
        [
            (base, "- H: first", "- H2: first"),
            (base + 8, "- H3: third", "- H4: third"),
        ]
    );
    assert!(l.text.contains("- H2: first\n"));
    assert!(l.text.contains("- H3: second\n"));
    assert!(l.text.contains("- H4: third\n"));
    assert!(l.text.contains("- H1: fourth\n"));
}

#[test]
fn strict_valid_out_of_order_ids_are_left_alone() {
    let s = format!("- H2: b\n{F}- H1: a\n{F}");
    let l = lenient(&s);
    assert!(l.fixed.is_empty());
    assert_eq!(l.text, format!("{PLAN}\n## Prerequisites\n\n{s}"));
    let ids: Vec<&str> = l.prereqs.iter().map(|p| p.id.as_str()).collect();
    assert_eq!(ids, ["H2", "H1"]);
}

#[test]
fn star_bullets_and_dash_separators_are_normalized() {
    let s = format!("* H1: star\n{F}- H2 - dash\n{F}- H: bare\n{F}");
    let l = lenient(&s);
    assert_eq!(l.prereqs.len(), 3);
    assert!(l.text.contains("\n- H1: star\n"));
    assert!(l.text.contains("\n- H2: dash\n"));
    assert!(l.text.contains("\n- H3: bare\n"));
    assert_eq!(l.fixed.len(), 3);
    assert_eq!(l.fixed[0].from, "* H1: star");
    assert_eq!(l.fixed[0].to, "- H1: star");
}

#[test]
fn only_the_prerequisite_section_changes() {
    let s = format!("- H: a\n{F}");
    let t = format!("{PLAN}\n## Prerequisites\n\n{s}\n## Notes\n\n- H: not a prereq\n");
    let p = plan::parse(&t).unwrap();
    let l = pq::parse_lenient(&t, &p).unwrap();
    assert!(l.text.ends_with("## Notes\n\n- H: not a prereq\n"));
    assert!(l.text.starts_with(PLAN));
}

#[test]
fn a_fenced_example_is_left_alone() {
    let s = format!("```\n- H: example\n```\n- H: real\n{F}");
    let l = lenient(&s);
    assert_eq!(l.prereqs.len(), 1);
    assert!(l.text.contains("```\n- H: example\n```"));
    assert!(l.text.contains("- H1: real\n"));
}

#[test]
fn other_errors_still_fail_with_their_lines() {
    let t = format!("{PLAN}\n## Prerequisites\n\n- H: a\n  steps: s\n  blocks: M1\n");
    let p = plan::parse(&t).unwrap();
    let errs = pq::parse_lenient(&t, &p).unwrap_err();
    assert!(errs[0].message.contains("no `on:`"), "{errs:?}");
}

#[test]
fn strict_parse_still_refuses_the_sloppy_forms() {
    let t = format!("{PLAN}\n## Prerequisites\n\n- H: a\n{F}");
    let p = plan::parse(&t).unwrap();
    assert!(pq::parse(&t, &p).is_err());
    let t = format!("{PLAN}\n## Prerequisites\n\n* H1: a\n{F}");
    assert!(pq::parse(&t, &p).is_err());
}

#[test]
fn only_missing_or_duplicate_ids_count_as_renumbered() {
    assert!(lenient(&format!("- H: a\n{F}")).renumbered);
    assert!(lenient(&format!("- H1: a\n{F}- H1: b\n{F}")).renumbered);
    assert!(!lenient(&format!("* H1 - a\n{F}")).renumbered);
    assert!(!lenient(&format!("- H2: b\n{F}- H1: a\n{F}")).renumbered);
}
