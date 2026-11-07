//! Unit tests of the pure pin logic (spec D35).

use bosun::core::pin::*;
#[test]
fn semver_is_strict() {
    assert_eq!(parse_semver("0.4.10"), Some((0, 4, 10)));
    assert!(parse_semver("0.4.1") < parse_semver("0.4.10"));
    for bad in [
        "",
        "1.2",
        "1.2.3.4",
        "v1.2.3",
        "1.2.3-rc.1",
        "1.2.3+b",
        "1..3",
        "a.b.c",
        "-1.2.3",
    ] {
        assert_eq!(parse_semver(bad), None, "{bad}");
    }
}

#[test]
fn integrity_needs_sha512() {
    assert_eq!(sha512_of("sha512-abc=="), Some("abc=="));
    assert_eq!(sha512_of("sha1-abc"), None);
    assert_eq!(sha512_of("sha512-"), None);
}

#[test]
fn set_line_replaces_only_the_exact_key() {
    let t = "A_VERSION=1\nA_VERSION_X=2\nB=3\nA_SHA512=old";
    assert_eq!(
        set_line(t, "A_VERSION", "9").unwrap(),
        "A_VERSION=9\nA_VERSION_X=2\nB=3\nA_SHA512=old"
    );
    assert_eq!(
        set_line("K=1\nK=2\n", "K", "3").unwrap_err(),
        "K= must appear exactly once"
    );
    assert!(set_line("X=1\n", "K", "3").is_err());
    assert_eq!(set_line("K=1\n", "K", "2").unwrap(), "K=2\n");
}

#[test]
fn apply_rewrites_four_lines() {
    let t = "HOIST_VERSION=0.4.0\nHOIST_SHA512=a\nHOIST_LINUX_X64_SHA512=b\nHOIST_LINUX_ARM64_SHA512=c\nz=1\n";
    let p = Pins {
        version: "0.4.1".into(),
        launcher: "L".into(),
        x64: "X".into(),
        arm64: "A".into(),
    };
    assert_eq!(
        apply(t, "hoist", &p).unwrap(),
        "HOIST_VERSION=0.4.1\nHOIST_SHA512=L\nHOIST_LINUX_X64_SHA512=X\nHOIST_LINUX_ARM64_SHA512=A\nz=1\n"
    );
    assert!(apply(t, "splice", &p).is_err());
}
