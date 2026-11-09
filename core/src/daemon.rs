//! The tracker loop: sample once a second, batch writes every 30 s, save at
//! once on shutdown, logoff and sleep.
//!
//! The process owns one hidden top-level window. It is never shown; it exists
//! so Windows delivers WM_QUERYENDSESSION / WM_ENDSESSION (shutdown, logoff,
//! restart) and WM_POWERBROADCAST (sleep), which only reach windows.

use std::cell::RefCell;
use std::collections::HashMap;
use std::io::Write;
use std::ptr::{null, null_mut};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, WAIT_OBJECT_0, WAIT_TIMEOUT, WPARAM};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DispatchMessageW, MsgWaitForMultipleObjects, PeekMessageW,
    RegisterClassW, TranslateMessage, MSG, PM_REMOVE, QS_ALLINPUT, WM_ENDSESSION,
    WM_POWERBROADCAST, WM_QUERYENDSESSION, WNDCLASSW,
};

use crate::config::Config;
use crate::names;
use crate::store::{self, Store};
use crate::time::{local_day_end, now_ts};
use crate::tracker::{Sample, Tracker};
use crate::win::{self, NamedEvent};

/// How often finished/open sessions are committed. Bounds crash loss.
pub const FLUSH_EVERY_SECS: u64 = 30;

const PBT_APMSUSPEND: usize = 0x4;

/// Resolves the foreground window to an app id, caching the expensive parts
/// (process path lookups and version-info reads) between ticks.
struct Probe {
    last: Option<(win::Window, u32, Option<i64>)>, // (window, pid, app)
    by_exe: HashMap<String, Option<i64>>,
}

impl Probe {
    fn new() -> Self {
        Self { last: None, by_exe: HashMap::new() }
    }

    fn sample(&mut self, store: &Store, idle_limit: u64) -> Sample {
        let ts = now_ts();
        let locked = win::is_locked();
        let idle_secs = win::idle_secs();
        let fg = if locked { None } else { win::foreground() };
        let app = fg.and_then(|(hwnd, pid)| self.app_for(store, hwnd, pid));
        // Only worth the extra calls when it can change the outcome.
        let fullscreen = idle_secs >= idle_limit && fg.is_some_and(|(hwnd, _)| win::is_fullscreen(hwnd));
        Sample { ts, app, idle_secs, fullscreen, locked }
    }

    fn app_for(&mut self, store: &Store, hwnd: win::Window, pid: u32) -> Option<i64> {
        if let Some((h, p, app)) = self.last {
            if h == hwnd && p == pid {
                return app;
            }
        }
        let app = self.resolve(store, hwnd, pid);
        self.last = Some((hwnd, pid, app));
        app
    }

    fn resolve(&mut self, store: &Store, hwnd: win::Window, pid: u32) -> Option<i64> {
        let mut exe = win::exe_path(pid)?;
        if exe.to_ascii_lowercase().ends_with(r"\applicationframehost.exe") {
            if let Some(real) = win::uwp_child_pid(hwnd, pid).and_then(win::exe_path) {
                exe = real;
            }
        }
        let key = exe.to_lowercase();
        if let Some(&id) = self.by_exe.get(&key) {
            return id;
        }
        // Shell plumbing (Start, search, lock screen…) is not an app.
        let id = if names::is_hidden(&exe) {
            None
        } else {
            match store.app_id(&exe, &win::app_name(&exe)) {
                Ok(id) => Some(id),
                Err(e) => {
                    log(&format!("app_id({exe}): {e}"));
                    return None; // not cached: retry next time
                }
            }
        };
        self.by_exe.insert(key, id);
        id
    }
}

/// Append a line to `tracker.log` next to the database. Errors only; the
/// tracker has no console to print to.
pub fn log(msg: &str) {
    let path = store::data_dir().join("tracker.log");
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
        let _ = writeln!(f, "{} {msg}", now_ts());
    }
}

/// Milliseconds until just after the next wall-clock second, so each tick
/// lands in its own second and none is skipped or doubled.
fn ms_to_next_second() -> u32 {
    let ms = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.subsec_millis()).unwrap_or(0);
    1000 - ms + 10
}

/// Re-read display names once per start, so app updates (and naming fixes)
/// reach rows recorded earlier. A handful of version-info reads; milliseconds.
fn refresh_app_names(store: &Store) {
    let Ok(apps) = store.apps() else { return };
    for app in apps {
        if std::path::Path::new(&app.exe_path).exists() {
            let name = win::app_name(&app.exe_path);
            if name != app.name {
                let _ = store.rename_app(app.id, &name);
            }
        }
    }
}

/// Everything the window procedure needs to save on shutdown.
struct Ctx {
    store: Store,
    tracker: Tracker,
    ending: bool,
}

thread_local! {
    static CTX: RefCell<Option<Ctx>> = const { RefCell::new(None) };
}

/// Close the open session and write everything now.
fn save_now(why: &str) {
    CTX.with(|c| {
        if let Ok(mut c) = c.try_borrow_mut() {
            if let Some(ctx) = c.as_mut() {
                ctx.tracker.close();
                if let Err(e) = ctx.store.flush(&mut ctx.tracker) {
                    log(&format!("flush on {why}: {e}"));
                }
            }
        }
    });
}

unsafe extern "system" fn wnd_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    match msg {
        // Save first, then agree: the session may end at any moment after.
        WM_QUERYENDSESSION => {
            save_now("query-end-session");
            1
        }
        WM_ENDSESSION => {
            if wp != 0 {
                save_now("end-session");
                CTX.with(|c| {
                    if let Ok(mut c) = c.try_borrow_mut() {
                        if let Some(ctx) = c.as_mut() {
                            ctx.ending = true;
                        }
                    }
                });
            }
            0
        }
        WM_POWERBROADCAST if wp == PBT_APMSUSPEND => {
            save_now("suspend");
            1
        }
        _ => DefWindowProcW(hwnd, msg, wp, lp),
    }
}

/// A hidden top-level window (message-only windows miss these broadcasts).
fn create_listener_window() -> bool {
    unsafe {
        let class = win::wide("ScreentimeTrackerListener");
        let inst = GetModuleHandleW(null());
        let wc = WNDCLASSW {
            lpfnWndProc: Some(wnd_proc),
            hInstance: inst,
            lpszClassName: class.as_ptr(),
            ..std::mem::zeroed()
        };
        RegisterClassW(&wc);
        let hwnd = CreateWindowExW(
            0,
            class.as_ptr(),
            class.as_ptr(),
            0,
            0,
            0,
            0,
            0,
            null_mut(),
            null_mut(),
            inst,
            null(),
        );
        !hwnd.is_null()
    }
}

/// Dispatch pending window messages. True if the session is ending.
fn pump_messages() -> bool {
    unsafe {
        let mut msg: MSG = std::mem::zeroed();
        while PeekMessageW(&mut msg, null_mut(), 0, 0, PM_REMOVE) != 0 {
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
    CTX.with(|c| c.borrow().as_ref().is_some_and(|c| c.ending))
}

pub enum Exit {
    Stopped,
    AlreadyRunning,
}

/// Run the tracker until a stop is requested. Blocks.
pub fn run() -> Result<Exit, String> {
    let Some(_instance) = win::claim_single_instance() else {
        return Ok(Exit::AlreadyRunning);
    };
    // The Store build marks its tracker so windows can tell the builds apart.
    let _store_mark = win::is_packaged().then(|| win::claim_instance(win::names::TRACKER_STORE)).flatten();
    let stop = NamedEvent::create(&win::scoped(win::names::TRACKER_STOP)).ok_or("cannot create stop event")?;
    let reload = NamedEvent::create(&win::scoped(win::names::TRACKER_RELOAD)).ok_or("cannot create reload event")?;
    win::make_process_frugal();

    let path = store::default_db_path();
    let store = Store::open(&path).map_err(|e| format!("open {}: {e}", path.display()))?;
    refresh_app_names(&store);
    let mut config = Config::load();
    let mut tracker = Tracker::new(local_day_end);
    tracker.set_idle_limit(config.idle_secs());
    CTX.with(|c| *c.borrow_mut() = Some(Ctx { store, tracker, ending: false }));
    if !create_listener_window() {
        log("cannot create listener window; shutdown saves rely on the 30 s flush");
    }

    let mut probe = Probe::new();
    let mut last_flush = Instant::now();
    let handles = [stop.raw(), reload.raw()];

    'outer: loop {
        CTX.with(|c| {
            let mut c = c.borrow_mut();
            let ctx = c.as_mut().expect("context set");
            let was_open = ctx.tracker.is_open();
            if config.paused {
                ctx.tracker.close();
            } else {
                let sample = probe.sample(&ctx.store, ctx.tracker.idle_limit());
                ctx.tracker.tick(&sample);
            }
            // Flush on the timer, and when tracking pauses (lock/idle): a lock
            // often precedes sleep or shutdown, and idle subtraction should
            // reach the disk promptly.
            let stopped_counting = was_open && !ctx.tracker.is_open();
            let due = last_flush.elapsed().as_secs() >= FLUSH_EVERY_SECS;
            if stopped_counting || due {
                if let Err(e) = ctx.store.flush(&mut ctx.tracker) {
                    log(&format!("flush: {e}"));
                }
                if due {
                    // Pick up hand edits of config.ini too.
                    config = apply_config(ctx, config, Config::load());
                }
                last_flush = Instant::now();
            }
        });

        // Sleep until the next second, handling events and messages meanwhile.
        loop {
            let r = unsafe { MsgWaitForMultipleObjects(2, handles.as_ptr(), 0, ms_to_next_second(), QS_ALLINPUT) };
            match r {
                x if x == WAIT_OBJECT_0 => break 'outer,
                x if x == WAIT_OBJECT_0 + 1 => CTX.with(|c| {
                    let mut c = c.borrow_mut();
                    let ctx = c.as_mut().expect("context set");
                    config = apply_config(ctx, config, Config::load());
                }),
                x if x == WAIT_OBJECT_0 + 2 => {
                    if pump_messages() {
                        break 'outer;
                    }
                }
                WAIT_TIMEOUT => break,
                _ => break, // WAIT_FAILED: fall back to ticking
            }
        }
    }

    CTX.with(|c| {
        let mut c = c.borrow_mut();
        let ctx = c.as_mut().expect("context set");
        ctx.tracker.close();
        ctx.store.flush(&mut ctx.tracker).map_err(|e| format!("final flush: {e}"))
    })?;
    Ok(Exit::Stopped)
}

fn apply_config(ctx: &mut Ctx, old: Config, new: Config) -> Config {
    if new.idle_minutes != old.idle_minutes {
        ctx.tracker.set_idle_limit(new.idle_secs());
    }
    if new.paused && !old.paused {
        ctx.tracker.close();
        if let Err(e) = ctx.store.flush(&mut ctx.tracker) {
            log(&format!("flush on pause: {e}"));
        }
    }
    new
}
