//! Start at login for the Microsoft Store (MSIX) build.
//!
//! A packaged app may not use the Run key; it declares a `windows.startupTask`
//! in its manifest (task id below, it starts `screentime-tracker.exe`) and turns
//! it on and off through the StartupTask API. The GitHub build keeps using the
//! Run key (see app.rs).
//!
//! Each call runs on its own short-lived thread so a WinRT wait never blocks
//! (or deadlocks) the window's thread.

use windows::core::HSTRING;
use windows::ApplicationModel::{StartupTask, StartupTaskState};

/// Must match `TaskId` in store/AppxManifest.xml.
pub const TASK_ID: &str = "ScreentimeTracker";

fn task() -> windows::core::Result<StartupTask> {
    StartupTask::GetAsync(&HSTRING::from(TASK_ID))?.join()
}

fn on_thread<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> T {
    std::thread::spawn(f).join().expect("startup task thread")
}

/// The task's state, or `None` if it cannot be read.
pub fn state() -> Option<StartupTaskState> {
    on_thread(|| task().and_then(|t| t.State()).ok())
}

pub fn is_on() -> bool {
    matches!(state(), Some(StartupTaskState::Enabled | StartupTaskState::EnabledByPolicy))
}

/// Turn it on or off. Windows keeps the last word: if the user turned it off in
/// Settings > Apps > Startup (or Task Manager), only they can turn it back on there.
pub fn set(on: bool) -> Result<(), String> {
    on_thread(move || {
        let t = task().map_err(|e| e.message().to_string())?;
        if !on {
            return t.Disable().map_err(|e| e.message().to_string());
        }
        match t.RequestEnableAsync().and_then(|op| op.join()).map_err(|e| e.message().to_string())? {
            StartupTaskState::Enabled | StartupTaskState::EnabledByPolicy => Ok(()),
            StartupTaskState::DisabledByUser => Err("Windows has start at login turned off for Screentime. Turn it on in Settings > Apps > Startup.".into()),
            StartupTaskState::DisabledByPolicy => Err("Start at login is turned off by your organisation's policy.".into()),
            _ => Err("Windows did not turn on start at login.".into()),
        }
    })
}

/// First run of the Store build: on by default, unless the user already turned
/// it off in Windows (then their choice stands).
pub fn enable_by_default() {
    if state() == Some(StartupTaskState::Disabled) {
        let _ = set(true);
    }
}
