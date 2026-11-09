# Microsoft Store build

The Store version is the same app as the GitHub download, packaged as an MSIX. The GitHub
one-file build is unchanged: everything below only switches on when the app runs with a
package identity.

## Build

Needs Rust and the Windows SDK (`winget install Microsoft.WindowsSDK.10.0.26100`).

```powershell
powershell -File store\build.ps1                         # unsigned, for Partner Center
powershell -File store\build.ps1 -TestCert <thumbprint>  # plus a test-signed copy to install locally
```

Output: `store\out\Screentime_1.0.0.0_x64.msix` (unsigned; Microsoft signs it after
certification). `store\out\` is not committed.

What the script does: builds `screentime-app.exe` (shipped as `Screentime.exe`) and
`screentime-tracker.exe`, generates every tile and icon size from `gui/src/logo.rs` with
`store/tools/assets.rs`, builds one `resources.pri`, and packs with MakeAppx.

## How the packaged build differs

| | GitHub build | Store build |
|---|---|---|
| Data folder | `%LOCALAPPDATA%\screentime` | `%LOCALAPPDATA%\Packages\prasenx.ScreentimeforWindows_xcf18caytt9by\LocalState`, removed by Windows on uninstall |
| Start at login | Run key, asked on first launch | the package's `windows.startupTask` (`ScreentimeTracker`), on by default; the Settings toggle and Windows Settings > Apps > Startup both turn it off |
| Tracker | `Screentime.exe --tracker` | `screentime-tracker.exe` (a startup task cannot pass arguments) |
| Capabilities | n/a | `runFullTrust` only; no network |

Both builds can be installed at once without counting time twice: they share the tracker's
single-instance mutex, so only one tracker runs. The window of the build that is not tracking
says which one is, and offers Try again once the other is stopped. The Store tracker also
holds `Local\screentime.tracker.store`, and its stop and reload events end in `.store`, so each
window controls only its own tracker.

## Local test (10 November 2026, Windows 11 Pro 10.0.26300)

Test-signed with a self-signed certificate whose subject matches the manifest's Publisher,
installed with `Add-AppxPackage`, then removed along with the certificate.

- Installs with the reserved package family name `prasenx.ScreentimeforWindows_xcf18caytt9by`.
- Opens from Start. With the GitHub build already tracking, it shows the "GitHub version is
  tracking" message and does not track. The GitHub build's config and Run entry are untouched.
- First launch writes `config.ini` to LocalState and enables the startup task (state Enabled).
- After the GitHub tracker was stopped, Try again starts `screentime-tracker.exe` from the
  package and sessions are written to LocalState\data.db.
- Store tracker: 8.4 MB working set (1.3 MB private), no measurable CPU time over 40 s.
- Settings > Start at login: off sets the task to Disabled, on sets it back to Enabled.
- Uninstall removes the package, its LocalState folder and the startup task entry.

## Listing

Everything for Partner Center is in `store/listing/`; start with `checklist.md`.
