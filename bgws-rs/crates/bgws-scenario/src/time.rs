//! UTC date-times for the scenario start (§7.4: the sun and moon follow
//! from the date, the time and the place). A small, dependency-free RFC 3339
//! reader and writer for whole UTC instants, using the proleptic Gregorian
//! day count (Howard Hinnant's `days_from_civil`). No clocks are read.

use thiserror::Error;

#[derive(Clone, Debug, PartialEq, Eq, Error)]
#[error("`{0}` is not a UTC date-time like 2026-06-01T05:30:00Z")]
pub struct TimeError(pub String);

/// Days from 1970-01-01 to a civil date.
pub fn days_from_civil(y: i64, m: u32, d: u32) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (m as i64 + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d as i64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// The civil date of a day count from 1970-01-01.
pub fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

fn days_in_month(y: i64, m: u32) -> u32 {
    match m {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        _ if (y % 4 == 0 && y % 100 != 0) || y % 400 == 0 => 29,
        _ => 28,
    }
}

/// Parse `YYYY-MM-DDTHH:MM[:SS[.fff]](Z|+00:00|±hh:mm)` into Unix seconds (fractions dropped).
pub fn parse_utc(s: &str) -> Result<i64, TimeError> {
    let err = || TimeError(s.to_owned());
    let b = s.as_bytes();
    if b.len() < 17 || !s.is_ascii() {
        return Err(err());
    }
    let n = |r: std::ops::Range<usize>| -> Result<i64, TimeError> { s.get(r).filter(|t| t.bytes().all(|c| c.is_ascii_digit())).and_then(|t| t.parse().ok()).ok_or_else(err) };
    if b[4] != b'-' || b[7] != b'-' || !(b[10] == b'T' || b[10] == b't' || b[10] == b' ') || b[13] != b':' {
        return Err(err());
    }
    let (y, mo, d, h, mi) = (n(0..4)?, n(5..7)?, n(8..10)?, n(11..13)?, n(14..16)?);
    let mut i = 16;
    let mut sec = 0;
    if b.get(i) == Some(&b':') {
        sec = n(i + 1..i + 3)?;
        i += 3;
        if b.get(i) == Some(&b'.') {
            i += 1;
            while b.get(i).is_some_and(u8::is_ascii_digit) {
                i += 1;
            }
        }
    }
    let offset = match b.get(i) {
        Some(b'Z') | Some(b'z') if i + 1 == b.len() => 0,
        Some(&c) if (c == b'+' || c == b'-') && i + 6 == b.len() && b[i + 3] == b':' => {
            let off = n(i + 1..i + 3)? * 3600 + n(i + 4..i + 6)? * 60;
            if c == b'+' {
                off
            } else {
                -off
            }
        }
        _ => return Err(err()),
    };
    if !(1..=12).contains(&mo) || d < 1 || d > days_in_month(y, mo as u32) as i64 || h > 23 || mi > 59 || sec > 60 {
        return Err(err());
    }
    Ok(days_from_civil(y, mo as u32, d as u32) * 86_400 + h * 3600 + mi * 60 + sec - offset)
}

/// Unix seconds as `YYYY-MM-DDTHH:MM:SSZ`.
pub fn format_utc(t: i64) -> String {
    let (y, m, d) = civil_from_days(t.div_euclid(86_400));
    let s = t.rem_euclid(86_400);
    format!("{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z", s / 3600, (s / 60) % 60, s % 60)
}

/// Local mean solar hour (0–24) at a longitude for a UTC instant.
pub fn local_solar_hour(unix_s: i64, lon_deg: f64) -> f64 {
    let h = unix_s.rem_euclid(86_400) as f64 / 3600.0 + lon_deg / 15.0;
    h.rem_euclid(24.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    #[test]
    fn known_instants() {
        assert_eq!(parse_utc("1970-01-01T00:00:00Z"), Ok(0));
        assert_eq!(parse_utc("2000-03-01T00:00:00Z"), Ok(951_868_800));
        assert_eq!(parse_utc("2026-10-24T00:00:00.000Z"), Ok(1_792_800_000));
        assert_eq!(parse_utc("2026-10-24T02:00+02:00"), Ok(1_792_800_000));
        assert_eq!(format_utc(1_792_800_000), "2026-10-24T00:00:00Z");
        assert!(parse_utc("2026-02-30T00:00:00Z").is_err());
        assert!(parse_utc("yesterday").is_err());
        assert!(parse_utc("2026-10-24T00:00:00").is_err());
        assert!((local_solar_hour(parse_utc("2026-06-01T12:00:00Z").unwrap(), 30.0) - 14.0).abs() < 1e-9);
    }

    proptest! {
        #[test]
        fn round_trips(t in -5_000_000_000i64..10_000_000_000i64) {
            prop_assert_eq!(parse_utc(&format_utc(t)), Ok(t));
        }

        #[test]
        fn never_panics(s in ".{0,40}") {
            let _ = parse_utc(&s);
        }
    }
}
