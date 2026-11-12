//! `core::heal` heal_main helpers: brake cap and the one PR merged since green.
use bosun::core::heal::{capped_main, since_green};
use bosun::core::mode::HealMain;

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
