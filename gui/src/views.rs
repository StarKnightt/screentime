//! The three views: Today (the day dial), Week, Settings.
//!
//! No cards: groups are made by space (at least twice the gap inside a
//! group), headings are real text, and the only strong colour is each app's.

use iced::alignment::{Horizontal, Vertical};
use iced::widget::{button, canvas, column, container, image, row, scrollable, space, text, toggler, Column};
use iced::{Element, Fill, Length, Padding};
use screentime_core::config::ThemePref;
use screentime_core::export::Format;
use screentime_core::report::{DayReport, TIMELINE_MERGE_SECS};
use screentime_core::store;
use screentime_core::time::fmt_duration;

use crate::app::{App, Msg, Screen};
use crate::chart::{self, Dial, Key, Piece, WeekRow};
use crate::data::{Rows, OTHER};
use crate::style::{self, size, Colors, DISPLAY, GAP, MARGIN, MEDIUM, SECTION, SEMIBOLD};

type El<'a> = Element<'a, Msg>;

/// Rows carry their own 10 px inset (for the hover tint); everything else
/// gets it from this, so all text starts on one edge.
const ROW_INSET: f32 = 10.0;

pub fn view(app: &App) -> El<'_> {
    let c = app.colors();
    let body = match app.screen {
        Screen::Today => today(app, c),
        Screen::Week => week(app, c),
        Screen::Settings => settings(app, c),
    };
    let side = MARGIN - ROW_INSET;
    let content = scrollable(container(body).padding(Padding { top: 8.0, right: side, bottom: MARGIN, left: side }))
        .style(style::scroll(c))
        .height(Fill);
    container(column![header(app, c), content]).style(style::window(c)).width(Fill).height(Fill).into()
}

/// Wrap non-row content so it lines up with the text inside rows.
fn inset<'a>(e: impl Into<El<'a>>) -> El<'a> {
    container(e).padding([0.0, ROW_INSET]).width(Fill).into()
}

// ------------------------------------------------------------------- header

fn header(app: &App, c: Colors) -> El<'_> {
    // Text tabs with an ink underline: navigation, not a setting.
    let tab = |label: &'static str, s: Screen, width: f32| -> El {
        let active = app.screen == s;
        column![
            button(text(label).size(size::BODY).font(if active { SEMIBOLD } else { MEDIUM }).width(Fill).align_x(Horizontal::Center))
                .width(width)
                .padding([10, 0])
                .style(style::tab(c, active))
                .on_press(Msg::Show(s)),
            container(space()).width(width - 16.0).height(2).style(style::fill(if active { c.ink } else { style::alpha(c.ink, 0.0) }, 1.0)),
        ]
        .align_x(Horizontal::Center)
        .into()
    };
    let (mark, label): (El, &str) = match (app.tracker_running, app.config.paused) {
        (true, false) => (container(space()).width(7).height(7).style(style::fill(c.ink2, 4.0)).into(), "Tracking"),
        (true, true) => (container(space()).width(8).height(8).style(style::ring(c.ink2)).into(), "Paused"),
        (false, _) => (container(space()).width(8).height(8).style(style::ring(c.ink3)).into(), "Not tracking"),
    };
    let status = row![mark, text(label).size(size::CAPTION).color(c.ink2)].spacing(7).align_y(Vertical::Center);
    row![
        tab("Today", Screen::Today, 70.0),
        tab("Week", Screen::Week, 64.0),
        tab("Settings", Screen::Settings, 84.0),
        space::horizontal(),
        status
    ]
    .spacing(2)
    .align_y(Vertical::Center)
    .padding(Padding { top: 10.0, right: MARGIN, bottom: 6.0, left: MARGIN - 14.0 })
    .into()
}

// ------------------------------------------------------------------ pieces

fn caption<'a>(s: impl text::IntoFragment<'a>, c: Colors) -> El<'a> {
    text(s).size(size::CAPTION).color(c.ink3).into()
}

fn heading<'a>(s: impl text::IntoFragment<'a>, c: Colors) -> El<'a> {
    text(s).size(size::HEADING).font(SEMIBOLD).color(c.ink).into()
}

fn icon<'a>(app: &App, exe: &str, name: &str, c: Colors) -> El<'a> {
    let slot = |e: El<'a>| container(e).width(28).height(28).center(28).into();
    if name == OTHER {
        let dots = text("···").size(13).font(SEMIBOLD).color(c.ink3);
        return slot(container(dots).width(24).height(24).center(24).style(style::fill(c.surface, 7.0)).into());
    }
    match app.source.icon(exe) {
        Some(h) => slot(image(h).width(24).height(24).into()),
        None => {
            let letter: String = name.chars().next().map(|ch| ch.to_uppercase().collect()).unwrap_or_default();
            slot(container(text(letter).size(12).font(SEMIBOLD).color(c.ink2)).width(24).height(24).center(24).style(style::fill(c.surface, 7.0)).into())
        }
    }
}

/// A quiet proportional bar in the app's own colour: the dial's key.
fn thin_bar<'a>(frac: f32, color: iced::Color) -> El<'a> {
    let filled = (frac.clamp(0.0, 1.0) * 1000.0).round().max(6.0) as u16;
    row![
        container(space()).width(Length::FillPortion(filled)).height(3).style(style::fill(color, 2.0)),
        space().width(Length::FillPortion(1000u16.saturating_sub(filled).max(1))),
    ]
    .into()
}

/// The app list. `focusable` rows select the app on the dial when clicked.
fn app_rows<'a>(app: &'a App, rows: &Rows, focusable: bool, c: Colors) -> El<'a> {
    let max = rows.apps.first().map_or(rows.other, |u| u.secs.max(rows.other)).max(1);
    let mut items: Vec<(String, String, i64)> = rows.apps.iter().map(|u| (u.name.clone(), u.exe_path.clone(), u.secs)).collect();
    if rows.other > 0 {
        items.push((OTHER.into(), String::new(), rows.other));
    }
    let mut col = Column::new().spacing(2);
    for (name, exe, secs) in items {
        let color = app.snap.color(&name, c.dark);
        let focused = focusable && app.focus.as_deref() == Some(name.as_str());
        let line = row![
            icon(app, &exe, &name, c),
            column![
                row![
                    text(name.clone()).size(size::BODY).font(MEDIUM).color(c.ink).wrapping(text::Wrapping::None),
                    space::horizontal(),
                    text(fmt_duration(secs)).size(size::BODY).font(MEDIUM).color(if focused || !focusable { c.ink } else { c.ink2 }),
                ]
                .align_y(Vertical::Center),
                thin_bar(secs as f32 / max as f32, color),
            ]
            .spacing(6)
            .width(Fill),
        ]
        .spacing(12)
        .align_y(Vertical::Center);
        let b = button(line).padding([9.0, ROW_INSET]).width(Fill).style(style::row(c, focused));
        col = col.push(if focusable {
            let pick = (!focused).then_some(name);
            b.on_press(Msg::Focus(pick))
        } else {
            b
        });
    }
    col.into()
}

fn banner<'a>(app: &App, c: Colors) -> Option<El<'a>> {
    let (msg, action, label) = if !app.tracker_running {
        ("Screentime isn't recording right now.", Msg::StartTracker, "Start tracking")
    } else if app.config.paused {
        ("Tracking is paused. Nothing new is recorded.", Msg::SetPaused(false), "Resume")
    } else {
        return None;
    };
    Some(
        container(
            row![
                text(msg).size(size::CAPTION).color(c.ink).width(Fill),
                button(text(label).size(size::CAPTION).font(MEDIUM)).padding([6, 12]).style(style::outline(c)).on_press(action),
            ]
            .spacing(12)
            .align_y(Vertical::Center),
        )
        .padding([10, 14])
        .width(Fill)
        .style(style::fill(c.surface, 10.0))
        .into(),
    )
}

/// Pieces and keys for one day's dial, keyed to the rows of its list.
fn dial_parts(app: &App, day: &DayReport, rows: &Rows, c: Colors) -> (Vec<Piece>, Vec<Key>) {
    let names = rows.names();
    let keys = names.iter().map(|n| Key { name: n.clone(), secs: rows.secs(n), color: app.snap.color(n, c.dark) }).collect();
    let mut pieces: Vec<Piece> = Vec::new();
    for s in &day.timeline {
        let k = rows.key(&s.name);
        let Some(key) = names.iter().position(|n| n == k) else { continue };
        // Neighbours that ended up under one key (two small apps in Other)
        // are one arc.
        match pieces.last_mut() {
            Some(p) if p.key == key && s.start - p.end <= TIMELINE_MERGE_SECS => p.end = p.end.max(s.end),
            _ => pieces.push(Piece { start: s.start, end: s.end, key }),
        }
    }
    (pieces, keys)
}

// -------------------------------------------------------------------- today

fn today<'a>(app: &'a App, c: Colors) -> El<'a> {
    let s = &app.snap;
    let mut col = Column::new();
    if let Some(b) = banner(app, c) {
        col = col.push(inset(b)).push(space().height(GAP));
    }
    if let Some(e) = &s.error {
        col = col.push(inset(column![text("Couldn't read your data").size(size::BODY).font(MEDIUM).color(c.ink), caption(e.clone(), c)].spacing(4)));
    }
    col = col.push(inset(text(s.today.format("%A, %-d %B").to_string()).size(size::CAPTION).font(MEDIUM).color(c.ink2)));

    let rows = Rows::new(&s.day.apps);
    let (pieces, keys) = dial_parts(app, &s.day, &rows, c);
    let focus = app.focus.as_ref().and_then(|f| keys.iter().position(|k| &k.name == f));
    let focus_prev = app.focus_prev.as_ref().and_then(|f| keys.iter().position(|k| &k.name == f));
    let empty = match (s.has_db, s.day.total) {
        (false, _) => Some("No data yet".to_string()),
        (true, 0) => Some("Nothing yet today".to_string()),
        _ => None,
    };
    let has_data = empty.is_none();
    col = col.push(
        canvas(Dial {
            date: s.today,
            pieces,
            keys,
            length: s.day.length.max(1),
            total: s.day.total,
            now: Some(s.now_offset),
            focus,
            focus_prev,
            focus_t: app.focus_t(),
            reveal: app.reveal(),
            empty,
            c,
        })
        .width(Fill)
        .height(chart::DIAL_HEIGHT),
    );

    // Compared with yesterday by this time: only when there is something to say.
    let d = s.day.total - s.yesterday_so_far;
    let compare = match (s.day.total, s.yesterday_so_far) {
        (0, _) | (_, 0) => None,
        _ if d.abs() < 60 => Some("About the same as yesterday by now".to_string()),
        _ if d > 0 => Some(format!("{} more than yesterday by now", fmt_duration(d))),
        _ => Some(format!("{} less than yesterday by now", fmt_duration(-d))),
    };
    if let Some(msg) = compare {
        col = col.push(text(msg).size(size::CAPTION).color(c.ink3).width(Fill).align_x(Horizontal::Center));
    }
    col = col.push(space().height(SECTION - 8.0));

    if has_data {
        col = col.push(app_rows(app, &rows, true, c));
    } else if !s.has_db {
        let mut intro = column![
            text("Your day goes here").size(size::HEADING).font(SEMIBOLD).color(c.ink),
            text("Screentime notes which app is in front while you use this PC. Each one fills its part of the dial, starting at midnight at the top and going clockwise.")
                .size(size::BODY)
                .color(c.ink2),
        ]
        .spacing(6);
        if !app.tracker_running {
            intro = intro.push(space().height(6)).push(
                button(text("Start tracking").size(size::BODY).font(MEDIUM)).padding([8, 14]).style(style::outline(c)).on_press(Msg::StartTracker),
            );
        }
        col = col.push(inset(intro));
    } else {
        col = col.push(inset(
            column![
                text("Nothing recorded yet today").size(size::HEADING).font(SEMIBOLD).color(c.ink),
                text("Apps show up here within half a minute of using them.").size(size::BODY).color(c.ink2),
            ]
            .spacing(6),
        ));
    }
    col.into()
}

// --------------------------------------------------------------------- week

fn week<'a>(app: &'a App, c: Colors) -> El<'a> {
    let s = &app.snap;
    let mut col = Column::new();
    if let Some(b) = banner(app, c) {
        col = col.push(inset(b)).push(space().height(GAP));
    }
    let days = s.days_with_data();
    let summary = match days {
        0 => "Days fill in as you use your PC.".to_string(),
        1 => "Your first day with Screentime. The rest of the week fills in as you go.".to_string(),
        n => format!("{} a day on average, over {n} days", fmt_duration(s.week_average())),
    };
    col = col.push(inset(
        column![
            text("Last 7 days").size(size::CAPTION).font(MEDIUM).color(c.ink2),
            text(fmt_duration(s.week_total())).font(DISPLAY).size(size::TITLE).color(c.ink),
            caption(summary, c),
        ]
        .spacing(4),
    ));
    col = col.push(space().height(SECTION - 8.0));

    let week_days = s
        .week
        .iter()
        .map(|(date, d)| {
            let rows = Rows::new(&d.apps);
            chart::Day {
                date: *date,
                total: d.total,
                length: d.length.max(1),
                now: (*date == s.today).then_some(s.now_offset),
                pieces: d.timeline.iter().map(|p| (p.start, p.end, s.color(rows.key(&p.name), c.dark))).collect(),
            }
        })
        .collect();
    col = col.push(
        canvas(WeekRow { days: week_days, today: s.today, selected: app.selected, c })
            .width(Fill)
            .height(chart::WEEK_HEIGHT),
    );
    col = col.push(space().height(SECTION - 4.0));

    match app.selected.and_then(|d| s.day(d).map(|r| (d, r))) {
        Some((date, day)) => {
            let name = if date == s.today { "Today".to_string() } else { date.format("%A, %-d %B").to_string() };
            col = col.push(inset(
                row![
                    column![heading(name, c), caption(format!("{} of screen time", fmt_duration(day.total)), c)].spacing(2),
                    space::horizontal(),
                    button(text("Back to the week").size(size::CAPTION).font(MEDIUM)).padding([7, 12]).style(style::outline(c)).on_press(Msg::SelectDay(None)),
                ]
                .align_y(Vertical::Center),
            ));
            col = col.push(space().height(GAP));
            col = if day.apps.is_empty() {
                col.push(inset(text("No screen time on this day.").size(size::BODY).color(c.ink2)))
            } else {
                col.push(app_rows(app, &Rows::new(&day.apps), false, c))
            };
        }
        None => {
            col = col.push(inset(heading("Most used", c))).push(space().height(GAP - 4.0));
            col = if s.week_apps.is_empty() {
                col.push(inset(text("Nothing recorded in the last 7 days.").size(size::BODY).color(c.ink2)))
            } else {
                let mut rows = Rows::new(&s.week_apps);
                rows.apps.truncate(8);
                rows.other = 0; // a top list needs no remainder row
                col.push(app_rows(app, &rows, false, c))
            };
        }
    }
    col.into()
}

// ----------------------------------------------------------------- settings

fn setting<'a>(label: &'a str, detail: &'a str, control: impl Into<El<'a>>, c: Colors) -> El<'a> {
    let mut text_col = column![text(label).size(size::BODY).font(MEDIUM).color(c.ink)].spacing(3).width(Fill);
    if !detail.is_empty() {
        text_col = text_col.push(caption(detail, c));
    }
    row![text_col, control.into()].spacing(20).align_y(Vertical::Center).into()
}

/// A titled group: the heading sits close to its items, groups far apart.
fn group<'a>(title: &'a str, items: Vec<El<'a>>, c: Colors) -> El<'a> {
    inset(column![heading(title, c), Column::with_children(items).spacing(18)].spacing(GAP + 2.0))
}

fn settings<'a>(app: &'a App, c: Colors) -> El<'a> {
    let cfg = &app.config;
    let step = |label: &'static str, dir: i32| {
        button(text(label).size(15).width(Fill).align_x(Horizontal::Center)).width(32).padding([4, 0]).style(style::outline(c)).on_press(Msg::IdleStep(dir))
    };
    let stepper = row![
        step("−", -1),
        text(format!("{} min", cfg.idle_minutes)).size(size::BODY).font(MEDIUM).color(c.ink).width(56).align_x(Horizontal::Center),
        step("+", 1),
    ]
    .align_y(Vertical::Center);

    let tracking = group(
        "Tracking",
        vec![
            setting("Start at login", "Records from the moment you sign in.", toggler(app.startup_on).on_toggle(Msg::SetStartup).size(20).style(style::toggle(c)), c),
            setting("Pause tracking", "Nothing is recorded while paused.", toggler(cfg.paused).on_toggle(Msg::SetPaused).size(20).style(style::toggle(c)), c),
            setting("Idle after", "With no input for this long, the clock stops and the wait isn't counted.", stepper, c),
        ],
        c,
    );

    let themes = row(ThemePref::ALL.into_iter().map(|t| {
        let active = cfg.theme == t;
        button(text(t.label()).size(size::CAPTION).font(if active { SEMIBOLD } else { MEDIUM }).width(Fill).align_x(Horizontal::Center))
            .width(62)
            .padding([6, 0])
            .style(style::quiet(c, active))
            .on_press(Msg::SetTheme(t))
            .into()
    }))
    .spacing(2);
    let appearance = group("Appearance", vec![setting("Theme", "System follows Windows.", themes, c)], c);

    let export_buttons = row![
        button(text("Export CSV").size(size::CAPTION).font(MEDIUM)).padding([7, 12]).style(style::outline(c)).on_press(Msg::Export(Format::Csv)),
        button(text("Export JSON").size(size::CAPTION).font(MEDIUM)).padding([7, 12]).style(style::outline(c)).on_press(Msg::Export(Format::Json)),
    ]
    .spacing(8);
    let mut data: Vec<El> = vec![column![
        setting("Your sessions", "Every session, saved to your Downloads folder.", space().width(0), c),
        export_buttons,
    ]
    .spacing(10)
    .into()];
    match &app.export_note {
        Some(Ok(path)) => {
            let file = path.file_name().map(|f| f.to_string_lossy().into_owned()).unwrap_or_default();
            data.push(
                row![
                    text(format!("Saved {file}")).size(size::CAPTION).color(c.ink2).width(Fill),
                    button(text("Show in folder").size(size::CAPTION).font(MEDIUM)).padding([5, 8]).style(style::quiet(c, false)).on_press(Msg::Reveal),
                ]
                .align_y(Vertical::Center)
                .into(),
            );
        }
        Some(Err(e)) => data.push(text(e.clone()).size(size::CAPTION).color(c.ink).into()),
        None => {}
    }
    data.push(
        column![
            text("Stored only on this PC").size(size::BODY).font(MEDIUM).color(c.ink),
            caption(store::default_db_path().display().to_string(), c),
        ]
        .spacing(3)
        .into(),
    );
    let data = group("Your data", data, c);

    let about = inset(caption(format!("Screentime {}", env!("CARGO_PKG_VERSION")), c));
    column![tracking, space().height(SECTION), appearance, space().height(SECTION), data, space().height(SECTION), about].into()
}
