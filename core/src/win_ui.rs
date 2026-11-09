//! Win32 helpers for front ends: app icons, theme, known folders.

use std::ffi::c_void;
use std::path::PathBuf;
use std::ptr::null_mut;

use windows_sys::Win32::Graphics::Gdi::{
    DeleteObject, GetDC, GetDIBits, GetObjectW, ReleaseDC, BITMAP, BITMAPINFO, BITMAPINFOHEADER,
    BI_RGB, DIB_RGB_COLORS, HBITMAP,
};
use windows_sys::Win32::System::Com::CoTaskMemFree;
use windows_sys::Win32::System::Registry::{
    RegCloseKey, RegGetValueW, RegNotifyChangeKeyValue, RegOpenKeyExW, HKEY, HKEY_CURRENT_USER,
    KEY_NOTIFY, KEY_READ, REG_NOTIFY_CHANGE_LAST_SET, RRF_RT_REG_DWORD,
};
use windows_sys::Win32::UI::Shell::{FOLDERID_Downloads, SHDefExtractIconW, SHGetKnownFolderPath};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    AllowSetForegroundWindow, DestroyIcon, GetIconInfo, HICON, ICONINFO,
};

use crate::win::{from_wide, wide};

const PERSONALIZE: &str = r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize";

/// Straight (non-premultiplied) RGBA pixels.
#[derive(Debug, Clone)]
pub struct Rgba {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u8>,
}

/// The icon of an executable, extracted at `size` px (the closest the file
/// provides, scaled by the shell).
pub fn exe_icon(exe: &str, size: u32) -> Option<Rgba> {
    let path = wide(exe);
    let mut icon: HICON = null_mut();
    let hr = unsafe { SHDefExtractIconW(path.as_ptr(), 0, 0, &mut icon, null_mut(), size & 0xFFFF) };
    if hr != 0 || icon.is_null() {
        return None;
    }
    let out = unsafe { icon_pixels(icon) };
    unsafe { DestroyIcon(icon) };
    out
}

unsafe fn bitmap_bgra(bmp: HBITMAP) -> Option<(u32, u32, Vec<u8>)> {
    let mut bm: BITMAP = std::mem::zeroed();
    if GetObjectW(bmp, size_of::<BITMAP>() as i32, &mut bm as *mut _ as *mut c_void) == 0 {
        return None;
    }
    let (w, h) = (bm.bmWidth, bm.bmHeight.abs());
    if w <= 0 || h <= 0 || w > 1024 || h > 1024 {
        return None;
    }
    let mut bi: BITMAPINFO = std::mem::zeroed();
    bi.bmiHeader = BITMAPINFOHEADER {
        biSize: size_of::<BITMAPINFOHEADER>() as u32,
        biWidth: w,
        biHeight: -h, // top-down
        biPlanes: 1,
        biBitCount: 32,
        biCompression: BI_RGB,
        ..std::mem::zeroed()
    };
    let mut buf = vec![0u8; (w * h * 4) as usize];
    let dc = GetDC(null_mut());
    let lines = GetDIBits(dc, bmp, 0, h as u32, buf.as_mut_ptr() as *mut c_void, &mut bi, DIB_RGB_COLORS);
    ReleaseDC(null_mut(), dc);
    (lines == h).then_some((w as u32, h as u32, buf))
}

unsafe fn icon_pixels(icon: HICON) -> Option<Rgba> {
    let mut info: ICONINFO = std::mem::zeroed();
    if GetIconInfo(icon, &mut info) == 0 {
        return None;
    }
    let color = bitmap_bgra(info.hbmColor);
    let mask = if info.hbmMask.is_null() { None } else { bitmap_bgra(info.hbmMask) };
    if !info.hbmColor.is_null() {
        DeleteObject(info.hbmColor);
    }
    if !info.hbmMask.is_null() {
        DeleteObject(info.hbmMask);
    }
    let (w, h, mut px) = color?;
    // Old icons have no alpha channel: take transparency from the AND mask
    // (black = opaque).
    if px.as_chunks::<4>().0.iter().all(|p| p[3] == 0) {
        let (mw, mh, m) = mask?;
        if (mw, mh) != (w, h) {
            return None;
        }
        for (p, mp) in px.as_chunks_mut::<4>().0.iter_mut().zip(m.as_chunks::<4>().0) {
            p[3] = if mp[0] == 0 { 255 } else { 0 };
        }
    }
    for p in px.as_chunks_mut::<4>().0 {
        p.swap(0, 2); // BGRA -> RGBA
    }
    Some(Rgba { width: w, height: h, pixels: px })
}

/// The user's Downloads folder (wherever it was moved to).
pub fn downloads_dir() -> Option<PathBuf> {
    unsafe {
        let mut p = null_mut();
        let hr = SHGetKnownFolderPath(&FOLDERID_Downloads, 0, null_mut(), &mut p);
        if hr != 0 || p.is_null() {
            return None;
        }
        let len = (0..).take_while(|&i| *p.add(i) != 0).count();
        let s = from_wide(std::slice::from_raw_parts(p, len));
        CoTaskMemFree(p as *const c_void);
        Some(PathBuf::from(s))
    }
}

/// Whether Windows "Animation effects" is on (Settings > Accessibility >
/// Visual effects). Off is the platform's reduced-motion preference.
pub fn animations_enabled() -> bool {
    use windows_sys::Win32::UI::WindowsAndMessaging::{SystemParametersInfoW, SPI_GETCLIENTAREAANIMATION};
    let mut on: i32 = 1;
    let ok = unsafe { SystemParametersInfoW(SPI_GETCLIENTAREAANIMATION, 0, &mut on as *mut i32 as *mut c_void, 0) };
    ok == 0 || on != 0
}

/// Whether Windows apps are set to dark mode.
pub fn system_dark_mode() -> bool {
    let mut value = 1u32;
    let mut size = 4u32;
    let rc = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            wide(PERSONALIZE).as_ptr(),
            wide("AppsUseLightTheme").as_ptr(),
            RRF_RT_REG_DWORD,
            null_mut(),
            &mut value as *mut u32 as *mut c_void,
            &mut size,
        )
    };
    rc == 0 && value == 0
}

/// Call `on_change` (on a background thread) whenever the Windows
/// personalisation settings change. Blocks in the kernel; costs nothing.
pub fn watch_theme(on_change: impl Fn() + Send + 'static) {
    std::thread::spawn(move || unsafe {
        let mut key: HKEY = null_mut();
        if RegOpenKeyExW(HKEY_CURRENT_USER, wide(PERSONALIZE).as_ptr(), 0, KEY_NOTIFY | KEY_READ, &mut key) != 0 {
            return;
        }
        while RegNotifyChangeKeyValue(key, 0, REG_NOTIFY_CHANGE_LAST_SET, null_mut(), 0) == 0 {
            on_change();
        }
        RegCloseKey(key);
    });
}

/// Let whichever process we signal next bring its window to the front.
pub fn allow_any_foreground() {
    const ASFW_ANY: u32 = u32::MAX;
    unsafe { AllowSetForegroundWindow(ASFW_ANY) };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_explorer_icon() {
        let icon = exe_icon(r"C:\Windows\explorer.exe", 48).expect("explorer has an icon");
        assert_eq!((icon.width, icon.height), (48, 48));
        assert_eq!(icon.pixels.len(), 48 * 48 * 4);
        assert!(icon.pixels.as_chunks::<4>().0.iter().any(|p| p[3] > 200), "has opaque pixels");
        assert!(icon.pixels.as_chunks::<4>().0.iter().any(|p| p[3] == 0), "has a transparent margin");
    }

    #[test]
    fn missing_files_have_no_icon_and_downloads_exist() {
        assert!(exe_icon(r"C:\definitely\not\here.exe", 32).is_none());
        assert!(downloads_dir().is_some());
    }
}
