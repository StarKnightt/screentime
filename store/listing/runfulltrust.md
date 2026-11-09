# Restricted capability: runFullTrust

Paste into Partner Center > Submission options > Restricted capabilities > "Why do you need the runFullTrust capability?":

> Screentime is a full-trust Win32 desktop app (written in Rust, packaged with MSIX) that shows the user how much time they spend in each app on their PC. To do that, a small background tracker asks Windows once a second which window is in the foreground (GetForegroundWindow, GetWindowThreadProcessId, QueryFullProcessImageName), whether the user is idle (GetLastInputInfo) and whether the session is locked, and reads each app's icon from its program file to show it in the list. These Win32 APIs are not available to an app container, so the app needs runFullTrust. The tracker starts at sign-in through the package's declared windows.startupTask, which the user can turn off in the app or in Windows Settings > Apps > Startup. The app makes no network connections, declares no other capabilities, and stores its data only in the package's own LocalState folder, which Windows removes on uninstall.

## Notes for certification

Paste into Submission options > Notes for certification:

> No account or sign-in is needed. Open "Screentime" from Start: it starts tracking at once. Use a few apps for a minute, then look at the Today tab (a 24-hour dial and a list of apps with their times). Click an app in the list to see when it was used. Settings has "Start at login" (the package's startup task, on by default), "Pause tracking", idle time, theme, and Export CSV / JSON (written to the Downloads folder). Closing the window keeps Screentime in the notification area; "Quit Screentime" in its tray menu closes the window, and "Stop tracking and quit" also stops the background tracker. The app makes no network connections. If the GitHub version of Screentime is also installed and tracking, the Store version shows a message and does not track, so time is never counted twice.
