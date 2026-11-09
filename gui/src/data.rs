//! Reading the tracker's database (read-only) into a snapshot the views draw.

use std::cell::RefCell;
use std::collections::HashMap;

use chrono::{Days, NaiveDate};
use iced::widget::image;
use iced::Color;
use screentime_core::report::{self, group_small, Catalog, DayReport, Usage};
use screentime_core::store::{self, Store};
use screentime_core::time::{day_range, now_ts, today};
use screentime_core::win_ui;

use crate::palette::{self, Hue, Tone};

/// Apps under this many seconds are folded into "Other".
pub const OTHER_BELOW_SECS: i64 = 60;
pub const OTHER: &str = "Other";

/// Icon pixel size: crisp up to 2x for the 24 px slot.
const ICON_PX: u32 = 64;

#[derive(Debug, Clone, Default)]
pub struct Snapshot {
    pub has_db: bool,
    pub error: Option<String>,
    pub today: NaiveDate,
    /// Seconds since local midnight, for the "now" mark.
    pub now_offset: i64,
    pub day: DayReport,
    /// Yesterday up to the same time of day, for a fair comparison.
    pub yesterday_so_far: i64,
    /// Last 7 days, oldest first; the last one is today.
    pub week: Vec<(NaiveDate, DayReport)>,
    pub week_apps: Vec<Usage>,
    /// Colour of every app on screen, by display name.
    pub tones: HashMap<String, Tone>,
}

impl Snapshot {
    pub fn week_total(&self) -> i64 {
        self.week.iter().map(|d| d.1.total).sum()
    }

    pub fn days_with_data(&self) -> usize {
        self.week.iter().filter(|d| d.1.total > 0).count()
    }

    /// Average over days that have any data (a fresh install is not "0h").
    pub fn week_average(&self) -> i64 {
        self.week_total() / self.days_with_data().max(1) as i64
    }

    pub fn day(&self, date: NaiveDate) -> Option<&DayReport> {
        self.week.iter().find(|d| d.0 == date).map(|d| &d.1)
    }

    pub fn color(&self, name: &str, dark: bool) -> Color {
        match self.tones.get(name) {
            Some(&t) => palette::color(t, dark),
            None => palette::other(dark),
        }
    }
}

/// A list ready to draw: apps of at least a minute, then "Other".
pub struct Rows {
    pub apps: Vec<Usage>,
    pub other: i64,
}

impl Rows {
    pub fn new(usage: &[Usage]) -> Self {
        let (apps, other) = group_small(usage.to_vec(), OTHER_BELOW_SECS);
        Self { apps, other }
    }

    /// The name a slice is drawn under: its app, or "Other".
    pub fn key<'a>(&self, name: &'a str) -> &'a str {
        if self.apps.iter().any(|u| u.name == name) {
            name
        } else {
            OTHER
        }
    }

    pub fn names(&self) -> Vec<String> {
        let mut v: Vec<String> = self.apps.iter().map(|u| u.name.clone()).collect();
        if self.other > 0 {
            v.push(OTHER.into());
        }
        v
    }

    pub fn secs(&self, name: &str) -> i64 {
        if name == OTHER {
            self.other
        } else {
            self.apps.iter().find(|u| u.name == name).map_or(0, |u| u.secs)
        }
    }
}

struct IconEntry {
    handle: Option<image::Handle>,
    hues: Vec<Hue>,
}

pub struct Source {
    store: Option<Store>,
    /// Filled lazily (also while drawing), hence the RefCell.
    icons: RefCell<HashMap<String, IconEntry>>,
}

impl Source {
    pub fn new() -> Self {
        Self { store: None, icons: RefCell::new(HashMap::new()) }
    }

    fn store(&mut self) -> Result<Option<&Store>, String> {
        if self.store.is_none() {
            let path = store::default_db_path();
            self.store = Store::open_read_only(&path).map_err(|e| format!("Cannot open {}: {e}", path.display()))?;
        }
        Ok(self.store.as_ref())
    }

    pub fn load(&mut self) -> Snapshot {
        let today = today();
        let now = now_ts();
        let (t0, _) = day_range(today);
        let mut snap = Snapshot { today, now_offset: now - t0, ..Default::default() };
        let result = (|| -> Result<(), String> {
            let Some(store) = self.store()? else { return Ok(()) };
            snap.has_db = true;
            let e = |e: store::Error| e.to_string();
            let cat = Catalog::load(store).map_err(e)?;
            let yesterday = today.checked_sub_days(Days::new(1)).unwrap_or(today);
            let (y0, _) = day_range(yesterday);
            snap.yesterday_so_far = report::day(store, &cat, yesterday, Some(y0 + (now - t0))).map_err(e)?.total;
            let first = today.checked_sub_days(Days::new(6)).unwrap_or(today);
            for date in first.iter_days().take(7) {
                let until = (date == today).then_some(now + 1);
                snap.week.push((date, report::day(store, &cat, date, until).map_err(e)?));
            }
            snap.day = snap.week.last().map(|d| d.1.clone()).unwrap_or_default();
            snap.week_apps = report::range_usage(store, &cat, first, today).map_err(e)?;
            Ok(())
        })();
        if let Err(e) = result {
            // Drop the connection; the next refresh reopens it.
            self.store = None;
            snap.error = Some(e);
        }
        snap.tones = self.tones(&snap.week_apps);
        snap
    }

    /// One colour per app, stable across Today and Week because it is
    /// assigned over the whole week's list (most used first).
    fn tones(&self, usage: &[Usage]) -> HashMap<String, Tone> {
        let apps: Vec<(String, Vec<Hue>)> = usage.iter().map(|u| (u.name.clone(), self.hues(&u.exe_path))).collect();
        palette::assign(&apps)
    }

    fn with_icon<T>(&self, exe: &str, f: impl FnOnce(&IconEntry) -> T) -> T {
        let mut icons = self.icons.borrow_mut();
        let entry = icons.entry(exe.to_ascii_lowercase()).or_insert_with(|| match win_ui::exe_icon(exe, ICON_PX) {
            Some(i) => IconEntry { hues: palette::icon_hues(&i.pixels), handle: Some(image::Handle::from_rgba(i.width, i.height, i.pixels)) },
            None => IconEntry { handle: None, hues: Vec::new() },
        });
        f(entry)
    }

    /// The icon of an app's executable, extracted once and cached.
    pub fn icon(&self, exe: &str) -> Option<image::Handle> {
        self.with_icon(exe, |e| e.handle.clone())
    }

    fn hues(&self, exe: &str) -> Vec<Hue> {
        self.with_icon(exe, |e| e.hues.clone())
    }
}
