//! Turning executables into the names people recognise, and hiding the
//! Windows plumbing nobody thinks of as "an app".

use std::path::Path;

/// Known executables, by lower-case file name. `None` hides the process:
/// it is shell plumbing (Start, search, lock screen, popups), not something
/// a person chose to spend time in.
const KNOWN: &[(&str, Option<&str>)] = &[
    // Terminals: the host process owns the window, whatever runs inside.
    ("windowsterminal.exe", Some("Terminal")),
    ("openconsole.exe", Some("Terminal")),
    ("conhost.exe", Some("Terminal")),
    ("explorer.exe", Some("File Explorer")),
    ("snippingtool.exe", Some("Snipping Tool")),
    ("notepad.exe", Some("Notepad")),
    ("mspaint.exe", Some("Paint")),
    ("systemsettings.exe", Some("Settings")),
    ("taskmgr.exe", Some("Task Manager")),
    ("calculatorapp.exe", Some("Calculator")),
    ("msedge.exe", Some("Microsoft Edge")),
    ("screentime-app.exe", Some("Screentime")),
    // Shell plumbing.
    ("shellhost.exe", None),
    ("lockapp.exe", None),
    ("logonui.exe", None),
    ("searchhost.exe", None),
    ("searchapp.exe", None),
    ("searchui.exe", None),
    ("startmenuexperiencehost.exe", None),
    ("shellexperiencehost.exe", None),
    ("textinputhost.exe", None),
    ("applicationframehost.exe", None), // unresolved UWP frame
    ("screenclippinghost.exe", None),
    ("credentialuibroker.exe", None),
    ("consent.exe", None),
    ("dwm.exe", None),
    ("screentime-tracker.exe", None),
];

fn file_name_lower(exe: &str) -> String {
    let name = exe.rsplit(['\\', '/']).next().unwrap_or(exe);
    name.to_ascii_lowercase()
}

fn known(exe: &str) -> Option<Option<&'static str>> {
    let file = file_name_lower(exe);
    KNOWN.iter().find(|(k, _)| *k == file).map(|(_, v)| *v)
}

/// Shell plumbing that is neither tracked nor shown.
pub fn is_hidden(exe: &str) -> bool {
    matches!(known(exe), Some(None))
}

/// Display name from an exe's FileDescription (if any) and its path.
///
/// Known executables get a fixed clean name. Otherwise the description is
/// used; empty ones fall back to the file stem, and descriptions that are
/// just a file name ("Notepad.exe") lose the extension.
pub fn display_name(description: Option<&str>, exe: &str) -> String {
    if let Some(Some(name)) = known(exe) {
        return name.to_string();
    }
    let stem = || {
        Path::new(exe)
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| exe.to_string())
    };
    let Some(desc) = description.map(str::trim).filter(|d| !d.is_empty()) else {
        return stem();
    };
    match desc.len().checked_sub(4) {
        Some(cut) if desc.is_char_boundary(cut) && desc[cut..].eq_ignore_ascii_case(".exe") => {
            desc[..cut].to_string()
        }
        _ => desc.to_string(),
    }
}

/// Name to show for a stored app row: the mapping wins over what was stored,
/// so old rows get clean names without rewriting the database.
pub fn shown_name(stored: &str, exe: &str) -> String {
    match known(exe) {
        Some(Some(name)) => name.to_string(),
        _ => stored.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn description_is_preferred_and_cleaned() {
        let exe = r"C:\Program Files\App\thing.exe";
        assert_eq!(display_name(Some("Firefox"), exe), "Firefox");
        assert_eq!(display_name(Some("  Grok Bot "), exe), "Grok Bot");
        assert_eq!(display_name(Some("Notepad.exe"), exe), "Notepad");
        assert_eq!(display_name(Some("TOOL.EXE"), exe), "TOOL");
    }

    #[test]
    fn missing_description_falls_back_to_file_stem() {
        assert_eq!(display_name(None, r"C:\x\mytool.exe"), "mytool");
        assert_eq!(display_name(Some("   "), r"C:\x\mytool.exe"), "mytool");
        assert_eq!(display_name(Some("é"), r"C:\x\a.exe"), "é");
    }

    #[test]
    fn system_processes_get_clean_names() {
        let wt = r"C:\Program Files\WindowsApps\Microsoft.WindowsTerminal_1.24.12741.0_x64__8wekyb3d8bbwe\WindowsTerminal.exe";
        assert_eq!(display_name(Some("Windows Terminal Host"), wt), "Terminal");
        assert_eq!(display_name(None, r"C:\Windows\System32\OpenConsole.exe"), "Terminal");
        assert_eq!(display_name(Some("Windows Explorer"), r"C:\WINDOWS\Explorer.EXE"), "File Explorer");
        assert_eq!(display_name(Some("SnippingTool"), r"C:\x\SnippingTool\SnippingTool.exe"), "Snipping Tool");
        assert_eq!(shown_name("Windows Terminal Host", wt), "Terminal");
        assert_eq!(shown_name("Firefox", r"C:\Program Files\Mozilla Firefox\firefox.exe"), "Firefox");
    }

    #[test]
    fn shell_plumbing_is_hidden() {
        for exe in [
            r"C:\Windows\System32\ShellHost.exe",
            r"C:\Windows\SystemApps\Microsoft.LockApp_cw5n1h2txyewy\LockApp.exe",
            r"C:\Windows\SystemApps\MicrosoftWindows.Client.CBS_cw5n1h2txyewy\SearchHost.exe",
            r"C:\Windows\SystemApps\Microsoft.Windows.StartMenuExperienceHost_cw5n1h2txyewy\StartMenuExperienceHost.exe",
            r"C:\WINDOWS\system32\ApplicationFrameHost.exe",
        ] {
            assert!(is_hidden(exe), "{exe}");
        }
        for exe in [r"C:\Windows\explorer.exe", r"C:\Program Files\Mozilla Firefox\firefox.exe", "shellhost"] {
            assert!(!is_hidden(exe), "{exe}");
        }
    }
}
