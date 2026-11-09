//! Aggregation shared by the CLI and the GUI: per-app usage merged by
//! friendly name, hidden plumbing removed, hourly and daily buckets.

use std::collections::HashMap;

use chrono::{Local, NaiveDate, TimeZone, Timelike};

use crate::names;
use crate::store::{AppRow, Result, Span, Store};
use crate::time::day_range;

/// Time spent in one app (all its executables / versions merged).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Usage {
    pub name: String,
    /// The executable with the most time under this name (for its icon).
    pub exe_path: String,
    pub secs: i64,
}

#[derive(Debug, Clone)]
struct AppInfo {
    name: String,
    exe: String,
    hidden: bool,
}

/// Everything needed to turn spans into named, visible usage.
#[derive(Debug, Clone, Default)]
pub struct Catalog {
    apps: HashMap<i64, AppInfo>,
}

impl Catalog {
    pub fn load(store: &Store) -> Result<Self> {
        Ok(Self::from_rows(store.apps()?))
    }

    pub fn from_rows(rows: impl IntoIterator<Item = AppRow>) -> Self {
        let apps = rows
            .into_iter()
            .map(|r| {
                let info = AppInfo {
                    name: names::shown_name(&r.name, &r.exe_path),
                    hidden: names::is_hidden(&r.exe_path),
                    exe: r.exe_path,
                };
                (r.id, info)
            })
            .collect();
        Self { apps }
    }

    fn visible(&self, app_id: i64) -> Option<&AppInfo> {
        self.apps.get(&app_id).filter(|a| !a.hidden)
    }

    /// Display name and exe of an app id (hidden or not).
    pub fn describe(&self, app_id: i64) -> Option<(&str, &str)> {
        self.apps.get(&app_id).map(|a| (a.name.as_str(), a.exe.as_str()))
    }

    /// Visible spans only.
    pub fn filter<'a>(&'a self, spans: &'a [Span]) -> impl Iterator<Item = &'a Span> + 'a {
        spans.iter().filter(|s| self.visible(s.app_id).is_some())
    }

    /// Per-app usage, longest first, merged by display name.
    pub fn usage(&self, spans: &[Span]) -> Vec<Usage> {
        // name -> (total, per-exe totals)
        let mut by_name: HashMap<&str, (i64, HashMap<&str, i64>)> = HashMap::new();
        for s in spans {
            if let Some(app) = self.visible(s.app_id) {
                let e = by_name.entry(&app.name).or_default();
                e.0 += s.secs();
                *e.1.entry(&app.exe).or_default() += s.secs();
            }
        }
        let mut out: Vec<Usage> = by_name
            .into_iter()
            .filter(|(_, (secs, _))| *secs > 0)
            .map(|(name, (secs, exes))| {
                let exe = exes.into_iter().max_by_key(|(e, t)| (*t, *e)).map(|(e, _)| e).unwrap_or("");
                Usage { name: name.to_string(), exe_path: exe.to_string(), secs }
            })
            .collect();
        out.sort_by(|a, b| b.secs.cmp(&a.secs).then_with(|| a.name.cmp(&b.name)));
        out
    }

    /// When each app was in use, as offsets from `day_start`, merged by
    /// display name. Neighbouring pieces of the same app closer than
    /// `merge_gap` seconds are joined, so a dial draws a few arcs instead of
    /// hundreds of slivers. Sorted by start.
    pub fn timeline(&self, spans: &[Span], day_start: i64, merge_gap: i64) -> Vec<Slice> {
        let mut pieces: Vec<(&str, i64, i64)> = self
            .filter(spans)
            .filter_map(|s| {
                let name = self.visible(s.app_id)?.name.as_str();
                let (a, b) = ((s.start - day_start).max(0), s.end - day_start);
                (b > a).then_some((name, a, b))
            })
            .collect();
        pieces.sort_by_key(|p| (p.1, p.2));
        let mut out: Vec<Slice> = Vec::new();
        // The last slice of each app, so an app interrupted briefly by
        // another (Alt+Tab and back) still reads as one stretch.
        let mut last: HashMap<&str, usize> = HashMap::new();
        for (name, a, b) in pieces {
            if let Some(&i) = last.get(name) {
                let is_latest = i + 1 == out.len();
                let s = &mut out[i];
                if is_latest && a - s.end <= merge_gap {
                    s.end = s.end.max(b);
                    continue;
                }
            }
            last.insert(name, out.len());
            out.push(Slice { name: name.to_string(), start: a, end: b });
        }
        out
    }

    pub fn total(&self, spans: &[Span]) -> i64 {
        self.filter(spans).map(Span::secs).sum()
    }

    /// Seconds per local hour of the day starting at `day_start`.
    /// `hour_of` maps a timestamp to its local hour (injected for tests).
    pub fn hourly(&self, spans: &[Span], day_start: i64, hour_of: impl Fn(i64) -> u32) -> [i64; 24] {
        let mut hours = [0i64; 24];
        for s in self.filter(spans) {
            let mut t = s.start;
            while t < s.end {
                // Hour boundaries counted from the local midnight, so half-hour
                // zones (IST) bucket correctly.
                let next = day_start + ((t - day_start).div_euclid(3600) + 1) * 3600;
                let seg_end = next.min(s.end);
                let h = hour_of(t).min(23) as usize;
                hours[h] += seg_end - t;
                t = seg_end;
            }
        }
        hours
    }
}

/// A stretch of one app's use within a day, in seconds from local midnight.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Slice {
    pub name: String,
    pub start: i64,
    pub end: i64,
}

/// Split usage into rows of at least `min_secs` and the summed rest ("Other").
pub fn group_small(usage: Vec<Usage>, min_secs: i64) -> (Vec<Usage>, i64) {
    let (big, small): (Vec<_>, Vec<_>) = usage.into_iter().partition(|u| u.secs >= min_secs);
    (big, small.iter().map(|u| u.secs).sum())
}

/// One local day, ready to show.
#[derive(Debug, Clone, Default)]
pub struct DayReport {
    pub total: i64,
    pub apps: Vec<Usage>,
    pub hourly: [i64; 24],
    /// When each app was used, for the day dial.
    pub timeline: Vec<Slice>,
    /// Length of the day in seconds (23 or 25 hours on DST changes).
    pub length: i64,
}

/// Gaps up to this long between pieces of one app are drawn as one arc.
pub const TIMELINE_MERGE_SECS: i64 = 60;

fn local_hour(ts: i64) -> u32 {
    Local.timestamp_opt(ts, 0).earliest().map(|d| d.hour()).unwrap_or(0)
}

/// Usage of a whole local day, optionally only up to `until` (exclusive).
pub fn day(store: &Store, cat: &Catalog, date: NaiveDate, until: Option<i64>) -> Result<DayReport> {
    let (from, mut to) = day_range(date);
    if let Some(u) = until {
        to = to.min(u).max(from);
    }
    let spans = store.spans(from, to)?;
    Ok(DayReport {
        total: cat.total(&spans),
        apps: cat.usage(&spans),
        hourly: cat.hourly(&spans, from, local_hour),
        timeline: cat.timeline(&spans, from, TIMELINE_MERGE_SECS),
        length: day_range(date).1 - from,
    })
}

/// Visible totals for each date.
pub fn day_totals(store: &Store, cat: &Catalog, dates: &[NaiveDate]) -> Result<Vec<i64>> {
    dates
        .iter()
        .map(|d| {
            let (a, b) = day_range(*d);
            store.spans(a, b).map(|s| cat.total(&s))
        })
        .collect()
}

/// Usage over a span of local days `[first, last]`.
pub fn range_usage(store: &Store, cat: &Catalog, first: NaiveDate, last: NaiveDate) -> Result<Vec<Usage>> {
    let (from, _) = day_range(first);
    let (_, to) = day_range(last);
    Ok(cat.usage(&store.spans(from, to)?))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(id: i64, exe: &str, name: &str) -> AppRow {
        AppRow { id, exe_path: exe.into(), name: name.into() }
    }

    fn cat() -> Catalog {
        Catalog::from_rows([
            row(1, r"C:\Program Files\Mozilla Firefox\firefox.exe", "Firefox"),
            row(2, r"C:\Apps\WindowsTerminal_1.0\WindowsTerminal.exe", "Windows Terminal Host"),
            row(3, r"C:\Apps\WindowsTerminal_1.1\WindowsTerminal.exe", "Windows Terminal Host"),
            row(4, r"C:\Windows\System32\ShellHost.exe", "ShellHost"),
        ])
    }

    fn span(app_id: i64, start: i64, end: i64) -> Span {
        Span { app_id, start, end }
    }

    #[test]
    fn usage_merges_versions_hides_plumbing_and_sorts() {
        let spans = [span(1, 0, 100), span(2, 100, 130), span(3, 130, 230), span(4, 230, 900)];
        let u = cat().usage(&spans);
        assert_eq!(
            u.iter().map(|u| (u.name.as_str(), u.secs)).collect::<Vec<_>>(),
            vec![("Terminal", 130), ("Firefox", 100)]
        );
        assert!(u[0].exe_path.contains("1.1"), "icon from the most-used exe");
        assert_eq!(cat().total(&spans), 230);
    }

    #[test]
    fn small_apps_are_grouped() {
        let u = vec![
            Usage { name: "A".into(), exe_path: String::new(), secs: 600 },
            Usage { name: "B".into(), exe_path: String::new(), secs: 59 },
            Usage { name: "C".into(), exe_path: String::new(), secs: 30 },
        ];
        let (big, other) = group_small(u, 60);
        assert_eq!(big.len(), 1);
        assert_eq!(other, 89);
    }

    #[test]
    fn hourly_buckets_split_spans_at_hour_boundaries() {
        // Day starting at 0, half-hour zone irrelevant here: hour = t/3600.
        let spans = [span(1, 3000, 4000), span(4, 0, 50_000), span(2, 7200, 7300)];
        let h = cat().hourly(&spans, 0, |t| (t / 3600) as u32);
        assert_eq!(h[0], 600);
        assert_eq!(h[1], 400);
        assert_eq!(h[2], 100);
        assert_eq!(h.iter().sum::<i64>(), 1100, "hidden app not counted");
    }

    #[test]
    fn timeline_merges_by_name_and_hides_plumbing() {
        // Terminal from two versions back to back, a hidden ShellHost
        // between Firefox pieces, a long gap that starts a new slice.
        let spans = [
            span(1, 100, 200),
            span(4, 200, 230),
            span(1, 230, 300),
            span(2, 300, 400),
            span(3, 400, 500),
            span(1, 2000, 2100),
        ];
        let t = cat().timeline(&spans, 0, 60);
        let got: Vec<_> = t.iter().map(|s| (s.name.as_str(), s.start, s.end)).collect();
        assert_eq!(got, vec![("Firefox", 100, 300), ("Terminal", 300, 500), ("Firefox", 2000, 2100)]);
    }

    #[test]
    fn timeline_is_relative_to_the_day_and_never_merges_across_another_app() {
        // Firefox, then Terminal, then Firefox again within the gap: the
        // Terminal slice sits between them, so they stay separate.
        let spans = [span(1, 1000, 1100), span(2, 1100, 1120), span(1, 1120, 1200)];
        let t = cat().timeline(&spans, 900, 60);
        let got: Vec<_> = t.iter().map(|s| (s.name.as_str(), s.start, s.end)).collect();
        assert_eq!(got, vec![("Firefox", 100, 200), ("Terminal", 200, 220), ("Firefox", 220, 300)]);
        assert_eq!(t.iter().map(|s| s.end - s.start).sum::<i64>(), 200, "no time invented");
    }

    #[test]
    fn hourly_respects_offset_day_start() {
        // Local midnight at 1800 (a +x:30 zone): hours run 1800..5400, ...
        let spans = [span(1, 5000, 5800)];
        let h = cat().hourly(&spans, 1800, |t| ((t - 1800) / 3600) as u32);
        assert_eq!((h[0], h[1]), (400, 400));
    }
}
