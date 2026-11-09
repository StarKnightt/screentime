//! Platform-independent tracking state machine.
//!
//! The tracker is fed one [`Sample`] per tick and turns them into
//! [`Session`]s: runs of consecutive seconds spent in one app. It does no I/O,
//! which keeps it unit-testable; the daemon drains it into the store.
//!
//! Idle handling: when no input has arrived for the idle limit, the clock
//! stops *and the idle wait itself is taken back*: every session is cut at the
//! moment of the last input. Sessions already written to disk are kept for a
//! while (`recent`) so they can be corrected too. Time spent with a fullscreen
//! app in front is never taken back.

/// Default idle limit.
pub const DEFAULT_IDLE_LIMIT_SECS: u64 = 5 * 60;

/// Seconds a tick may arrive late and still extend the open session.
/// Timer jitter can skip one wall-clock second; anything longer (sleep,
/// a stalled process) starts a new session rather than inventing time.
const MAX_FILL_SECS: i64 = 1;

/// Written sessions stay correctable this long past the idle limit.
const RECENT_MARGIN_SECS: i64 = 120;

/// One observation of the machine, taken once per second.
#[derive(Debug, Clone, Copy, Default)]
pub struct Sample {
    /// Unix time, seconds.
    pub ts: i64,
    /// Database id of the foreground app, `None` if there is none we count.
    pub app: Option<i64>,
    /// Seconds since the last keyboard/mouse input.
    pub idle_secs: u64,
    /// Foreground window covers its whole monitor (video, game, slides).
    pub fullscreen: bool,
    /// Workstation locked / secure desktop showing.
    pub locked: bool,
}

/// A half-open interval `[start, end)` of unix seconds spent in one app.
/// `end <= start` marks a session emptied by idle subtraction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Session {
    pub app_id: i64,
    pub start: i64,
    pub end: i64,
    /// Row id once written to the database, so later flushes update in place.
    pub row: Option<i64>,
}

impl Session {
    pub fn secs(&self) -> i64 {
        (self.end - self.start).max(0)
    }

    pub fn is_empty(&self) -> bool {
        self.end <= self.start
    }
}

pub struct Tracker {
    open: Option<Session>,
    /// Finished or corrected sessions waiting to be written.
    closed: Vec<Session>,
    /// Written sessions young enough to be cut back by idle subtraction.
    recent: Vec<Session>,
    /// Maps a timestamp to the start of the next local day. Injected so tests
    /// can use a fixed time zone.
    day_end: fn(i64) -> i64,
    idle_limit: u64,
    /// Time up to which counting was backed by a fullscreen app; idle
    /// subtraction never cuts before this.
    keep_until: i64,
}

impl Tracker {
    pub fn new(day_end: fn(i64) -> i64) -> Self {
        Self {
            open: None,
            closed: Vec::new(),
            recent: Vec::new(),
            day_end,
            idle_limit: DEFAULT_IDLE_LIMIT_SECS,
            keep_until: i64::MIN,
        }
    }

    pub fn idle_limit(&self) -> u64 {
        self.idle_limit
    }

    pub fn set_idle_limit(&mut self, secs: u64) {
        self.idle_limit = secs.max(60);
    }

    /// Feed one sample. Each counted sample credits the second `[ts, ts+1)`.
    pub fn tick(&mut self, s: &Sample) {
        let t = s.ts;
        let horizon = t - self.idle_limit as i64 - RECENT_MARGIN_SECS;
        self.recent.retain(|r| r.end > horizon);

        if s.idle_secs >= self.idle_limit && !s.fullscreen {
            self.close();
            // The last input happened during second `t - idle`; keep that one.
            let cutoff = (t - s.idle_secs as i64 + 1).max(self.keep_until);
            self.cut_back(cutoff);
            return;
        }
        let Some(app) = s.app.filter(|_| !s.locked) else {
            self.close();
            return;
        };
        if s.fullscreen {
            self.keep_until = t + 1;
        }
        match &mut self.open {
            // Same app, and this second is a duplicate, the next one, or one
            // jitter-skipped second later: extend.
            Some(cur) if cur.app_id == app && t >= cur.start && t <= cur.end + MAX_FILL_SECS => {
                cur.end = cur.end.max(t + 1);
            }
            _ => {
                self.close();
                self.open = Some(Session { app_id: app, start: t, end: t + 1, row: None });
            }
        }
        self.split_at_midnight();
    }

    /// Cut every known session back to end at `cutoff`. Changed sessions that
    /// were already written are queued for rewriting (or deletion if emptied).
    fn cut_back(&mut self, cutoff: i64) {
        for s in &mut self.closed {
            if s.end > cutoff {
                s.end = cutoff.max(s.start);
            }
        }
        // Never-written sessions that became empty can simply be forgotten.
        self.closed.retain(|s| !(s.is_empty() && s.row.is_none()));
        let (changed, kept): (Vec<_>, Vec<_>) =
            self.recent.drain(..).partition(|s| s.end > cutoff);
        self.recent = kept;
        self.closed.extend(changed.into_iter().map(|s| Session { end: cutoff.max(s.start), ..s }));
    }

    /// Sessions never span local midnight, so per-day sums are plain sums.
    fn split_at_midnight(&mut self) {
        while let Some(cur) = &mut self.open {
            let boundary = (self.day_end)(cur.start);
            if cur.end <= boundary {
                break;
            }
            // The head keeps the row id: it is the part already on disk.
            let head = Session { end: boundary, ..*cur };
            *cur = Session { app_id: cur.app_id, start: boundary, end: cur.end, row: None };
            self.closed.push(head);
        }
    }

    /// End the open session (idle, lock, pause, shutdown).
    pub fn close(&mut self) {
        if let Some(s) = self.open.take() {
            self.closed.push(s);
        }
    }

    pub fn is_open(&self) -> bool {
        self.open.is_some()
    }

    pub fn open_session(&self) -> Option<&Session> {
        self.open.as_ref()
    }

    /// Sessions waiting to be written (may include emptied ones to delete).
    pub fn pending(&self) -> &[Session] {
        &self.closed
    }

    /// Record a successful write. `rows[i]` is the row of `pending()[i]`
    /// (`None` if it was deleted or never stored); `open_row` that of the
    /// open session.
    pub fn written(&mut self, rows: &[Option<i64>], open_row: Option<i64>) {
        let done = std::mem::take(&mut self.closed);
        for (s, row) in done.into_iter().zip(rows) {
            if let Some(row) = row {
                self.recent.push(Session { row: Some(*row), ..s });
            }
        }
        if let (Some(s), Some(row)) = (self.open.as_mut(), open_row) {
            s.row = Some(row);
        }
    }

    /// True if there is anything to write.
    pub fn is_dirty(&self) -> bool {
        !self.closed.is_empty() || self.open.is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DAY: i64 = 86_400;
    /// Midnight UTC + 10:00 on some day, far from any boundary.
    const T0: i64 = 20_000 * DAY + 10 * 3600;
    const LIM: i64 = DEFAULT_IDLE_LIMIT_SECS as i64;

    fn utc_day_end(ts: i64) -> i64 {
        (ts.div_euclid(DAY) + 1) * DAY
    }

    fn at(ts: i64, app: i64) -> Sample {
        Sample { ts, app: Some(app), ..Default::default() }
    }

    /// Everything the tracker knows, written or not, minus empty sessions.
    fn all(t: &Tracker) -> Vec<(i64, i64, i64)> {
        let mut v: Vec<_> = t
            .recent
            .iter()
            .chain(t.pending())
            .chain(t.open_session())
            .filter(|s| !s.is_empty())
            .map(|s| (s.app_id, s.start, s.end))
            .collect();
        v.sort_by_key(|x| (x.1, x.0));
        v
    }

    /// Pretend a flush happened: give every pending/open session a row.
    fn fake_flush(t: &mut Tracker, next_row: &mut i64) {
        let rows: Vec<_> = t
            .pending()
            .iter()
            .map(|s| {
                if s.is_empty() {
                    None
                } else {
                    Some(s.row.unwrap_or_else(|| {
                        *next_row += 1;
                        *next_row
                    }))
                }
            })
            .collect();
        let open = t.open_session().map(|s| {
            s.row.unwrap_or_else(|| {
                *next_row += 1;
                *next_row
            })
        });
        t.written(&rows, open);
    }

    #[test]
    fn consecutive_seconds_merge_into_one_session() {
        let mut t = Tracker::new(utc_day_end);
        for i in 0..60 {
            t.tick(&at(T0 + i, 1));
        }
        assert_eq!(all(&t), vec![(1, T0, T0 + 60)]);
    }

    #[test]
    fn switching_apps_starts_a_new_session() {
        let mut t = Tracker::new(utc_day_end);
        for i in 0..10 {
            t.tick(&at(T0 + i, 1));
        }
        for i in 10..15 {
            t.tick(&at(T0 + i, 2));
        }
        for i in 15..20 {
            t.tick(&at(T0 + i, 1));
        }
        assert_eq!(all(&t), vec![(1, T0, T0 + 10), (2, T0 + 10, T0 + 15), (1, T0 + 15, T0 + 20)]);
    }

    #[test]
    fn jitter_is_absorbed_but_real_gaps_split() {
        let mut t = Tracker::new(utc_day_end);
        t.tick(&at(T0, 1));
        t.tick(&at(T0, 1)); // duplicate second: no double count
        t.tick(&at(T0 + 2, 1)); // one skipped second: filled
        assert_eq!(all(&t), vec![(1, T0, T0 + 3)]);
        // Machine slept for an hour: not counted, new session.
        t.tick(&at(T0 + 3600, 1));
        assert_eq!(all(&t), vec![(1, T0, T0 + 3), (1, T0 + 3600, T0 + 3601)]);
    }

    #[test]
    fn clock_going_backwards_starts_a_new_session() {
        let mut t = Tracker::new(utc_day_end);
        t.tick(&at(T0 + 100, 1));
        t.tick(&at(T0, 1));
        assert_eq!(all(&t), vec![(1, T0, T0 + 1), (1, T0 + 100, T0 + 101)]);
    }

    #[test]
    fn idle_wait_is_subtracted() {
        let mut t = Tracker::new(utc_day_end);
        // 100 s of activity (input every second), then the user walks away.
        for i in 0..100 {
            t.tick(&at(T0 + i, 1));
        }
        for i in 100..100 + LIM + 30 {
            t.tick(&Sample { idle_secs: (i - 99) as u64, ..at(T0 + i, 1) });
        }
        assert!(!t.is_open());
        // Last input during second 99: the session ends right after it.
        assert_eq!(all(&t), vec![(1, T0, T0 + 100)]);
        // Input returns.
        t.tick(&at(T0 + 100 + LIM + 30, 1));
        assert_eq!(all(&t), vec![(1, T0, T0 + 100), (1, T0 + 130 + LIM, T0 + 131 + LIM)]);
    }

    #[test]
    fn idle_subtraction_corrects_sessions_already_written() {
        let mut t = Tracker::new(utc_day_end);
        let mut rows = 0;
        for i in 0..50 {
            t.tick(&at(T0 + i, 1));
        }
        // Idle from second 50. Focus moves to app 2 on its own (a popup) at
        // 120, so app 1's session is closed and written while idle.
        for i in 50..LIM + 60 {
            let app = if i < 120 { 1 } else { 2 };
            t.tick(&Sample { idle_secs: (i - 49) as u64, ..at(T0 + i, app) });
            if i % 30 == 0 {
                fake_flush(&mut t, &mut rows);
            }
        }
        // App 1 is cut back to the last input; app 2's session is all idle,
        // so it is queued with an empty interval for deletion.
        assert_eq!(all(&t), vec![(1, T0, T0 + 50)]);
        let emptied: Vec<_> = t.pending().iter().filter(|s| s.is_empty()).collect();
        assert_eq!(emptied.len(), 1);
        assert_eq!(emptied[0].app_id, 2);
        assert!(emptied[0].row.is_some(), "written row must be deleted, not leaked");
        let fixed = t.pending().iter().find(|s| s.app_id == 1).unwrap();
        assert!(fixed.row.is_some() && fixed.end == T0 + 50);
    }

    #[test]
    fn fullscreen_keeps_counting_and_is_never_subtracted() {
        let mut t = Tracker::new(utc_day_end);
        for i in 0..1000 {
            t.tick(&Sample { idle_secs: i as u64, fullscreen: true, ..at(T0 + i, 7) });
        }
        assert_eq!(all(&t), vec![(7, T0, T0 + 1000)]);
        // The video ends; still no input, no longer fullscreen.
        t.tick(&Sample { idle_secs: 1000, fullscreen: false, ..at(T0 + 1000, 7) });
        assert!(!t.is_open());
        assert_eq!(all(&t), vec![(7, T0, T0 + 1000)]);
    }

    #[test]
    fn idle_limit_is_configurable() {
        let mut t = Tracker::new(utc_day_end);
        t.set_idle_limit(120);
        t.tick(&at(T0, 1));
        t.tick(&Sample { idle_secs: 119, ..at(T0 + 1, 1) });
        assert!(t.is_open());
        t.tick(&Sample { idle_secs: 120, ..at(T0 + 2, 1) });
        assert!(!t.is_open());
        t.set_idle_limit(5);
        assert_eq!(t.idle_limit(), 60, "limits below a minute are clamped");
    }

    #[test]
    fn locked_and_no_app_do_not_count() {
        let mut t = Tracker::new(utc_day_end);
        t.tick(&at(T0, 1));
        t.tick(&Sample { locked: true, ..at(T0 + 1, 1) });
        t.tick(&Sample { app: None, ..at(T0 + 2, 1) });
        assert!(!t.is_open());
        assert_eq!(all(&t), vec![(1, T0, T0 + 1)]);
    }

    #[test]
    fn session_is_split_at_midnight() {
        let mut t = Tracker::new(utc_day_end);
        let midnight = utc_day_end(T0);
        for ts in midnight - 5..midnight + 5 {
            t.tick(&at(ts, 3));
        }
        assert_eq!(all(&t), vec![(3, midnight - 5, midnight), (3, midnight, midnight + 5)]);
    }

    #[test]
    fn last_second_before_midnight_is_not_split() {
        let mut t = Tracker::new(utc_day_end);
        let midnight = utc_day_end(T0);
        t.tick(&at(midnight - 1, 3));
        assert_eq!(all(&t), vec![(3, midnight - 1, midnight)]);
        assert!(t.pending().is_empty());
    }

    #[test]
    fn jitter_fill_across_midnight_still_splits() {
        let mut t = Tracker::new(utc_day_end);
        let midnight = utc_day_end(T0);
        t.tick(&at(midnight - 1, 3));
        t.tick(&at(midnight + 1, 3)); // skipped the midnight second
        assert_eq!(all(&t), vec![(3, midnight - 1, midnight), (3, midnight, midnight + 2)]);
    }

    #[test]
    fn idle_across_midnight_cuts_both_days() {
        let mut t = Tracker::new(utc_day_end);
        let midnight = utc_day_end(T0);
        let mut rows = 0;
        t.tick(&at(midnight - 100, 3));
        for ts in midnight - 99..midnight + LIM {
            let idle = (ts - (midnight - 100)) as u64;
            t.tick(&Sample { idle_secs: idle, ..at(ts, 3) });
            fake_flush(&mut t, &mut rows);
        }
        assert_eq!(all(&t), vec![(3, midnight - 100, midnight - 99)]);
    }

    #[test]
    fn written_moves_sessions_to_recent_with_rows() {
        let mut t = Tracker::new(utc_day_end);
        let midnight = utc_day_end(T0);
        t.tick(&at(midnight - 2, 3));
        t.written(&[], Some(42));
        t.tick(&at(midnight - 1, 3));
        t.tick(&at(midnight, 3));
        assert_eq!(t.pending()[0].row, Some(42), "split head keeps the row");
        assert_eq!(t.open_session().unwrap().row, None);
        t.written(&[Some(42)], Some(43));
        assert!(t.pending().is_empty());
        assert_eq!(t.recent.len(), 1);
    }
}
