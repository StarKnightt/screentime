//! Tray icon and the one subscription that carries every outside event into
//! the app: tray clicks and menu picks, Windows theme changes, a second
//! launch asking us to show the window, and the 30 s refresh ticker. All of
//! them block in the kernel or call back; nothing polls.

use std::time::Duration;

use iced::futures::channel::mpsc::Sender;
use iced::futures::{SinkExt, Stream};
use iced::stream;
use screentime_core::{win, win_ui};
use tray_icon::menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem};
use tray_icon::{MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent};

use crate::logo;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum External {
    Open,
    TogglePause,
    StopTracking,
    Quit,
    ThemeChanged,
}

pub struct Tray {
    _icon: TrayIcon,
    pause: MenuItem,
}

impl Tray {
    /// Must run on the main (event loop) thread.
    pub fn create(paused: bool) -> Option<Self> {
        let pause = MenuItem::with_id("pause", pause_label(paused), true, None);
        let menu = Menu::new();
        menu.append_items(&[
            &MenuItem::with_id("open", "Open Screentime", true, None),
            &pause,
            &PredefinedMenuItem::separator(),
            &MenuItem::with_id("stop", "Stop tracking and quit", true, None),
            &MenuItem::with_id("quit", "Quit Screentime", true, None),
        ])
        .ok()?;
        let icon = tray_icon::Icon::from_rgba(logo::render(32), 32, 32).ok()?;
        let tray = TrayIconBuilder::new()
            .with_icon(icon)
            .with_tooltip("Screentime")
            .with_menu(Box::new(menu))
            .with_menu_on_left_click(false)
            .build()
            .ok()?;
        Some(Self { _icon: tray, pause })
    }

    pub fn set_paused(&self, paused: bool) {
        self.pause.set_text(pause_label(paused));
    }
}

fn pause_label(paused: bool) -> &'static str {
    if paused {
        "Resume tracking"
    } else {
        "Pause tracking"
    }
}

fn send(tx: &Sender<External>, e: External) {
    let _ = tx.clone().try_send(e);
}

/// Outside events. Started once for the app's lifetime.
pub fn events() -> impl Stream<Item = External> {
    stream::channel(32, async |tx: Sender<External>| {
        let t = tx.clone();
        TrayIconEvent::set_event_handler(Some(move |e: TrayIconEvent| {
            if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } = e {
                send(&t, External::Open);
            }
        }));
        let t = tx.clone();
        MenuEvent::set_event_handler(Some(move |e: MenuEvent| {
            let ev = match e.id.0.as_str() {
                "open" => External::Open,
                "pause" => External::TogglePause,
                "stop" => External::StopTracking,
                "quit" => External::Quit,
                _ => return,
            };
            send(&t, ev);
        }));
        let t = tx.clone();
        win_ui::watch_theme(move || send(&t, External::ThemeChanged));
        // A second launch fires this event instead of opening another app.
        if let Some(show) = win::NamedEvent::create(&crate::instance_name(win::names::APP_SHOW)) {
            let t = tx.clone();
            std::thread::spawn(move || {
                while show.wait(u32::MAX) {
                    send(&t, External::Open);
                }
            });
        }
        std::future::pending::<()>().await;
    })
}

/// One tick every 30 s while subscribed (only while the window is open).
pub fn refresh_ticker() -> impl Stream<Item = ()> {
    stream::channel(1, async |mut tx: Sender<()>| loop {
        // A detached sleeper thread per tick keeps the executor free; the
        // stream (and with it the loop) ends when the window closes.
        let (done_tx, done_rx) = iced::futures::channel::oneshot::channel();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_secs(30));
            let _ = done_tx.send(());
        });
        if done_rx.await.is_err() || tx.send(()).await.is_err() {
            break;
        }
    })
}
