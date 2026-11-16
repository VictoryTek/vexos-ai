//! Time without a date library: an RFC 3339 reader and the clock.

use std::time::{SystemTime, UNIX_EPOCH};

/// Milliseconds since the Unix epoch, now.
pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
}

/// Days from 1970-01-01 to the given civil date (proleptic Gregorian).
/// Howard Hinnant's algorithm.
fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let y = if month <= 2 { year - 1 } else { year };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (month + 9) % 12;
    let doy = (153 * mp + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

fn digits(text: &str, from: usize, count: usize) -> Option<i64> {
    let part = text.get(from..from + count)?;
    if !part.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    part.parse().ok()
}

/// Reads `YYYY-MM-DDTHH:MM:SS`, optional fraction, then `Z` or `±HH:MM`, to
/// milliseconds since the epoch. Anything else is `None`. Times before 1970
/// are `None` too: a reset time is always in the future.
pub fn parse_rfc3339_ms(text: &str) -> Option<u64> {
    let text = text.trim();
    let year = digits(text, 0, 4)?;
    let bytes = text.as_bytes();
    if bytes.get(4) != Some(&b'-') || bytes.get(7) != Some(&b'-') {
        return None;
    }
    let month = digits(text, 5, 2)?;
    let day = digits(text, 8, 2)?;
    if !matches!(bytes.get(10), Some(b'T' | b't' | b' ')) {
        return None;
    }
    let hour = digits(text, 11, 2)?;
    let minute = digits(text, 14, 2)?;
    let second = digits(text, 17, 2)?;
    if bytes.get(13) != Some(&b':') || bytes.get(16) != Some(&b':') {
        return None;
    }
    if !(1..=12).contains(&month)
        || !(1..=31).contains(&day)
        || hour > 23
        || minute > 59
        || second > 60
    {
        return None;
    }
    let mut at = 19;
    let mut millis = 0i64;
    if bytes.get(at) == Some(&b'.') {
        at += 1;
        let start = at;
        while bytes.get(at).is_some_and(u8::is_ascii_digit) {
            at += 1;
        }
        if at == start {
            return None;
        }
        let fraction: String = text[start..at].chars().take(3).collect();
        millis = fraction.parse::<i64>().ok()? * 10i64.pow(3 - u32::try_from(fraction.len()).ok()?);
    }
    let offset_minutes = match bytes.get(at) {
        Some(b'Z' | b'z') if at + 1 == bytes.len() => 0,
        Some(sign @ (b'+' | b'-')) if at + 6 == bytes.len() && bytes.get(at + 3) == Some(&b':') => {
            let h = digits(text, at + 1, 2)?;
            let m = digits(text, at + 4, 2)?;
            if h > 23 || m > 59 {
                return None;
            }
            let total = h * 60 + m;
            if *sign == b'+' { total } else { -total }
        }
        _ => return None,
    };
    let days = days_from_civil(year, month, day);
    let seconds = days * 86_400 + hour * 3600 + minute * 60 + second - offset_minutes * 60;
    u64::try_from(seconds * 1000 + millis).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn epoch_and_known_dates() {
        assert_eq!(parse_rfc3339_ms("1970-01-01T00:00:00Z"), Some(0));
        assert_eq!(
            parse_rfc3339_ms("2000-03-01T00:00:00Z"),
            Some(951_868_800_000)
        );
        assert_eq!(
            parse_rfc3339_ms("2026-10-06T21:00:00+00:00"),
            Some(1_791_320_400_000)
        );
    }

    #[test]
    fn offsets_and_fractions() {
        assert_eq!(
            parse_rfc3339_ms("2026-10-06T21:00:00.250Z"),
            Some(1_791_320_400_250)
        );
        assert_eq!(
            parse_rfc3339_ms("2026-10-06T16:00:00-05:00"),
            parse_rfc3339_ms("2026-10-06T21:00:00Z")
        );
        assert_eq!(
            parse_rfc3339_ms("2026-10-06T21:00:00.5Z"),
            Some(1_791_320_400_500)
        );
    }

    #[test]
    fn rejects_everything_else() {
        for bad in [
            "",
            "now",
            "2026-10-06",
            "2026-13-06T21:00:00Z",
            "2026-10-06T25:00:00Z",
            "2026-10-06T21:00:00",
            "2026-10-06T21:00:00+0000",
            "2026-10-06T21:00:00Zjunk",
            "2026-10-06T21:00:00.Z",
            "1969-12-31T23:59:59Z",
            "２０２６-10-06T21:00:00Z",
        ] {
            assert_eq!(parse_rfc3339_ms(bad), None, "{bad}");
        }
    }
}
