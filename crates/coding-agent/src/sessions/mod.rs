//! Upstream's session files: the JSONL `/export` writes and the JSONL `/import` reads.

pub mod export;
pub mod import;

/// `new Date(ms).toISOString()`: `2026-08-01T16:26:34.264Z`.
pub fn iso8601(ms: u64) -> String {
    let seconds = ms / 1000;
    let (days, of_day) = ((ms / 86_400_000) as i64, seconds % 86_400);
    let (hour, minute, second) = (of_day / 3600, of_day % 3600 / 60, of_day % 60);
    let (year, month, day) = civil_from_days(days);
    format!(
        "{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}.{:03}Z",
        ms % 1000
    )
}

/// The milliseconds of an ISO timestamp, as far as it is one.
pub fn from_iso8601(text: &str) -> Option<u64> {
    let (date, rest) = text.split_once('T')?;
    let (year, month, day) = {
        let mut parts = date.split('-');
        (
            parts.next()?.parse::<i64>().ok()?,
            parts.next()?.parse::<u32>().ok()?,
            parts.next()?.parse::<u32>().ok()?,
        )
    };
    let clock = rest.trim_end_matches('Z');
    let (hms, millis) = match clock.split_once('.') {
        Some((hms, millis)) => (hms, millis.parse::<u64>().ok()?),
        None => (clock, 0),
    };
    let mut parts = hms.split(':');
    let (hour, minute, second) = (
        parts.next()?.parse::<u64>().ok()?,
        parts.next()?.parse::<u64>().ok()?,
        parts.next()?.parse::<u64>().ok()?,
    );
    let days = days_from_civil(year, month, day);
    Some(
        (days * 86_400 + hour as i64 * 3600 + minute as i64 * 60 + second as i64) as u64 * 1000
            + millis,
    )
}

/// Howard Hinnant's `civil_from_days`, for days since 1970-01-01.
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (if m <= 2 { y + 1 } else { y }, m as u32, d as u32)
}

/// The days since 1970-01-01 of a calendar date, Hinnant's `days_from_civil`.
fn days_from_civil(year: i64, month: u32, day: u32) -> i64 {
    let y = if month <= 2 { year - 1 } else { year };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = if month > 2 {
        month as i64 - 3
    } else {
        month as i64 + 9
    };
    let doy = (153 * mp + 2) / 5 + day as i64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}
