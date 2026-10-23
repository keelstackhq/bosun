use bosun::core::plan;

#[test]
fn the_repo_plan_parses() {
    let p = plan::parse(include_str!("../plans/bosun.md")).expect("plans/bosun.md is valid");
    let ids: Vec<_> = p.items.iter().map(|i| i.id.as_str()).collect();
    assert_eq!(ids, ["M1", "M2", "M3", "M4", "M5", "M6", "M7", "M8"]);
    let m2 = &p.items[1];
    assert_eq!((m2.tasks.len(), m2.ucs.len()), (4, 4));
    assert_eq!(m2.tasks[1].depends, ["M2/T1"]);
}

#[test]
fn a_broken_plan_reports_every_error_with_its_line() {
    let text = "## m1: Bad\n";
    let e = plan::parse(text).unwrap_err();
    let lines: Vec<_> = e.iter().map(|e| e.line).collect();
    assert_eq!(lines, [1, 1]);
    let text = "# P\n## M1: Bad\n### Tasks\n- T1: a\n  size: 999\n### Done when\n- UC-M1-2: x\n";
    let e = plan::parse(text).unwrap_err();
    let lines: Vec<_> = e.iter().map(|e| e.line).collect();
    assert_eq!(lines, [2, 2, 4, 5, 7]);
    assert!(e.iter().all(|e| !e.fix.is_empty()));
}
