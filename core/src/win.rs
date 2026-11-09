//! Thin, safe wrappers over the Win32 calls the tracker needs.
//! Everything here is cheap enough to run once per second.

use std::ffi::{c_void, OsString};
use std::os::windows::ffi::{OsStrExt, OsStringExt};
use std::ptr::{null, null_mut};

use windows_sys::Win32::Foundation::{
    CloseHandle, GetLastError, ERROR_ALREADY_EXISTS, HANDLE, HWND, LPARAM, RECT, WAIT_OBJECT_0,
};
use windows_sys::Win32::Graphics::Dwm::{DwmGetWindowAttribute, DWMWA_CLOAKED};
use windows_sys::Win32::Graphics::Gdi::{
    GetMonitorInfoW, MonitorFromWindow, MONITORINFO, MONITOR_DEFAULTTONEAREST,
};
use windows_sys::Win32::Storage::FileSystem::{
    GetFileVersionInfoSizeW, GetFileVersionInfoW, VerQueryValueW,
};
use windows_sys::Win32::System::Registry::{
    RegDeleteKeyValueW, RegGetValueW, RegSetKeyValueW, HKEY_CURRENT_USER, REG_SZ, RRF_RT_REG_SZ,
};
use windows_sys::Win32::System::StationsAndDesktops::{
    CloseDesktop, GetUserObjectInformationW, OpenInputDesktop, UOI_NAME,
};
use windows_sys::Win32::System::SystemInformation::GetTickCount;
use windows_sys::Win32::System::Threading::{
    CreateEventW, CreateMutexW, GetCurrentProcess, OpenEventW, OpenMutexW, OpenProcess,
    ProcessPowerThrottling, QueryFullProcessImageNameW, SetEvent, SetPriorityClass,
    SetProcessInformation, WaitForSingleObject, BELOW_NORMAL_PRIORITY_CLASS,
    PROCESS_NAME_WIN32, PROCESS_POWER_THROTTLING_CURRENT_VERSION,
    PROCESS_POWER_THROTTLING_EXECUTION_SPEED, PROCESS_POWER_THROTTLING_IGNORE_TIMER_RESOLUTION,
    PROCESS_POWER_THROTTLING_STATE, PROCESS_QUERY_LIMITED_INFORMATION,
};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{GetLastInputInfo, LASTINPUTINFO};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    EnumChildWindows, GetClassNameW, GetDesktopWindow, GetForegroundWindow, GetShellWindow,
    GetWindowLongW, GetWindowRect, GetWindowThreadProcessId, IsIconic, IsWindowVisible,
    GWL_EXSTYLE, WS_EX_APPWINDOW, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW,
};

const SYNCHRONIZE: u32 = 0x0010_0000;
const EVENT_MODIFY_STATE: u32 = 0x0002;
const DESKTOP_READOBJECTS: u32 = 0x0001;

const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
const RUN_VALUE: &str = "Screentime";

/// The Run value name. `SCREENTIME_TEST_ID` gives a test run its own value,
/// so testing the first-run flow never reads or writes the real entry.
fn run_value() -> String {
    match std::env::var("SCREENTIME_TEST_ID") {
        Ok(id) if !id.is_empty() => format!("{RUN_VALUE}.test.{id}"),
        _ => RUN_VALUE.to_string(),
    }
}

pub(crate) fn wide(s: impl AsRef<std::ffi::OsStr>) -> Vec<u16> {
    s.as_ref().encode_wide().chain(Some(0)).collect()
}

pub(crate) fn from_wide(buf: &[u16]) -> String {
    let end = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    OsString::from_wide(&buf[..end]).to_string_lossy().into_owned()
}

/// Owned kernel handle, closed on drop.
pub struct Handle(HANDLE);

impl Drop for Handle {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe { CloseHandle(self.0) };
        }
    }
}

// ---------------------------------------------------------------- probing

/// Opaque top-level window handle. Only ever passed back to Win32.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Window(HWND);

/// What the tracker checks about the foreground window before counting it.
///
/// Windows hands focus to plenty of windows nobody is looking at: tray flyouts
/// and notification popups (tool windows, often never activated), windows that
/// are cloaked by DWM (a UWP app suspended in the background, a window on
/// another virtual desktop), minimized windows, and invisible or zero-size
/// helper windows. Only a real, visible window counts as screen time.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct WindowFacts {
    pub visible: bool,
    pub cloaked: bool,
    pub minimized: bool,
    pub width: i32,
    pub height: i32,
    pub ex_style: u32,
}

/// Smaller than this on either side is a helper window, not something you use.
const MIN_SIDE: i32 = 8;

impl WindowFacts {
    /// True for a window the user can actually see and is working in.
    pub fn in_use(&self) -> bool {
        let tool = self.ex_style & WS_EX_TOOLWINDOW != 0 && self.ex_style & WS_EX_APPWINDOW == 0;
        let no_activate = self.ex_style & WS_EX_NOACTIVATE != 0;
        self.visible
            && !self.cloaked
            && !self.minimized
            && self.width >= MIN_SIDE
            && self.height >= MIN_SIDE
            && !tool
            && !no_activate
    }
}

/// Read a window's [`WindowFacts`].
pub fn window_facts(Window(hwnd): Window) -> WindowFacts {
    unsafe {
        let mut cloaked = 0u32;
        let hr = DwmGetWindowAttribute(
            hwnd,
            DWMWA_CLOAKED as u32,
            &mut cloaked as *mut u32 as *mut c_void,
            size_of::<u32>() as u32,
        );
        let mut r = RECT { left: 0, top: 0, right: 0, bottom: 0 };
        let has_rect = GetWindowRect(hwnd, &mut r) != 0;
        WindowFacts {
            visible: IsWindowVisible(hwnd) != 0,
            cloaked: hr >= 0 && cloaked != 0,
            minimized: IsIconic(hwnd) != 0,
            width: if has_rect { r.right - r.left } else { 0 },
            height: if has_rect { r.bottom - r.top } else { 0 },
            ex_style: GetWindowLongW(hwnd, GWL_EXSTYLE) as u32,
        }
    }
}

/// Foreground window and the process that owns it, if it is a window the user
/// is really using (see [`WindowFacts::in_use`]).
pub fn foreground() -> Option<(Window, u32)> {
    unsafe {
        let hwnd = GetForegroundWindow();
        if hwnd.is_null() || !window_facts(Window(hwnd)).in_use() {
            return None;
        }
        let mut pid = 0u32;
        GetWindowThreadProcessId(hwnd, &mut pid);
        (pid != 0).then_some((Window(hwnd), pid))
    }
}

/// Full Win32 path of a process's executable.
pub fn exe_path(pid: u32) -> Option<String> {
    unsafe {
        let h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if h.is_null() {
            return None;
        }
        let _h = Handle(h);
        let mut buf = [0u16; 1024];
        let mut len = buf.len() as u32;
        if QueryFullProcessImageNameW(h, PROCESS_NAME_WIN32, buf.as_mut_ptr(), &mut len) == 0 {
            return None;
        }
        Some(OsString::from_wide(&buf[..len as usize]).to_string_lossy().into_owned())
    }
}

/// UWP apps are hosted by ApplicationFrameHost.exe; the real app owns a child
/// window. Returns that child's pid, if there is one.
pub fn uwp_child_pid(Window(hwnd): Window, host_pid: u32) -> Option<u32> {
    struct Find {
        host: u32,
        found: u32,
    }
    unsafe extern "system" fn cb(child: HWND, lp: LPARAM) -> i32 {
        let f = &mut *(lp as *mut Find);
        let mut pid = 0u32;
        GetWindowThreadProcessId(child, &mut pid);
        if pid != 0 && pid != f.host {
            f.found = pid;
            return 0; // stop
        }
        1
    }
    let mut f = Find { host: host_pid, found: 0 };
    unsafe { EnumChildWindows(hwnd, Some(cb), &mut f as *mut Find as LPARAM) };
    (f.found != 0).then_some(f.found)
}

/// Human name of an executable: its version-info FileDescription, else the
/// file stem ("notepad").
pub fn app_name(exe: &str) -> String {
    crate::names::display_name(file_description(exe).as_deref(), exe)
}

fn file_description(exe: &str) -> Option<String> {
    unsafe {
        let path = wide(exe);
        let size = GetFileVersionInfoSizeW(path.as_ptr(), null_mut());
        if size == 0 {
            return None;
        }
        let mut data = vec![0u8; size as usize];
        if GetFileVersionInfoW(path.as_ptr(), 0, size, data.as_mut_ptr() as *mut c_void) == 0 {
            return None;
        }
        // Prefer the file's own language/codepage, then common fallbacks.
        let mut langs: Vec<(u16, u16)> = Vec::new();
        let mut p: *mut c_void = null_mut();
        let mut len = 0u32;
        let q = wide(r"\VarFileInfo\Translation");
        if VerQueryValueW(data.as_ptr() as *const c_void, q.as_ptr(), &mut p, &mut len) != 0
            && !p.is_null()
        {
            let pairs = std::slice::from_raw_parts(p as *const u16, len as usize / 2);
            langs.extend(pairs.as_chunks::<2>().0.iter().map(|&[lang, cp]| (lang, cp)));
        }
        langs.extend([(0x0409, 0x04B0), (0x0409, 0x04E4), (0x0000, 0x04B0)]);
        for (lang, cp) in langs {
            let q = wide(format!(r"\StringFileInfo\{lang:04x}{cp:04x}\FileDescription"));
            let mut p: *mut c_void = null_mut();
            let mut len = 0u32;
            if VerQueryValueW(data.as_ptr() as *const c_void, q.as_ptr(), &mut p, &mut len) != 0
                && !p.is_null()
                && len > 0
            {
                let s = std::slice::from_raw_parts(p as *const u16, len as usize);
                return Some(from_wide(s));
            }
        }
        None
    }
}

/// Seconds since the last keyboard or mouse input in this session.
pub fn idle_secs() -> u64 {
    unsafe {
        let mut lii = LASTINPUTINFO { cbSize: size_of::<LASTINPUTINFO>() as u32, dwTime: 0 };
        if GetLastInputInfo(&mut lii) == 0 {
            return 0;
        }
        // Both are 32-bit tick counts; wrapping_sub survives the 49-day wrap.
        u64::from(GetTickCount().wrapping_sub(lii.dwTime)) / 1000
    }
}

/// True when the input desktop is not the user's ("Default"): lock screen,
/// sign-in screen, or a UAC prompt on the secure desktop.
pub fn is_locked() -> bool {
    unsafe {
        let desk = OpenInputDesktop(0, 0, DESKTOP_READOBJECTS);
        if desk.is_null() {
            return true; // access denied: the secure desktop is active
        }
        let mut buf = [0u16; 64];
        let mut needed = 0u32;
        let ok = GetUserObjectInformationW(
            desk as HANDLE,
            UOI_NAME,
            buf.as_mut_ptr() as *mut c_void,
            (buf.len() * 2) as u32,
            &mut needed,
        );
        CloseDesktop(desk);
        ok != 0 && !from_wide(&buf).eq_ignore_ascii_case("Default")
    }
}

/// Foreground window covers its entire monitor (and is not the desktop).
pub fn is_fullscreen(Window(hwnd): Window) -> bool {
    unsafe {
        if hwnd == GetDesktopWindow() || hwnd == GetShellWindow() {
            return false;
        }
        let mut cls = [0u16; 32];
        let n = GetClassNameW(hwnd, cls.as_mut_ptr(), cls.len() as i32);
        let cls = from_wide(&cls[..n.max(0) as usize]);
        if cls == "Progman" || cls == "WorkerW" {
            return false;
        }
        let mut r = RECT { left: 0, top: 0, right: 0, bottom: 0 };
        if GetWindowRect(hwnd, &mut r) == 0 {
            return false;
        }
        let mon = MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST);
        let mut mi: MONITORINFO = std::mem::zeroed();
        mi.cbSize = size_of::<MONITORINFO>() as u32;
        if GetMonitorInfoW(mon, &mut mi) == 0 {
            return false;
        }
        let m = mi.rcMonitor;
        r.left <= m.left && r.top <= m.top && r.right >= m.right && r.bottom >= m.bottom
    }
}

// ---------------------------------------------------------- process control

/// Named kernel objects shared by the tracker, the CLI and the GUI. `Local\`
/// scopes them to this login session.
pub mod names {
    pub const TRACKER_INSTANCE: &str = r"Local\screentime.tracker.instance";
    pub const TRACKER_STOP: &str = r"Local\screentime.tracker.stop";
    /// Set after changing config.ini so the tracker re-reads it at once.
    pub const TRACKER_RELOAD: &str = r"Local\screentime.tracker.reload";
    pub const APP_INSTANCE: &str = r"Local\screentime.app.instance";
    /// Set by a second GUI launch to make the first one show its window.
    pub const APP_SHOW: &str = r"Local\screentime.app.show";
}

/// Holds a single-instance mutex for as long as it lives.
pub struct InstanceGuard(#[allow(dead_code)] Handle);

/// Claim a named single-instance mutex. `None` if another process holds it.
pub fn claim_instance(name: &str) -> Option<InstanceGuard> {
    unsafe {
        let h = CreateMutexW(null(), 1, wide(name).as_ptr());
        if h.is_null() {
            return None;
        }
        let guard = Handle(h);
        (GetLastError() != ERROR_ALREADY_EXISTS).then_some(InstanceGuard(guard))
    }
}

/// Claim the tracker's single-instance mutex.
pub fn claim_single_instance() -> Option<InstanceGuard> {
    claim_instance(names::TRACKER_INSTANCE)
}

/// Whether a named mutex exists (its owner is running in this session).
pub fn instance_running(name: &str) -> bool {
    unsafe {
        let h = OpenMutexW(SYNCHRONIZE, 0, wide(name).as_ptr());
        let running = !h.is_null();
        drop(Handle(h));
        running
    }
}

/// Whether a tracker is running in this login session.
pub fn tracker_running() -> bool {
    instance_running(names::TRACKER_INSTANCE)
}

/// A named auto-reset event owned by the waiting side.
pub struct NamedEvent(Handle);

impl NamedEvent {
    pub fn create(name: &str) -> Option<Self> {
        let h = unsafe { CreateEventW(null(), 0, 0, wide(name).as_ptr()) };
        (!h.is_null()).then(|| Self(Handle(h)))
    }

    pub fn raw(&self) -> HANDLE {
        self.0 .0
    }

    /// Wait up to `ms` (`u32::MAX` = forever); true if the event fired.
    pub fn wait(&self, ms: u32) -> bool {
        unsafe { WaitForSingleObject(self.0 .0, ms) == WAIT_OBJECT_0 }
    }
}

// SAFETY: an event handle may be waited on and closed from any thread.
unsafe impl Send for NamedEvent {}
unsafe impl Sync for NamedEvent {}

/// Fire a named event. False if nobody created it (its owner is not running).
pub fn signal(name: &str) -> bool {
    unsafe {
        let h = OpenEventW(EVENT_MODIFY_STATE, 0, wide(name).as_ptr());
        if h.is_null() {
            return false;
        }
        let h = Handle(h);
        SetEvent(h.0) != 0
    }
}

/// Ask a running tracker to flush and exit. False if none is listening.
pub fn signal_stop() -> bool {
    signal(names::TRACKER_STOP)
}

/// Tell a running tracker that config.ini changed.
pub fn signal_reload() -> bool {
    signal(names::TRACKER_RELOAD)
}

/// Stop child processes from inheriting this process's stdin/stdout/stderr.
///
/// `CreateProcess` passes on every inheritable handle, not just the child's
/// own stdio. Without this, a detached tracker would keep a caller's output
/// pipe open (e.g. `screentime run | more`), and the reader would never see
/// end-of-file until the tracker exited.
pub fn disinherit_std_handles() {
    use windows_sys::Win32::Foundation::{SetHandleInformation, HANDLE_FLAG_INHERIT};
    use windows_sys::Win32::System::Console::{
        GetStdHandle, STD_ERROR_HANDLE, STD_INPUT_HANDLE, STD_OUTPUT_HANDLE,
    };
    for which in [STD_INPUT_HANDLE, STD_OUTPUT_HANDLE, STD_ERROR_HANDLE] {
        unsafe {
            let h = GetStdHandle(which);
            if !h.is_null() && h as isize != -1 {
                SetHandleInformation(h, HANDLE_FLAG_INHERIT, 0);
            }
        }
    }
}

/// Below-normal priority plus EcoQoS: the scheduler may run us on efficient
/// cores at low clocks, and we do not raise the system timer resolution.
pub fn make_process_frugal() {
    unsafe {
        let me = GetCurrentProcess();
        SetPriorityClass(me, BELOW_NORMAL_PRIORITY_CLASS);
        let state = PROCESS_POWER_THROTTLING_STATE {
            Version: PROCESS_POWER_THROTTLING_CURRENT_VERSION,
            ControlMask: PROCESS_POWER_THROTTLING_EXECUTION_SPEED
                | PROCESS_POWER_THROTTLING_IGNORE_TIMER_RESOLUTION,
            StateMask: PROCESS_POWER_THROTTLING_EXECUTION_SPEED
                | PROCESS_POWER_THROTTLING_IGNORE_TIMER_RESOLUTION,
        };
        SetProcessInformation(
            me,
            ProcessPowerThrottling,
            &state as *const _ as *const c_void,
            size_of::<PROCESS_POWER_THROTTLING_STATE>() as u32,
        );
    }
}

// ----------------------------------------------------------------- startup

/// Register `command` to run at login (HKCU Run key, current user only).
pub fn set_startup(command: &str) -> std::io::Result<()> {
    let data = wide(command);
    let rc = unsafe {
        RegSetKeyValueW(
            HKEY_CURRENT_USER,
            wide(RUN_KEY).as_ptr(),
            wide(run_value()).as_ptr(),
            REG_SZ,
            data.as_ptr() as *const c_void,
            (data.len() * 2) as u32,
        )
    };
    win32_result(rc)
}

/// Remove the login entry. Succeeds if it was not there.
pub fn clear_startup() -> std::io::Result<()> {
    let rc = unsafe {
        RegDeleteKeyValueW(HKEY_CURRENT_USER, wide(RUN_KEY).as_ptr(), wide(run_value()).as_ptr())
    };
    const ERROR_FILE_NOT_FOUND: u32 = 2;
    if rc == ERROR_FILE_NOT_FOUND {
        return Ok(());
    }
    win32_result(rc)
}

/// The registered login command, if any.
pub fn get_startup() -> Option<String> {
    unsafe {
        let mut buf = [0u16; 1024];
        let mut size = (buf.len() * 2) as u32;
        let rc = RegGetValueW(
            HKEY_CURRENT_USER,
            wide(RUN_KEY).as_ptr(),
            wide(run_value()).as_ptr(),
            RRF_RT_REG_SZ,
            null_mut(),
            buf.as_mut_ptr() as *mut c_void,
            &mut size,
        );
        (rc == 0).then(|| from_wide(&buf))
    }
}

fn win32_result(rc: u32) -> std::io::Result<()> {
    if rc == 0 {
        Ok(())
    } else {
        Err(std::io::Error::from_raw_os_error(rc as i32))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn real_executables_get_friendly_names() {
        // Version info of a stock Windows binary: a description, never a file name.
        let name = app_name(r"C:\Windows\explorer.exe");
        assert!(!name.is_empty() && !name.to_ascii_lowercase().ends_with(".exe"), "{name}");
        // A path with no version info falls back to the stem.
        assert_eq!(app_name(r"C:\nope\missing-tool.exe"), "missing-tool");
    }

    fn real() -> WindowFacts {
        WindowFacts { visible: true, width: 1200, height: 800, ..Default::default() }
    }

    #[test]
    fn only_a_real_visible_window_counts() {
        assert!(real().in_use());
        assert!(!WindowFacts { visible: false, ..real() }.in_use(), "invisible");
        assert!(!WindowFacts { cloaked: true, ..real() }.in_use(), "cloaked by DWM");
        assert!(!WindowFacts { minimized: true, ..real() }.in_use(), "minimized");
        assert!(!WindowFacts { width: 0, height: 0, ..real() }.in_use(), "zero size");
        assert!(!WindowFacts { width: 1, ..real() }.in_use(), "a 1 px helper");
        assert!(!WindowFacts { ex_style: WS_EX_TOOLWINDOW, ..real() }.in_use(), "tray flyout");
        assert!(!WindowFacts { ex_style: WS_EX_NOACTIVATE, ..real() }.in_use(), "notification popup");
        // A tool-window bit with WS_EX_APPWINDOW is a real app window (it has a taskbar button).
        assert!(WindowFacts { ex_style: WS_EX_TOOLWINDOW | WS_EX_APPWINDOW, ..real() }.in_use());
    }

    /// The same rules on real windows: read back through Win32 and DWM.
    #[test]
    fn real_windows_are_judged_by_what_win32_reports() {
        use windows_sys::Win32::Graphics::Dwm::{DwmSetWindowAttribute, DWMWA_CLOAK};
        use windows_sys::Win32::UI::WindowsAndMessaging::{
            CreateWindowExW, DestroyWindow, ShowWindow, SW_SHOWMINNOACTIVE, SW_SHOWNOACTIVATE,
            WS_OVERLAPPEDWINDOW,
        };
        let make = |ex: u32| unsafe {
            // Far off screen, never activated: nothing appears or takes focus.
            let h = CreateWindowExW(ex, wide("STATIC").as_ptr(), wide("screentime test").as_ptr(),
                WS_OVERLAPPEDWINDOW, -20000, -20000, 300, 200, null_mut(), null_mut(), null_mut(), null());
            assert!(!h.is_null());
            h
        };
        unsafe {
            let shown = make(0);
            assert!(!window_facts(Window(shown)).in_use(), "created hidden");
            ShowWindow(shown, SW_SHOWNOACTIVATE);
            assert!(window_facts(Window(shown)).in_use(), "{:?}", window_facts(Window(shown)));
            let on: i32 = 1;
            DwmSetWindowAttribute(shown, DWMWA_CLOAK as u32, &on as *const i32 as *const c_void, 4);
            assert!(window_facts(Window(shown)).cloaked && !window_facts(Window(shown)).in_use());

            let tool = make(WS_EX_TOOLWINDOW);
            ShowWindow(tool, SW_SHOWNOACTIVATE);
            assert!(!window_facts(Window(tool)).in_use(), "tool window");

            let mini = make(0);
            ShowWindow(mini, SW_SHOWMINNOACTIVE);
            assert!(window_facts(Window(mini)).minimized && !window_facts(Window(mini)).in_use());

            for h in [shown, tool, mini] {
                DestroyWindow(h);
            }
        }
    }

    #[test]
    fn probes_do_not_fail_in_a_normal_session() {
        let _ = idle_secs();
        let _ = is_locked();
        if let Some((w, pid)) = foreground() {
            let _ = is_fullscreen(w);
            let _ = exe_path(pid);
        }
    }
}
