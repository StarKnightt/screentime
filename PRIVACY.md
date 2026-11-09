# Privacy policy

Screentime (sold in the Microsoft Store as "Screentime for Windows") shows how you spend time on your PC. It is built so that your data never leaves it.

Last updated: 10 November 2026.

## Nothing is sent anywhere

Screentime makes no network connections. There is no account, no sign-in, no telemetry, no analytics, no crash reporting, no ads and no update check. Nothing about you or your use of the app is sent to the developer or to anyone else.

## What is stored, on your PC only

For each app you use, Screentime stores:

- the app's name and the path of its program file (for example `C:\Program Files\Mozilla Firefox\firefox.exe`)
- the start and end time of each stretch of use

It never records window titles, web addresses (URLs), document names, keystrokes, mouse movements, screenshots, or the content of anything you do. To tell when you are idle it asks Windows how long ago your last input was, never what that input was.

Next to the data there is a small settings file (`config.ini`: idle time, pause, theme) and a `tracker.log` that is only written if something goes wrong (it may name a program path).

## Where it is stored

- Microsoft Store version: `%LOCALAPPDATA%\Packages\prasenx.ScreentimeforWindows_xcf18caytt9by\LocalState\`
- Version downloaded from GitHub: `%LOCALAPPDATA%\screentime\`

The data is in a single SQLite file, `data.db`, readable only by your Windows user account (and administrators of the PC).

## Your choices

- **See it:** everything the app knows is shown in the app.
- **Export it:** Settings > Export CSV or Export JSON saves a copy to your Downloads folder. That copy is created only when you click the button.
- **Pause it:** Settings > Pause tracking, or the tray icon. Nothing is recorded while paused.
- **Delete it:** uninstalling the Microsoft Store version deletes its folder and all of its data. For the GitHub version, quit Screentime (tray icon > Stop tracking and quit), then delete `%LOCALAPPDATA%\screentime`.

## Children

Screentime does not collect personal information from anyone, including children.

## Changes

If this policy changes, the new version will be published at this address with a new date.

## Contact

Questions: open an issue at https://github.com/StarKnightt/screentime/issues
