//! RFC 3339 parsing, display and the BOSUN_NOW override.

use bosun::time::{Timestamp, parse_rfc3339};

#[test]
fn rfc3339_parse_and_display() {
    let t = parse_rfc3339("2026-10-22T12:30:45Z").unwrap();
    assert_eq!(t.to_string(), "2026-10-22T12:30:45Z");
    assert_eq!(parse_rfc3339("1970-01-01T00:00:00Z"), Some(Timestamp(0)));
    // Fraction ignored; offsets applied.
    assert_eq!(parse_rfc3339("2026-10-22T12:30:45.987Z"), Some(t));
    assert_eq!(parse_rfc3339("2026-10-22T14:30:45+02:00"), Some(t));
    assert_eq!(parse_rfc3339("2026-10-22T07:00:45-05:30"), Some(t));
    // Leap years: 2024 and 2000 have Feb 29; 1900 and 2100 do not.
    for s in [
        "2024-02-29T00:00:00Z",
        "2000-02-29T23:59:59Z",
        "2028-12-31T23:59:59Z",
        "1999-03-01T00:00:00Z",
    ] {
        assert_eq!(parse_rfc3339(s).unwrap().to_string(), s);
    }
    let secs = |s: &str| parse_rfc3339(s).unwrap().0;
    assert_eq!(
        secs("2024-03-01T00:00:00Z") - secs("2024-02-28T00:00:00Z"),
        2 * 86_400
    );
    assert_eq!(
        secs("2023-03-01T00:00:00Z") - secs("2023-02-28T00:00:00Z"),
        86_400
    );
    assert_eq!(parse_rfc3339("2023-02-29T00:00:00Z"), None);
    assert_eq!(parse_rfc3339("1900-02-29T00:00:00Z"), None);
    assert_eq!(parse_rfc3339("2100-02-29T00:00:00Z"), None);
    for bad in [
        "",
        "2026-10-22",
        "yesterday",
        "2026-13-01T00:00:00Z",
        "2026-10-22T25:00:00Z",
        "2026-10-22T10:00:00+0200",
    ] {
        assert_eq!(parse_rfc3339(bad), None, "{bad}");
    }
}

#[test]
fn day_start_and_plus_secs() {
    let t = parse_rfc3339("2026-10-22T23:59:59Z").unwrap();
    assert_eq!(t.day_start().to_string(), "2026-10-22T00:00:00Z");
    assert_eq!(t.day_start().day_start(), t.day_start());
    assert_eq!(t.plus_secs(1).to_string(), "2026-10-23T00:00:00Z");
    assert_eq!(
        t.plus_secs(1).day_start().to_string(),
        "2026-10-23T00:00:00Z"
    );
    let before_epoch = Timestamp(-1);
    assert_eq!(before_epoch.to_string(), "1969-12-31T23:59:59Z");
    assert_eq!(before_epoch.day_start().to_string(), "1969-12-31T00:00:00Z");
}

#[test]
fn bosun_now_override_is_validated() {
    assert_eq!(
        Timestamp::resolve(Some("2026-10-22T00:00:00Z")).unwrap().0,
        1_792_627_200
    );
    assert!(Timestamp::resolve(Some("garbage")).is_err());
    assert!(Timestamp::resolve(None).unwrap().0 > 1_700_000_000);
}
