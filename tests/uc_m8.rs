//! Use cases of M8 (spec §12): Review mode (b).
//! Written by `bosun plan import`; the build task removes the ignore and makes each test pass.

/// UC-M8-1: with review.mode = "ci", the review-verdict check runs with no session.
#[test]
#[ignore = "UC-M8-1"]
fn uc_m8_1() {
    todo!("UC-M8-1")
}

/// UC-M8-2: a second review covers only the new commits.
#[test]
#[ignore = "UC-M8-2"]
fn uc_m8_2() {
    todo!("UC-M8-2")
}

/// UC-M8-3: on 20 recorded PRs, mode (b) agrees with mode (a) on at least 18.
#[test]
#[ignore = "UC-M8-3"]
fn uc_m8_3() {
    todo!("UC-M8-3")
}
