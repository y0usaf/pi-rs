//! Wall-clock mechanism the spec gets from JS `Date`: `toISOString()`
//! formatting and `new Date(string).getTime()` parsing, written once.
//!
//! Only the ISO-8601 subset that session files actually contain is parsed
//! (pi writes `toISOString()` output exclusively); a malformed string maps
//! to `None` where JS produces `NaN`.

/// Days-from-civil (Howard Hinnant's algorithm). `m` is 1-12.
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = y - i64::from(m <= 2);
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// Civil-from-days: epoch day count → (year, month, day).
fn civil_from_days(z: i64) -> (i64, i64, i64) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (y + i64::from(m <= 2), m, d)
}

/// JS `new Date(ms).toISOString()` — `YYYY-MM-DDTHH:MM:SS.mmmZ`.
pub fn iso_from_ms(ms: i64) -> String {
    let days = ms.div_euclid(86_400_000);
    let rem = ms.rem_euclid(86_400_000);
    let (y, mo, d) = civil_from_days(days);
    let h = rem / 3_600_000;
    let mi = rem % 3_600_000 / 60_000;
    let s = rem % 60_000 / 1000;
    let milli = rem % 1000;
    format!("{y:04}-{mo:02}-{d:02}T{h:02}:{mi:02}:{s:02}.{milli:03}Z")
}

/// JS `new Date().toISOString()`.
pub fn now_iso() -> String {
    iso_from_ms(now_ms())
}

/// JS `Date.now()`.
pub fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// [`std::time::SystemTime`] → epoch ms (JS `Date` value of an mtime).
pub fn system_time_ms(t: std::time::SystemTime) -> i64 {
    t.duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// JS `new Date(iso).getTime()` for the `toISOString` subset:
/// `YYYY-MM-DD[THH:MM[:SS[.fff…]]][Z|±hh[:mm]]`. A date without an offset
/// is read as UTC. Returns `None` where JS yields `NaN`.
pub fn parse_iso_ms(s: &str) -> Option<i64> {
    let b = s.as_bytes();
    let digits = |from: usize, len: usize| -> Option<i64> {
        let end = from.checked_add(len)?;
        let part = b.get(from..end)?;
        if !part.iter().all(u8::is_ascii_digit) {
            return None;
        }
        std::str::from_utf8(part).ok()?.parse().ok()
    };
    let year = digits(0, 4)?;
    if b.get(4) != Some(&b'-') {
        return None;
    }
    let month = digits(5, 2)?;
    if b.get(7) != Some(&b'-') {
        return None;
    }
    let day = digits(8, 2)?;
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }

    let mut ms = days_from_civil(year, month, day) * 86_400_000;
    let mut i = 10;
    if b.get(i) == Some(&b'T') {
        let hour = digits(i + 1, 2)?;
        if b.get(i + 3) != Some(&b':') {
            return None;
        }
        let minute = digits(i + 4, 2)?;
        if hour > 23 || minute > 59 {
            return None;
        }
        ms += hour * 3_600_000 + minute * 60_000;
        i += 6;
        if b.get(i) == Some(&b':') {
            let second = digits(i + 1, 2)?;
            if second > 59 {
                return None;
            }
            ms += second * 1000;
            i += 3;
            if b.get(i) == Some(&b'.') {
                i += 1;
                let start = i;
                while b.get(i).is_some_and(u8::is_ascii_digit) {
                    i += 1;
                }
                if i == start {
                    return None;
                }
                let frac = &s[start..i.min(start + 3)];
                let mut frac_ms: i64 = frac.parse().ok()?;
                for _ in frac.len()..3 {
                    frac_ms *= 10;
                }
                ms += frac_ms;
            }
        }
    }
    match b.get(i) {
        None => Some(ms),
        Some(&b'Z') if i + 1 == b.len() => Some(ms),
        Some(&sign @ (b'+' | b'-')) => {
            let hours = digits(i + 1, 2)?;
            let minutes = match b.get(i + 3) {
                Some(&b':') => {
                    if i + 6 != b.len() {
                        return None;
                    }
                    digits(i + 4, 2)?
                }
                None => 0,
                Some(_) => return None,
            };
            let offset = hours * 3_600_000 + minutes * 60_000;
            Some(if sign == b'+' {
                ms - offset
            } else {
                ms + offset
            })
        }
        Some(_) => None,
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;

    #[test]
    fn iso_round_trip() {
        for ms in [0, 1_735_689_600_000, 1_751_990_400_123] {
            assert_eq!(parse_iso_ms(&iso_from_ms(ms)).unwrap(), ms);
        }
        assert_eq!(iso_from_ms(0), "1970-01-01T00:00:00.000Z");
    }

    #[test]
    fn parse_variants() {
        assert_eq!(
            parse_iso_ms("2025-01-01T00:00:00Z"),
            Some(1_735_689_600_000)
        );
        assert_eq!(parse_iso_ms("2025-01-01"), Some(1_735_689_600_000));
        assert_eq!(
            parse_iso_ms("2025-01-01T01:00:00+01:00"),
            Some(1_735_689_600_000)
        );
        assert_eq!(parse_iso_ms("not a date"), None);
        assert_eq!(parse_iso_ms(""), None);
    }
}
