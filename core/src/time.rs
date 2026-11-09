//! Local-calendar helpers. Storage is in unix seconds; days are local.

use chrono::{Days, Local, NaiveDate, TimeZone};
use std::time::{SystemTime, UNIX_EPOCH};

pub fn now_ts() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

/// Unix time of the first instant of `date` in local time.
///
/// A few zones skip midnight on DST days; then the day starts at the first
/// hour that exists.
pub fn local_midnight(date: NaiveDate) -> i64 {
    for h in 0..3 {
        let naive = date.and_hms_opt(h, 0, 0).expect("valid hour");
        if let Some(dt) = Local.from_local_datetime(&naive).earliest() {
            return dt.timestamp();
        }
    }
    date.and_hms_opt(0, 0, 0).expect("valid").and_utc().timestamp()
}

/// The local calendar date containing `ts`.
pub fn local_date(ts: i64) -> NaiveDate {
    Local
        .timestamp_opt(ts, 0)
        .earliest()
        .map(|d| d.date_naive())
        .unwrap_or_default()
}

pub fn today() -> NaiveDate {
    Local::now().date_naive()
}

/// `[start, end)` of a local day in unix seconds (23/24/25 h long).
pub fn day_range(date: NaiveDate) -> (i64, i64) {
    let next = date.checked_add_days(Days::new(1)).unwrap_or(date);
    (local_midnight(date), local_midnight(next))
}

/// Start of the local day after the one containing `ts`.
pub fn local_day_end(ts: i64) -> i64 {
    day_range(local_date(ts)).1
}

/// "2h 14m", "14m", "45s".
pub fn fmt_duration(secs: i64) -> String {
    let secs = secs.max(0);
    let (h, m) = (secs / 3600, secs % 3600 / 60);
    match (h, m) {
        (0, 0) => format!("{secs}s"),
        (0, m) => format!("{m}m"),
        (h, 0) => format!("{h}h"),
        (h, m) => format!("{h}h {m}m"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn durations_format_as_hours_and_minutes() {
        assert_eq!(fmt_duration(0), "0s");
        assert_eq!(fmt_duration(45), "45s");
        assert_eq!(fmt_duration(60), "1m");
        assert_eq!(fmt_duration(14 * 60 + 59), "14m");
        assert_eq!(fmt_duration(3600), "1h");
        assert_eq!(fmt_duration(2 * 3600 + 14 * 60 + 5), "2h 14m");
        assert_eq!(fmt_duration(-5), "0s");
    }

    #[test]
    fn day_range_is_contiguous_and_contains_its_timestamps() {
        let d = NaiveDate::from_ymd_opt(2026, 3, 29).unwrap();
        let (a, b) = day_range(d);
        let (b2, _) = day_range(d.succ_opt().unwrap());
        assert_eq!(b, b2);
        assert!((23 * 3600..=25 * 3600).contains(&(b - a)));
        assert_eq!(local_date(a), d);
        assert_eq!(local_date(b - 1), d);
        assert_eq!(local_day_end(a), b);
        assert_eq!(local_day_end(b - 1), b);
    }
}
