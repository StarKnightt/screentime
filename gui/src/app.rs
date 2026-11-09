//! Application state and update logic.
//!
//! Runs as an iced *daemon*: the app lives in the tray, and the window is
//! created when shown and destroyed when closed ("hide to tray"), so a
//! closed window holds no renderer memory at all.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use chrono::NaiveDate;
use iced::keyboard::{self, key::Named, Key};
use iced::{window, Point, Size, Subscription, Task, Theme};
use screentime_core::config::{Config, ThemePref, IDLE_CHOICES};
use screentime_core::export::Format;
use screentime_core::{store, win, win_ui};

use crate::data::{Rows, Snapshot, Source};
use crate::tray::{self, External, Tray};
use crate::{logo, style, views};

pub const WINDOW_SIZE: Size = Size::new(460.0, 800.0);
/// The dial sweeps to "now" once, when the window opens.
const REVEAL: Duration = Duration::from_millis(420);
/// Focusing an app fades the others: a fast, interruptible response.
const FOCUS_FADE: Duration = Duration::from_millis(160);
const TRACKER_EXE: &str = "screentime-tracker.exe";
/// Run this exe as the windowless tracker instead of the window.
pub const TRACKER_ARG: &str = "--tracker";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Screen {
    Today,
    Week,
    Settings,
}

#[derive(Debug, Clone)]
pub enum Msg {
    External(External),
    WindowOpened(window::Id),
    WindowClosed(window::Id),
    Refresh,
    Frame,
    Show(Screen),
    SelectDay(Option<NaiveDate>),
    /// Highlight one app on the dial (by display name), or none.
    Focus(Option<String>),
    Key(keyboard::Event),
    SetStartup(bool),
    /// The first-run question: "Yes, start at login" or "Not now".
    AnswerStartup(bool),
    SetPaused(bool),
    IdleStep(i32),
    SetTheme(ThemePref),
    Export(Format),
    Reveal,
    StartTracker,
    CaptureScale(f32),
    CaptureStep(usize),
    CaptureShoot(usize),
    CaptureSaved(usize, window::Screenshot),
}

/// Scripted screenshots: `SCREENTIME_CAPTURE=<dir>` renders every screen in
/// light and dark at `SCREENTIME_CAPTURE_SCALE` (default 2) and exits.
struct Capture {
    dir: PathBuf,
    scale: f32,
    steps: Vec<(bool, Shot)>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Shot {
    Today,
    TodayFocus,
    Week,
    WeekDay,
    Settings,
}

impl Shot {
    fn name(self) -> &'static str {
        match self {
            Shot::Today => "today",
            Shot::TodayFocus => "today-focus",
            Shot::Week => "week",
            Shot::WeekDay => "week-day",
            Shot::Settings => "settings",
        }
    }
}

pub struct App {
    pub window: Option<window::Id>,
    pub screen: Screen,
    pub config: Config,
    pub startup_on: bool,
    pub system_dark: bool,
    /// Tracking in this build's own data (not the other build's tracker).
    pub tracker_running: bool,
    /// The other build (GitHub vs Store) is the one tracking on this PC.
    pub other_build: bool,
    pub snap: Snapshot,
    pub selected: Option<NaiveDate>,
    /// The app highlighted on today's dial, and the one before it (for the fade).
    pub focus: Option<String>,
    pub focus_prev: Option<String>,
    pub source: Source,
    pub export_note: Option<Result<PathBuf, String>>,
    focus_at: Option<Instant>,
    reveal_at: Option<Instant>,
    /// Extra zoom on top of Windows scaling (`SCREENTIME_ZOOM`, for recordings).
    zoom: f32,
    tray: Option<Tray>,
    scale: f32,
    dark_override: Option<bool>,
    capture: Option<Capture>,
}

pub fn run() -> iced::Result {
    let mut d = iced::daemon(App::boot, App::update, App::view)
        .title(App::title)
        .theme(App::theme)
        .subscription(App::subscription)
        .scale_factor(App::scale_factor)
        .default_font(style::TEXT)
        .antialiasing(true);
    for f in style::FONT_FILES {
        d = d.font(f);
    }
    d.run()
}

/// How to start the tracker: the separate screentime-tracker.exe when it sits
/// next to the app (a source install), else this same exe with `--tracker`
/// (the single-file release).
fn tracker_command() -> Option<(PathBuf, Option<&'static str>)> {
    let me = std::env::current_exe().ok()?;
    let sibling = me.with_file_name(TRACKER_EXE);
    Some(if sibling.exists() { (sibling, None) } else { (me, Some(TRACKER_ARG)) })
}

fn startup_line((exe, arg): &(PathBuf, Option<&str>)) -> String {
    match arg {
        Some(a) => format!("\"{}\" {a}", exe.display()),
        None => format!("\"{}\"", exe.display()),
    }
}

fn start_tracker() {
    if let Some((exe, arg)) = tracker_command() {
        use std::os::windows::process::CommandExt;
        const DETACHED_PROCESS: u32 = 0x8;
        let mut c = std::process::Command::new(exe);
        if let Some(a) = arg {
            c.arg(a);
        }
        let _ = c.creation_flags(DETACHED_PROCESS).spawn();
        std::thread::sleep(Duration::from_millis(300));
    }
}

/// First run on this PC (no database yet): start tracking, so a downloaded exe
/// works on the first double-click, and remember to ask about starting at
/// login (the GitHub build asks; it never turns that on by itself). The Store
/// build has start at login on by default, through its startup task, and the
/// user can turn it off. Skipped for screenshots and for a database given with
/// SCREENTIME_DB.
fn first_run_setup() {
    if std::env::var_os("SCREENTIME_DB").is_some() {
        return;
    }
    let first = !store::default_db_path().exists();
    // A settings file means this was already done (or never applied, for an
    // install older than it), so it happens once at most.
    let fresh_config = !screentime_core::config::path().exists();
    if win::is_packaged() {
        if fresh_config {
            let _ = Config::default().save();
            crate::startup::enable_by_default();
        }
    } else if first && fresh_config && win::get_startup().is_none() {
        let config = Config { ask_startup: true, ..Config::default() };
        let _ = config.save();
    }
    if first && !win::tracker_running() {
        start_tracker();
    }
}

/// Whether this build starts at login (Store: its startup task; GitHub: the Run key).
fn startup_on() -> bool {
    if win::is_packaged() {
        crate::startup::is_on()
    } else {
        win::get_startup().is_some()
    }
}

fn set_startup(on: bool) -> Result<(), String> {
    if win::is_packaged() {
        return crate::startup::set(on);
    }
    match (on, tracker_command()) {
        (true, Some(cmd)) => win::set_startup(&startup_line(&cmd)),
        (true, None) => Err(std::io::Error::other("cannot find the app's own path")),
        (false, _) => win::clear_startup(),
    }
    .map_err(|e| format!("Could not change startup: {e}"))
}

impl App {
    fn boot() -> (Self, Task<Msg>) {
        let capture = std::env::var_os("SCREENTIME_CAPTURE").map(|dir| {
            let scale = std::env::var("SCREENTIME_CAPTURE_SCALE").ok().and_then(|s| s.parse().ok()).unwrap_or(2.0);
            let mut steps = Vec::new();
            for dark in [false, true] {
                for shot in [Shot::Today, Shot::TodayFocus, Shot::Week, Shot::WeekDay, Shot::Settings] {
                    steps.push((dark, shot));
                }
            }
            Capture { dir: PathBuf::from(dir), scale, steps }
        });
        if capture.is_none() {
            first_run_setup();
        }
        let config = Config::load();
        let start_hidden = std::env::args().any(|a| a == "--tray");
        let mut app = App {
            window: None,
            screen: Screen::Today,
            startup_on: startup_on(),
            system_dark: win_ui::system_dark_mode(),
            tracker_running: win::tracker_running() && !win::other_build_tracking(),
            other_build: win::other_build_tracking(),
            snap: Snapshot::default(),
            selected: None,
            focus: None,
            focus_prev: None,
            source: Source::new(),
            export_note: None,
            focus_at: None,
            reveal_at: None,
            zoom: std::env::var("SCREENTIME_ZOOM").ok().and_then(|z| z.parse().ok()).filter(|z: &f32| (0.5..=3.0).contains(z)).unwrap_or(1.0),
            tray: if capture.is_none() { Tray::create(config.paused) } else { None },
            scale: 1.0,
            dark_override: None,
            capture,
            config,
        };
        let task = if start_hidden && app.capture.is_none() { Task::none() } else { app.open_window() };
        (app, task)
    }

    fn title(&self, _: window::Id) -> String {
        "Screentime".into()
    }

    pub fn dark(&self) -> bool {
        self.dark_override.unwrap_or(match self.config.theme {
            ThemePref::System => self.system_dark,
            ThemePref::Light => false,
            ThemePref::Dark => true,
        })
    }

    pub fn colors(&self) -> style::Colors {
        style::colors(self.dark())
    }

    fn theme(&self, _: window::Id) -> Theme {
        style::theme(&self.colors())
    }

    fn scale_factor(&self, _: window::Id) -> f32 {
        self.scale * self.zoom
    }

    /// The dial's opening sweep, 0..=1.
    pub fn reveal(&self) -> f32 {
        self.reveal_at.map_or(1.0, |t| (t.elapsed().as_secs_f32() / REVEAL.as_secs_f32()).min(1.0))
    }

    /// The focus fade, 0..=1.
    pub fn focus_t(&self) -> f32 {
        self.focus_at.map_or(1.0, |t| (t.elapsed().as_secs_f32() / FOCUS_FADE.as_secs_f32()).min(1.0))
    }

    /// Motion only when Windows animation effects are on (the platform's
    /// reduced-motion setting), and never in scripted captures. Without it,
    /// the dial and the focus change appear in their final state at once.
    pub fn motion(&self) -> bool {
        self.capture.is_none() && win_ui::animations_enabled()
    }

    fn animating(&self) -> bool {
        self.reveal_at.is_some() || self.focus_at.is_some()
    }

    fn set_focus(&mut self, focus: Option<String>) {
        if focus == self.focus {
            return;
        }
        self.focus_prev = std::mem::replace(&mut self.focus, focus);
        if self.motion() {
            self.focus_at = Some(Instant::now());
        }
    }

    /// Today's list, in the order it is shown, for arrow-key focus.
    fn today_names(&self) -> Vec<String> {
        Rows::new(&self.snap.day.apps).names()
    }

    fn on_key(&mut self, event: keyboard::Event) -> Task<Msg> {
        let keyboard::Event::KeyPressed { key, modifiers, .. } = event else { return Task::none() };
        let screens = [Screen::Today, Screen::Week, Screen::Settings];
        let at = screens.iter().position(|s| *s == self.screen).unwrap_or(0);
        match key.as_ref() {
            Key::Character(ch) if modifiers.command() => {
                if let Some(i) = ["1", "2", "3"].iter().position(|d| *d == ch) {
                    return self.update(Msg::Show(screens[i]));
                }
            }
            Key::Named(Named::Tab) if modifiers.command() => {
                let next = if modifiers.shift() { (at + 2) % 3 } else { (at + 1) % 3 };
                return self.update(Msg::Show(screens[next]));
            }
            Key::Named(Named::ArrowLeft) if at > 0 => return self.update(Msg::Show(screens[at - 1])),
            Key::Named(Named::ArrowRight) if at < 2 => return self.update(Msg::Show(screens[at + 1])),
            Key::Named(dir @ (Named::ArrowUp | Named::ArrowDown)) if self.screen == Screen::Today => {
                let names = self.today_names();
                if names.is_empty() {
                    return Task::none();
                }
                let pos = self.focus.as_ref().and_then(|f| names.iter().position(|n| n == f));
                let next = match (pos, dir) {
                    (None, Named::ArrowDown) => 0,
                    (None, _) => names.len() - 1,
                    (Some(i), Named::ArrowDown) => (i + 1).min(names.len() - 1),
                    (Some(i), _) => i.saturating_sub(1),
                };
                self.set_focus(Some(names[next].clone()));
            }
            Key::Named(Named::Escape) => {
                if self.focus.is_some() {
                    self.set_focus(None);
                } else if self.selected.is_some() {
                    self.selected = None;
                } else if let Some(id) = self.window {
                    // Same as closing the window: back to the tray.
                    return window::close(id);
                }
            }
            _ => {}
        }
        Task::none()
    }

    fn refresh(&mut self) {
        self.snap = self.source.load();
        self.other_build = win::other_build_tracking();
        self.tracker_running = win::tracker_running() && !self.other_build;
        self.startup_on = startup_on();
        // Settings may have been changed from the CLI.
        let fresh = Config::load();
        if fresh != self.config {
            self.config = fresh;
            if let Some(t) = &self.tray {
                t.set_paused(fresh.paused);
            }
        }
    }

    fn open_window(&mut self) -> Task<Msg> {
        if let Some(id) = self.window {
            return window::minimize(id, false).chain(window::gain_focus(id));
        }
        self.refresh();
        if self.motion() {
            self.reveal_at = Some(Instant::now());
        }
        // `SCREENTIME_WINDOW_AT=x,y` places the window (used for recordings).
        let position = std::env::var("SCREENTIME_WINDOW_AT")
            .ok()
            .and_then(|v| {
                let (x, y) = v.split_once(',')?;
                Some(window::Position::Specific(Point::new(x.trim().parse().ok()?, y.trim().parse().ok()?)))
            })
            .unwrap_or_default();
        let (id, open) = window::open(window::Settings {
            size: WINDOW_SIZE,
            position,
            min_size: Some(Size::new(400.0, 560.0)),
            icon: window::icon::from_rgba(logo::render(64), 64, 64).ok(),
            exit_on_close_request: true,
            ..window::Settings::default()
        });
        self.window = Some(id);
        open.map(Msg::WindowOpened)
    }

    fn save_config(&mut self) {
        if let Err(e) = self.config.save() {
            self.export_note = Some(Err(format!("Could not save settings: {e}")));
        }
        win::signal_reload();
        if let Some(t) = &self.tray {
            t.set_paused(self.config.paused);
        }
    }

    fn export(&mut self, format: Format) -> Result<PathBuf, String> {
        let dir = win_ui::downloads_dir().ok_or("No Downloads folder")?;
        let store = store::Store::open_read_only(&store::default_db_path())
            .map_err(|e| e.to_string())?
            .ok_or("No data yet")?;
        let records = screentime_core::export::records(&store).map_err(|e| e.to_string())?;
        let name = format!("screentime-sessions-{}.{}", self.snap.today.format("%Y-%m-%d"), format.extension());
        let path = dir.join(name);
        let mut out = std::io::BufWriter::new(std::fs::File::create(&path).map_err(|e| e.to_string())?);
        screentime_core::export::write(&mut out, format, &records).map_err(|e| e.to_string())?;
        std::io::Write::flush(&mut out).map_err(|e| e.to_string())?;
        Ok(path)
    }

    fn update(&mut self, msg: Msg) -> Task<Msg> {
        match msg {
            Msg::External(e) => match e {
                External::Open => return self.open_window(),
                External::TogglePause => {
                    self.config.paused = !self.config.paused;
                    self.save_config();
                }
                External::StopTracking => {
                    win::signal_stop();
                    return iced::exit();
                }
                External::Quit => return iced::exit(),
                External::ThemeChanged => self.system_dark = win_ui::system_dark_mode(),
            },
            Msg::WindowOpened(id) => {
                if self.capture.is_some() {
                    return window::scale_factor(id).map(Msg::CaptureScale);
                }
            }
            Msg::WindowClosed(id) => {
                if self.window == Some(id) {
                    self.window = None;
                    self.reveal_at = None;
                    self.focus_at = None;
                }
            }
            Msg::Refresh => self.refresh(),
            Msg::Frame => {
                if self.reveal() >= 1.0 {
                    self.reveal_at = None;
                }
                if self.focus_t() >= 1.0 {
                    self.focus_at = None;
                }
            }
            // Switching views is frequent, so it is instant: no animation.
            Msg::Show(screen) => {
                if screen != self.screen {
                    self.screen = screen;
                    self.export_note = None;
                }
            }
            Msg::SelectDay(day) => self.selected = day,
            Msg::Focus(name) => self.set_focus(name),
            Msg::Key(event) => return self.on_key(event),
            Msg::SetStartup(on) => {
                if let Err(e) = set_startup(on) {
                    self.export_note = Some(Err(e));
                }
                self.startup_on = startup_on();
                // Using the Settings switch answers the first-run question too.
                if self.config.ask_startup {
                    self.config.ask_startup = false;
                    self.save_config();
                }
            }
            Msg::AnswerStartup(yes) => {
                if yes {
                    // Also clears the question.
                    return self.update(Msg::SetStartup(true));
                }
                self.config.ask_startup = false;
                self.save_config();
            }
            Msg::SetPaused(p) => {
                self.config.paused = p;
                self.save_config();
            }
            Msg::IdleStep(dir) => {
                let i = IDLE_CHOICES.iter().position(|&m| m >= self.config.idle_minutes).unwrap_or(3) as i32;
                let j = (i + dir).clamp(0, IDLE_CHOICES.len() as i32 - 1) as usize;
                self.config.idle_minutes = IDLE_CHOICES[j];
                self.save_config();
            }
            Msg::SetTheme(t) => {
                self.config.theme = t;
                self.save_config();
            }
            Msg::Export(f) => self.export_note = Some(self.export(f)),
            Msg::Reveal => {
                if let Some(Ok(path)) = &self.export_note {
                    use std::os::windows::process::CommandExt;
                    let _ = std::process::Command::new("explorer.exe")
                        .raw_arg(format!("/select,\"{}\"", path.display()))
                        .spawn();
                }
            }
            Msg::StartTracker => {
                start_tracker();
                self.refresh();
            }
            Msg::CaptureScale(system) => {
                // Size the window so the content renders at exactly `scale`.
                let (Some(id), Some(cap)) = (self.window, &self.capture) else { return Task::none() };
                self.scale = cap.scale / system;
                // `resize` is in the app's own logical units, which already
                // include `self.scale`: the window keeps its logical size and
                // its physical size becomes WINDOW_SIZE * cap.scale.
                return window::resize(id, WINDOW_SIZE).chain(Task::done(Msg::CaptureStep(0)));
            }
            Msg::CaptureStep(i) => {
                let Some(cap) = &self.capture else { return Task::none() };
                let Some(&(dark, shot)) = cap.steps.get(i) else { return iced::exit() };
                self.dark_override = Some(dark);
                self.refresh();
                self.screen = match shot {
                    Shot::Today | Shot::TodayFocus => Screen::Today,
                    Shot::Week | Shot::WeekDay => Screen::Week,
                    Shot::Settings => Screen::Settings,
                };
                self.selected = (shot == Shot::WeekDay)
                    // The most recent earlier day with data, else today.
                    .then(|| self.snap.week.iter().rev().skip(1).find(|d| d.1.total > 0).or(self.snap.week.last()).map(|d| d.0))
                    .flatten();
                self.focus = (shot == Shot::TodayFocus).then(|| self.today_names().into_iter().next()).flatten();
                self.focus_prev = None;
                return Task::perform(async { std::thread::sleep(Duration::from_millis(900)) }, move |_| Msg::CaptureShoot(i));
            }
            Msg::CaptureShoot(i) => {
                if let Some(id) = self.window {
                    return window::screenshot(id).map(move |s| Msg::CaptureSaved(i, s));
                }
            }
            Msg::CaptureSaved(i, shot) => {
                let Some(cap) = &self.capture else { return Task::none() };
                let (dark, kind) = cap.steps[i];
                let theme = if dark { "dark" } else { "light" };
                let file = cap.dir.join(format!("{theme}-{}_{}x{}.rgba", kind.name(), shot.size.width, shot.size.height));
                let _ = std::fs::create_dir_all(&cap.dir);
                let _ = std::fs::write(file, &shot.rgba);
                return Task::done(Msg::CaptureStep(i + 1));
            }
        }
        Task::none()
    }

    fn view(&self, _: window::Id) -> iced::Element<'_, Msg> {
        views::view(self)
    }

    fn subscription(&self) -> Subscription<Msg> {
        let mut subs = vec![
            Subscription::run(tray::events).map(Msg::External),
            window::close_events().map(Msg::WindowClosed),
        ];
        if self.window.is_some() && self.capture.is_none() {
            subs.push(Subscription::run(tray::refresh_ticker).map(|_| Msg::Refresh));
            subs.push(keyboard::listen().map(Msg::Key));
        }
        if self.animating() {
            subs.push(window::frames().map(|_| Msg::Frame));
        }
        Subscription::batch(subs)
    }
}
