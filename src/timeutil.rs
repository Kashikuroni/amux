//! Dependency-free time helpers: age humanizer + local HH:MM via `date`.
use std::process::Command;

/// Human age from a duration in seconds: "0s","45s","3m","2h","5d".
pub fn humanize_age(secs: i64) -> String {
    let s = secs.max(0);
    if s < 60 {
        format!("{s}s")
    } else if s < 3600 {
        format!("{}m", s / 60)
    } else if s < 86_400 {
        format!("{}h", s / 3600)
    } else {
        format!("{}d", s / 86_400)
    }
}

/// Current local time as "HH:MM" via `date`. Empty string if it fails.
/// NOTE: forks a process — callers should cache (Task 11 wires this in main loop).
pub fn clock_hhmm() -> String {
    Command::new("date")
        .args(["+%H:%M"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_default()
}

/// Unix seconds now (for computing session age). 0 on failure.
pub fn now_unix() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// Local "HH:MM" for a Unix timestamp via BSD `date -r` (macOS). Empty on
/// failure. Forks a process — call off the render path (e.g. when polling).
pub fn local_hhmm(unix: i64) -> String {
    Command::new("date")
        .args(["-r", &unix.to_string(), "+%H:%M"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_default()
}

/// Parses an ISO-8601 instant like `2026-06-02T10:40:01.229996+00:00`,
/// `…Z`, or `…-05:00` into Unix seconds (UTC). `None` on malformed input.
/// Dependency-free: fractional seconds are ignored and the offset is applied.
pub fn parse_iso8601(s: &str) -> Option<i64> {
    let (date, time) = s.split_once('T')?;
    let mut d = date.split('-');
    let y: i64 = d.next()?.parse().ok()?;
    let mo: i64 = d.next()?.parse().ok()?;
    let da: i64 = d.next()?.parse().ok()?;
    // Time core is always HH:MM:SS at fixed positions; the rest is an optional
    // fractional part and/or a zone designator.
    let hh: i64 = time.get(0..2)?.parse().ok()?;
    let mm: i64 = time.get(3..5)?.parse().ok()?;
    let ss: i64 = time.get(6..8)?.parse().ok()?;
    let offset = parse_zone_offset(time.get(8..).unwrap_or(""));
    Some(days_from_civil(y, mo, da) * 86_400 + hh * 3600 + mm * 60 + ss - offset)
}

/// Seconds east of UTC encoded in a trailing zone designator (`Z`, `+HH:MM`,
/// `-HH:MM`), skipping any leading fractional-seconds part. 0 if absent.
fn parse_zone_offset(tail: &str) -> i64 {
    for (i, c) in tail.char_indices() {
        match c {
            'Z' | 'z' => return 0,
            '+' | '-' => {
                let sign = if c == '-' { -1 } else { 1 };
                let rest = &tail[i + 1..];
                let h: i64 = rest.get(0..2).and_then(|x| x.parse().ok()).unwrap_or(0);
                let m: i64 = rest.get(3..5).and_then(|x| x.parse().ok()).unwrap_or(0);
                return sign * (h * 3600 + m * 60);
            }
            _ => {}
        }
    }
    0
}

/// Days since 1970-01-01 for a proleptic-Gregorian date (Howard Hinnant's
/// `days_from_civil`). Valid for any reasonable year.
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400; // [0, 399]
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1; // [0, 365]
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy; // [0, 146096]
    era * 146_097 + doe - 719_468
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn humanize_age_buckets() {
        assert_eq!(humanize_age(0), "0s");
        assert_eq!(humanize_age(45), "45s");
        assert_eq!(humanize_age(60), "1m");
        assert_eq!(humanize_age(185), "3m");
        assert_eq!(humanize_age(3600), "1h");
        assert_eq!(humanize_age(90_000), "1d");
        assert_eq!(humanize_age(-5), "0s");
    }

    #[test]
    fn parse_iso8601_anchors_and_offsets() {
        assert_eq!(parse_iso8601("1970-01-01T00:00:00Z"), Some(0));
        assert_eq!(parse_iso8601("2021-01-01T00:00:00+00:00"), Some(1_609_459_200));
        // Same instant expressed in a +03:00 zone.
        assert_eq!(parse_iso8601("2021-01-01T03:00:00+03:00"), Some(1_609_459_200));
        // …and a -05:00 zone.
        assert_eq!(parse_iso8601("2020-12-31T19:00:00-05:00"), Some(1_609_459_200));
        // Fractional seconds + offset parse the same as the plain UTC form.
        assert_eq!(
            parse_iso8601("2026-06-02T10:40:01.229996+00:00"),
            parse_iso8601("2026-06-02T10:40:01Z"),
        );
        assert_eq!(parse_iso8601("garbage"), None);
    }
}
