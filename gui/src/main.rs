//! Screentime desktop app. Reads the tracker's database read-only; closing
//! the window keeps it in the tray, and the tracker runs independently.
#![windows_subsystem = "windows"]

mod app;
mod chart;
mod data;
mod logo;
mod palette;
mod style;
mod tray;
mod views;

use screentime_core::{win, win_ui};

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
    let Some(_instance) = win::claim_instance(win::names::APP_INSTANCE) else {
        win_ui::allow_any_foreground();
        win::signal(win::names::APP_SHOW);
        return Ok(());
    };
    app::run()
}
