//! SQLite storage. One file, WAL mode, safe to read while the tracker writes.
//!
//! Schema (user_version 1):
//!   apps(id, exe_path UNIQUE NOCASE, name)
//!   sessions(id, app_id -> apps, start, end)   -- [start, end) unix seconds
//!
//! The tracker splits sessions at local midnight, but queries still clip to
//! the requested range so a reader never depends on that. Aggregation
//! (friendly names, hidden apps, per-hour buckets) lives in [`crate::report`].

use crate::tracker::{Session, Tracker};
use rusqlite::{params, Connection, OpenFlags, OptionalExtension};
use std::path::{Path, PathBuf};

pub use rusqlite::Error;
pub type Result<T> = rusqlite::Result<T>;

/// Longest a single stored session can be (a 25 h DST day, with margin).
/// Lets range queries use the index on `start`.
const MAX_SESSION_SECS: i64 = 2 * 86_400;

const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS apps (
    id       INTEGER PRIMARY KEY,
    exe_path TEXT NOT NULL UNIQUE COLLATE NOCASE,
    name     TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS sessions (
    id     INTEGER PRIMARY KEY,
    app_id INTEGER NOT NULL REFERENCES apps(id),
    start  INTEGER NOT NULL,
    "end"  INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS sessions_start ON sessions(start);
PRAGMA user_version = 1;
"#;

/// Directory for all screentime files: `%LOCALAPPDATA%\screentime`.
///
/// The Store (MSIX) build uses its package's own LocalState folder instead,
/// `%LOCALAPPDATA%\Packages\<family name>\LocalState`. Windows deletes that
/// folder when the Store app is uninstalled, and it never shares data with a
/// GitHub install on the same PC.
pub fn data_dir() -> PathBuf {
    let base = std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    match crate::win::package_family_name() {
        Some(pfn) => base.join("Packages").join(pfn).join("LocalState"),
        None => base.join("screentime"),
    }
}

/// Database path. `SCREENTIME_DB` overrides it (tests, portable use).
pub fn default_db_path() -> PathBuf {
    std::env::var_os("SCREENTIME_DB")
        .map(PathBuf::from)
        .unwrap_or_else(|| data_dir().join("data.db"))
}

/// A row of `apps`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppRow {
    pub id: i64,
    pub exe_path: String,
    pub name: String,
}

/// A stored session, possibly clipped to a query range.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Span {
    pub app_id: i64,
    pub start: i64,
    pub end: i64,
}

impl Span {
    pub fn secs(&self) -> i64 {
        (self.end - self.start).max(0)
    }
}

pub struct Store {
    conn: Connection,
}

impl Store {
    /// Open (creating if needed) the database for writing. Tracker only.
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let conn = Connection::open(path)?;
        conn.busy_timeout(std::time::Duration::from_secs(5))?;
        // WAL: readers never block the tracker and vice versa.
        let _mode: String = conn.query_row("PRAGMA journal_mode=WAL", [], |r| r.get(0))?;
        // FULL fsyncs the WAL on every commit. Commits happen every ~30 s, so
        // the cost is nil, and a power cut loses at most the last batch.
        // (NORMAL would only sync at checkpoints and could lose far more.)
        conn.execute_batch(
            "PRAGMA synchronous=FULL;
             PRAGMA foreign_keys=ON;
             PRAGMA cache_size=-512;",
        )?;
        conn.execute_batch(SCHEMA)?;
        Ok(Self { conn })
    }

    /// Open an existing database read-only; `Ok(None)` if there is none yet.
    /// For readers (CLI, GUI): they can never modify or lock out the tracker.
    pub fn open_read_only(path: &Path) -> Result<Option<Self>> {
        if !path.exists() {
            return Ok(None);
        }
        let flags = OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX;
        let conn = Connection::open_with_flags(path, flags)?;
        conn.busy_timeout(std::time::Duration::from_secs(2))?;
        conn.execute_batch("PRAGMA cache_size=-512;")?;
        Ok(Some(Self { conn }))
    }

    pub fn open_in_memory() -> Result<Self> {
        let conn = Connection::open_in_memory()?;
        conn.execute_batch(SCHEMA)?;
        Ok(Self { conn })
    }

    /// Id for an executable, inserting it on first sight.
    pub fn app_id(&self, exe_path: &str, name: &str) -> Result<i64> {
        self.conn.query_row(
            "INSERT INTO apps(exe_path, name) VALUES (?1, ?2)
             ON CONFLICT(exe_path) DO UPDATE SET name = excluded.name
             RETURNING id",
            params![exe_path, name],
            |r| r.get(0),
        )
    }

    pub fn apps(&self) -> Result<Vec<AppRow>> {
        let mut st = self.conn.prepare_cached("SELECT id, exe_path, name FROM apps ORDER BY id")?;
        let rows = st.query_map([], |r| Ok(AppRow { id: r.get(0)?, exe_path: r.get(1)?, name: r.get(2)? }))?;
        rows.collect()
    }

    pub fn rename_app(&self, id: i64, name: &str) -> Result<()> {
        self.conn.execute("UPDATE apps SET name = ?2 WHERE id = ?1", params![id, name]).map(|_| ())
    }

    /// Write the tracker's pending sessions and its open one in one
    /// transaction. Open sessions are upserted, so a crash loses at most the
    /// time since the previous flush; sessions emptied by idle subtraction
    /// are deleted. Tracker state only changes once the commit succeeded, so
    /// a failed flush is retried next time.
    pub fn flush(&mut self, tr: &mut Tracker) -> Result<()> {
        if !tr.is_dirty() {
            return Ok(());
        }
        let tx = self.conn.transaction()?;
        let rows = tr.pending().iter().map(|s| write_session(&tx, s)).collect::<Result<Vec<_>>>()?;
        let open_row = match tr.open_session() {
            Some(s) => write_session(&tx, s)?,
            None => None,
        };
        tx.commit()?;
        tr.written(&rows, open_row);
        Ok(())
    }

    /// Sessions overlapping `[from, to)`, clipped to it, in start order.
    pub fn spans(&self, from: i64, to: i64) -> Result<Vec<Span>> {
        let mut st = self.conn.prepare_cached(
            r#"SELECT app_id, MAX(start, ?1), MIN("end", ?2) FROM sessions
               WHERE start >= ?1 - ?3 AND start < ?2 AND "end" > ?1
               ORDER BY start"#,
        )?;
        let rows = st.query_map(params![from, to, MAX_SESSION_SECS], |r| {
            Ok(Span { app_id: r.get(0)?, start: r.get(1)?, end: r.get(2)? })
        })?;
        rows.collect()
    }

    /// Every stored session, oldest first (export).
    pub fn all_spans(&self) -> Result<Vec<Span>> {
        self.spans(i64::MIN / 4, i64::MAX / 4)
    }

    /// Unix time of the most recent stored second, if any.
    pub fn last_seen(&self) -> Result<Option<i64>> {
        self.conn
            .query_row(r#"SELECT MAX("end") FROM sessions"#, [], |r| r.get::<_, Option<i64>>(0))
            .optional()
            .map(Option::flatten)
    }
}

/// Insert, update or delete one session. Returns its row, or `None` if the
/// session is (now) empty and so not stored.
fn write_session(tx: &rusqlite::Transaction, s: &Session) -> Result<Option<i64>> {
    match (s.row, s.is_empty()) {
        (Some(id), true) => {
            tx.execute("DELETE FROM sessions WHERE id = ?1", params![id])?;
            Ok(None)
        }
        (None, true) => Ok(None),
        (Some(id), false) => {
            tx.execute(
                r#"UPDATE sessions SET start = ?1, "end" = ?2 WHERE id = ?3"#,
                params![s.start, s.end, id],
            )?;
            Ok(Some(id))
        }
        (None, false) => {
            tx.execute(
                r#"INSERT INTO sessions(app_id, start, "end") VALUES (?1, ?2, ?3)"#,
                params![s.app_id, s.start, s.end],
            )?;
            Ok(Some(tx.last_insert_rowid()))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tracker::Sample;

    const DAY: i64 = 86_400;
    const T0: i64 = 20_000 * DAY + 10 * 3600;

    fn utc_day_end(ts: i64) -> i64 {
        (ts.div_euclid(DAY) + 1) * DAY
    }

    fn feed(t: &mut Tracker, app: i64, from: i64, to: i64) {
        for ts in from..to {
            t.tick(&Sample { ts, app: Some(app), ..Default::default() });
        }
    }

    fn total(s: &Store, from: i64, to: i64) -> i64 {
        s.spans(from, to).unwrap().iter().map(Span::secs).sum()
    }

    fn row_count(s: &Store) -> i64 {
        s.conn.query_row("SELECT COUNT(*) FROM sessions", [], |r| r.get(0)).unwrap()
    }

    #[test]
    fn app_ids_are_stable_and_case_insensitive() {
        let s = Store::open_in_memory().unwrap();
        let a = s.app_id(r"C:\Windows\notepad.exe", "Notepad").unwrap();
        let b = s.app_id(r"C:\WINDOWS\NOTEPAD.EXE", "Notepad").unwrap();
        let c = s.app_id(r"C:\Windows\explorer.exe", "Windows Explorer").unwrap();
        assert_eq!(a, b);
        assert_ne!(a, c);
        assert_eq!(s.apps().unwrap().len(), 2);
    }

    #[test]
    fn repeated_flushes_update_the_open_session_in_place() {
        let mut s = Store::open_in_memory().unwrap();
        let app = s.app_id("a.exe", "A").unwrap();
        let mut t = Tracker::new(utc_day_end);
        feed(&mut t, app, T0, T0 + 30);
        s.flush(&mut t).unwrap();
        feed(&mut t, app, T0 + 30, T0 + 60);
        s.flush(&mut t).unwrap();
        t.close();
        s.flush(&mut t).unwrap();
        assert_eq!(row_count(&s), 1);
        assert_eq!(total(&s, T0 - DAY, T0 + DAY), 60);
    }

    #[test]
    fn spans_are_clipped_to_the_range() {
        let mut s = Store::open_in_memory().unwrap();
        let a = s.app_id("a.exe", "A").unwrap();
        let mut t = Tracker::new(utc_day_end);
        feed(&mut t, a, T0, T0 + 100);
        s.flush(&mut t).unwrap();
        assert_eq!(total(&s, T0 + 40, T0 + 60), 20);
        assert_eq!(s.spans(T0 + 90, T0 + 1000).unwrap(), vec![Span { app_id: a, start: T0 + 90, end: T0 + 100 }]);
        assert_eq!(total(&s, T0 + 100, T0 + 200), 0);
    }

    #[test]
    fn midnight_split_after_flush_keeps_both_halves() {
        let mut s = Store::open_in_memory().unwrap();
        let a = s.app_id("a.exe", "A").unwrap();
        let mid = utc_day_end(T0);
        let mut t = Tracker::new(utc_day_end);
        feed(&mut t, a, mid - 20, mid - 10);
        s.flush(&mut t).unwrap(); // open session now has a row id
        feed(&mut t, a, mid - 10, mid + 10);
        s.flush(&mut t).unwrap();
        assert_eq!(total(&s, mid - DAY, mid), 20);
        assert_eq!(total(&s, mid, mid + DAY), 10);
    }

    #[test]
    fn idle_subtraction_rewrites_and_deletes_stored_rows() {
        let mut s = Store::open_in_memory().unwrap();
        let a = s.app_id("a.exe", "A").unwrap();
        let b = s.app_id("b.exe", "B").unwrap();
        let mut t = Tracker::new(utc_day_end);
        feed(&mut t, a, T0, T0 + 60); // active
        let lim = t.idle_limit() as i64;
        for i in 60..60 + lim {
            // idle from second 60; a popup takes focus at 90
            let app = if i < 90 { a } else { b };
            t.tick(&Sample { ts: T0 + i, app: Some(app), idle_secs: (i - 59) as u64, ..Default::default() });
            if i % 30 == 0 {
                s.flush(&mut t).unwrap();
            }
        }
        s.flush(&mut t).unwrap();
        assert_eq!(s.spans(T0 - DAY, T0 + DAY).unwrap(), vec![Span { app_id: a, start: T0, end: T0 + 60 }]);
        assert_eq!(row_count(&s), 1, "the all-idle session of B is deleted");
    }

    #[test]
    fn data_survives_reopen_and_reads_read_only() {
        let dir = std::env::temp_dir().join(format!("screentime-test-{}", std::process::id()));
        let path = dir.join("data.db");
        let _ = std::fs::remove_dir_all(&dir);
        assert!(Store::open_read_only(&path).unwrap().is_none());
        {
            let mut s = Store::open(&path).unwrap();
            let a = s.app_id("a.exe", "A").unwrap();
            let mut t = Tracker::new(utc_day_end);
            feed(&mut t, a, T0, T0 + 42);
            s.flush(&mut t).unwrap();
            // A reader can open while the writer is live.
            let r = Store::open_read_only(&path).unwrap().unwrap();
            assert_eq!(total(&r, T0, T0 + DAY), 42);
            assert!(r.app_id("x.exe", "X").is_err(), "read-only handle must not write");
            // Dropped without closing the session: like a crash after a flush.
        }
        let s = Store::open_read_only(&path).unwrap().unwrap();
        assert_eq!(total(&s, T0, T0 + DAY), 42);
        drop(s);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
