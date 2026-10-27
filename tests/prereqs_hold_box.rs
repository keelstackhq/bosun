//! The hold box of the owner checklist (design F6): its line, its marker and how it reads back.

#[test]
fn a_hold_box_reads_back_with_its_task_and_old_markers_still_parse() {
    use bosun::core::prereqs::{HoldBox, read_issue, split_found, with_found};
    let b = HoldBox::new("X3", "do `this`", 7, false);
    let body = format!(
        "<!-- bosun:prereqs p.md -->\n{}\n- [x] **H1** t <!-- bosun:prereq H1 sum=ab12cd34 state=owner -->\n",
        b.line()
    );
    let t = read_issue(&body);
    assert_eq!(
        (t[0].id.as_str(), t[0].task, t[0].state.as_str()),
        ("X3", Some(7), "owner")
    );
    assert_eq!(
        (t[1].id.as_str(), t[1].task, t[1].ticked),
        ("H1", None, true)
    );
    let (rest, boxes) = split_found(&body);
    assert_eq!(boxes, std::slice::from_ref(&b));
    assert!(!rest.contains("X3"));
    assert_eq!(HoldBox::parse(&b.line()), Some(b.clone()));
    assert!(with_found(&rest, &boxes).contains("### Found during the work\n- [ ] **X3**"));
    let forged = b.line().replace("task=7", "task=8");
    assert_eq!(
        HoldBox::parse(&forged),
        None,
        "the two task numbers must agree"
    );
}
