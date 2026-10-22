//! UTC timestamps and RFC 3339 parsing without a date crate. `BOSUN_NOW` (RFC 3339) overrides
//! "now" (spec D30); tests use it.

use std::fmt;

/// Seconds since 1970-01-01T00:00:00Z.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Timestamp(pub i64);

impl Timestamp {
    /// `now` from an optional `BOSUN_NOW` value, else the system clock.
    pub fn resolve(bosun_now: Option<&str>) -> Result<Timestamp, String> {
        match bosun_now.map(str::trim).filter(|s| !s.is_empty()) {
            Some(v) => parse_rfc3339(v).ok_or_else(|| format!("BOSUN_NOW={v:?} is not RFC 3339")),
            None => Ok(Timestamp(
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_secs() as i64)
                    .unwrap_or(0),
            )),
        }
    }

    /// `BOSUN_NOW` when it parses, else the system clock.
    pub fn now() -> Timestamp {
        let env = std::env::var("BOSUN_NOW").ok();
        Self::resolve(env.as_deref())
            .unwrap_or_else(|_| Self::resolve(None).unwrap_or(Timestamp(0)))
    }

    /// 00:00 UTC of the same day.
    pub fn day_start(self) -> Timestamp {
        Timestamp(self.0.div_euclid(86_400) * 86_400)
    }

    pub fn plus_secs(self, secs: i64) -> Timestamp {
        Timestamp(self.0 + secs)
    }
}

impl fmt::Display for Timestamp {
    /// `YYYY-MM-DDTHH:MM:SSZ`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let (y, m, d) = civil_from_days(self.0.div_euclid(86_400));
        let t = self.0.rem_euclid(86_400);
        write!(
            f,
            "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z",
            t / 3600,
            t % 3600 / 60,
            t % 60
        )
    }
}

/// Days since 1970-01-01 of a proleptic Gregorian date (Howard Hinnant's algorithm).
fn days_from_civil(y: i64, m: u32, d: u32) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y.rem_euclid(400);
    let mp = i64::from((m + 9) % 12);
    let doy = (153 * mp + 2) / 5 + i64::from(d) - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// Inverse of `days_from_civil`: (year, month, day).
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let y = yoe + era * 400 + i64::from(m <= 2);
    (y, m, d)
}

fn num(s: &str, len: usize) -> Option<i64> {
    (s.len() == len && s.bytes().all(|b| b.is_ascii_digit()))
        .then(|| s.parse().ok())
        .flatten()
}

/// Parse an RFC 3339 timestamp (`Z` or `±HH:MM`; a fraction of a second is ignored).
pub fn parse_rfc3339(s: &str) -> Option<Timestamp> {
    let (date, rest) = s.trim().split_once(['T', 't'])?;
    let mut dp = date.split('-');
    let (y, m, d) = (
        num(dp.next()?, 4)?,
        num(dp.next()?, 2)?,
        num(dp.next()?, 2)?,
    );
    if dp.next().is_some() || !(1..=12).contains(&m) || d < 1 {
        return None;
    }
    let leap = y % 4 == 0 && (y % 100 != 0 || y % 400 == 0);
    let dim = [
        31,
        if leap { 29 } else { 28 },
        31,
        30,
        31,
        30,
        31,
        31,
        30,
        31,
        30,
        31,
    ];
    if d > dim[m as usize - 1] {
        return None;
    }
    let (time, zone) = rest.split_at(rest.find(['Z', 'z', '+', '-'])?);
    let mut tp = time.split(':');
    let (h, mi) = (num(tp.next()?, 2)?, num(tp.next()?, 2)?);
    let sec = num(tp.next()?.split('.').next()?, 2)?;
    if tp.next().is_some() || h > 23 || mi > 59 || sec > 60 {
        return None;
    }
    let offset = match zone {
        "Z" | "z" => 0,
        z => {
            let sign = if z.starts_with('-') { -1 } else { 1 };
            let (oh, om) = z[1..].split_once(':')?;
            let (oh, om) = (num(oh, 2)?, num(om, 2)?);
            if oh > 23 || om > 59 {
                return None;
            }
            sign * (oh * 3600 + om * 60)
        }
    };
    let days = days_from_civil(y, m as u32, d as u32);
    Some(Timestamp(days * 86_400 + h * 3600 + mi * 60 + sec - offset))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn civil_days_known_values() {
        assert_eq!(days_from_civil(1970, 1, 1), 0);
        assert_eq!(days_from_civil(2000, 3, 1), 11_017);
        assert_eq!(days_from_civil(2026, 10, 16), 20_742);
        assert_eq!(civil_from_days(20_742), (2026, 10, 16));
        assert_eq!(civil_from_days(-1), (1969, 12, 31));
    }

    #[test]
    fn civil_roundtrip_across_leap_days() {
        for z in (-800_000..800_000).step_by(37) {
            let (y, m, d) = civil_from_days(z);
            assert_eq!(days_from_civil(y, m, d), z);
        }
    }
}
