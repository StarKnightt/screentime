//! Screentime desktop app. Reads the tracker's database read-only; closing
//! the window keeps it in the tray, and the tracker runs independently.
#![windows_subsystem = "windows"]

mod app;
mod chart;
mod data;
mod logo;
mod palette;
mod smooth;
mod style;
mod tray;
mod views;

use screentime_core::{win, win_ui};

/// The single-instance names. A window opened on another database
/// (`SCREENTIME_DB`: demos, tests) or in a test run (`SCREENTIME_TEST_ID`) is
/// its own instance, so it never takes over or blocks the everyday app.
pub fn instance_name(base: &str) -> String {
    let db = std::env::var_os("SCREENTIME_DB").map(|d| d.to_string_lossy().to_lowercase());
    let test = std::env::var("SCREENTIME_TEST_ID").ok().filter(|t| !t.is_empty());
    if db.is_none() && test.is_none() {
        return base.to_string();
    }
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    (db, test).hash(&mut h);
    format!("{base}.{:016x}", h.finish())
}

fn main() -> iced::Result {
    // The release is one file: `Screentime.exe --tracker` is the windowless
    // tracker (what "start at login" runs). A source build can still use the
    // separate screentime-tracker.exe.
    if std::env::args().any(|a| a == app::TRACKER_ARG) {
        match screentime_core::daemon::run() {
            Ok(screentime_core::daemon::Exit::Stopped) => std::process::exit(0),
            Ok(screentime_core::daemon::Exit::AlreadyRunning) => std::process::exit(2),
            Err(e) => {
                screentime_core::daemon::log(&e);
                std::process::exit(1);
            }
        }
    }
    // One app per login session: a second launch just shows the first.
    let Some(_instance) = win::claim_instance(&instance_name(win::names::APP_INSTANCE)) else {
        win_ui::allow_any_foreground();
        win::signal(&instance_name(win::names::APP_SHOW));
        return Ok(());
    };
    app::run()
}
