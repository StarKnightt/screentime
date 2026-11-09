//! The day as a clock: a 24-hour dial where each app owns its slices of the
//! day, plus the week's row of small dials.
//!
//! Midnight is at the top and the day runs clockwise. Canvases redraw only
//! when the hovered slice changes or while a short, user-triggered animation
//! runs (the reveal when the window opens, the 160 ms focus fade). Otherwise
//! nothing repaints.

use std::f32::consts::{FRAC_PI_2, TAU};

use chrono::{Local, NaiveDate, TimeZone};
use iced::widget::canvas::{self, path::Arc, Frame, Geometry, LineCap, Path, Stroke, Text};
use iced::widget::text::Alignment;
use iced::{alignment, mouse, Color, Event, Pixels, Point, Radians, Rectangle, Renderer, Size, Theme, Vector};
use screentime_core::time::{day_range, fmt_duration};

use crate::app::Msg;
use crate::style::{self, size, Colors, DISPLAY, MEDIUM, SEMIBOLD, TEXT};

/// Strong ease-out, cubic-bezier(0.23, 1, 0.32, 1) approximated: starts fast,
/// settles softly.
pub fn ease_out(t: f32) -> f32 {
    1.0 - (1.0 - t.clamp(0.0, 1.0)).powi(4)
}

fn text(content: impl Into<String>, at: Point, color: Color, size: f32, font: iced::Font, align_x: Alignment) -> Text {
    Text {
        content: content.into(),
        position: at,
        color,
        size: Pixels(size),
        font,
        align_x,
        align_y: alignment::Vertical::Center,
        ..Text::default()
    }
}

/// Angle of a moment of the day; midnight at the top, clockwise.
fn angle(offset: i64, length: i64) -> f32 {
    -FRAC_PI_2 + TAU * (offset as f32 / length.max(1) as f32)
}

fn polar(center: Point, r: f32, a: f32) -> Point {
    center + Vector::new(r * a.cos(), r * a.sin())
}

fn arc(f: &mut Frame, center: Point, r: f32, a0: f32, a1: f32, width: f32, color: Color) {
    let path = Path::new(|b| b.arc(Arc { center, radius: r, start_angle: Radians(a0), end_angle: Radians(a1) }));
    f.stroke(&path, Stroke::default().with_color(color).with_width(width).with_line_cap(LineCap::Butt));
}

/// Narrowest arc on the small week dials, so a short day still shows.
const MIN_ARC: f32 = 0.009; // radians, about half a degree
/// Overlap between neighbouring arcs, so no hairline of track shows between.
const SEAM: f32 = 0.0015;

/// A local clock time such as "4:12 PM".
fn clock(date: NaiveDate, offset: i64) -> String {
    let ts = day_range(date).0 + offset;
    Local.timestamp_opt(ts, 0).earliest().map(|t| t.format("%-I:%M %p").to_string()).unwrap_or_default()
}

/// One drawn piece of the dial: where it is, and which row of the list owns it.
#[derive(Debug, Clone, Copy)]
pub struct Piece {
    pub start: i64,
    pub end: i64,
    pub key: usize,
}

/// One row of the list, as the dial needs it.
#[derive(Debug, Clone)]
pub struct Key {
    pub name: String,
    pub secs: i64,
    pub color: Color,
}

// --------------------------------------------------------------------- dial

pub struct Dial {
    pub date: NaiveDate,
    pub pieces: Vec<Piece>,
    pub keys: Vec<Key>,
    pub length: i64,
    pub total: i64,
    /// Seconds since midnight when this is today.
    pub now: Option<i64>,
    /// Focused row, the row focused before it, and the fade between (0..1).
    pub focus: Option<usize>,
    pub focus_prev: Option<usize>,
    pub focus_t: f32,
    /// Opening sweep, 0..1.
    pub reveal: f32,
    /// Shown in the centre when there is no data.
    pub empty: Option<String>,
    pub c: Colors,
}

pub const DIAL_HEIGHT: f32 = 316.0;
const BAND: f32 = 22.0;

struct Geo {
    center: Point,
    /// Outer radius of the band.
    r: f32,
}

impl Dial {
    fn geo(&self, bounds: Size) -> Geo {
        let r = (bounds.width.min(bounds.height) / 2.0 - 30.0).max(40.0);
        Geo { center: Point::new(bounds.width / 2.0, bounds.height / 2.0), r }
    }

    /// The piece under the cursor, if it is on the band.
    fn piece_at(&self, p: Point, bounds: Size) -> Option<usize> {
        let g = self.geo(bounds);
        let d = p - g.center;
        let dist = (d.x * d.x + d.y * d.y).sqrt();
        if dist < g.r - BAND - 6.0 || dist > g.r + 6.0 {
            return None;
        }
        let a = (d.y.atan2(d.x) + FRAC_PI_2).rem_euclid(TAU);
        let t = (a / TAU * self.length as f32) as i64;
        // Nearest piece within a degree, so slivers can be hovered.
        let tol = self.length / 360;
        self.pieces
            .iter()
            .enumerate()
            .filter(|(_, s)| t >= s.start - tol && t < s.end + tol)
            .min_by_key(|(_, s)| if t >= s.start && t < s.end { 0 } else { (s.start - t).abs().min((t - s.end).abs()) })
            .map(|(i, _)| i)
    }

    fn alpha(&self, key: usize) -> f32 {
        let of = |focus: Option<usize>| match focus {
            None => 1.0,
            Some(f) if f == key => 1.0,
            Some(_) => 0.16,
        };
        let (a, b) = (of(self.focus_prev), of(self.focus));
        a + (b - a) * ease_out(self.focus_t)
    }
}

impl canvas::Program<Msg> for Dial {
    type State = Option<usize>;

    fn update(&self, hovered: &mut Option<usize>, event: &Event, bounds: Rectangle, cursor: mouse::Cursor) -> Option<canvas::Action<Msg>> {
        let at = cursor.position_in(bounds).and_then(|p| self.piece_at(p, bounds.size()));
        match event {
            Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)) if cursor.is_over(bounds) => {
                let key = at.map(|i| self.pieces[i].key);
                // Clicking the focused app (or empty space) clears the focus.
                let pick = key.filter(|k| Some(*k) != self.focus);
                Some(canvas::Action::publish(Msg::Focus(pick.map(|k| self.keys[k].name.clone()))).and_capture())
            }
            Event::Mouse(mouse::Event::CursorMoved { .. } | mouse::Event::CursorLeft) if at != *hovered => {
                *hovered = at;
                Some(canvas::Action::request_redraw())
            }
            _ => None,
        }
    }

    fn draw(&self, hovered: &Option<usize>, renderer: &Renderer, _: &Theme, bounds: Rectangle, _: mouse::Cursor) -> Vec<Geometry> {
        let c = self.c;
        let mut f = Frame::new(renderer, bounds.size());
        let Geo { center, r } = self.geo(bounds.size());
        let mid = r - BAND / 2.0;
        let len = self.length;

        // Track: the whole day, and the part of it already lived.
        f.stroke(&Path::circle(center, mid), Stroke::default().with_color(c.track).with_width(BAND));
        let lived_end = self.now.unwrap_or(len);
        let reveal_to = (lived_end as f32 * ease_out(self.reveal)) as i64;
        if reveal_to > 0 {
            arc(&mut f, center, mid, angle(0, len), angle(reveal_to, len), BAND, c.track_past);
        }

        // Hour ticks outside the band; the quarters carry numbers instead.
        for h in (0..24).filter(|h| h % 6 != 0) {
            let a = angle(h * len / 24, len);
            let tick = Path::line(polar(center, r + 4.0, a), polar(center, r + 8.0, a));
            f.stroke(&tick, Stroke::default().with_color(c.hairline).with_width(1.0));
        }
        for (h, label) in [(0, "0"), (6, "6"), (12, "12"), (18, "18")] {
            let a = angle(h * len / 24, len);
            f.fill_text(text(label, polar(center, r + 14.0, a), c.ink3, size::CAPTION, TEXT, Alignment::Center));
        }

        // The apps, revealed clockwise from midnight when the window opens.
        for (i, s) in self.pieces.iter().enumerate() {
            if s.start >= reveal_to {
                continue;
            }
            let end = s.end.min(reveal_to);
            // True to the clock: a ten-second visit is a hairline, not a
            // stripe. A sliver of overlap hides antialiasing seams.
            let (a0, a1) = (angle(s.start, len) - SEAM, angle(end, len) + SEAM);
            let key = &self.keys[s.key];
            let mut color = style::alpha(key.color, self.alpha(s.key));
            if Some(i) == *hovered {
                color.a = 1.0;
            }
            arc(&mut f, center, mid, a0, a1, BAND, color);
        }

        // Now: a mark across the band and a dot just outside it.
        if let Some(now) = self.now {
            let a = angle(now, len);
            f.stroke(
                &Path::line(polar(center, r - BAND - 3.0, a), polar(center, r + 2.0, a)),
                Stroke::default().with_color(c.ink).with_width(2.0).with_line_cap(LineCap::Round),
            );
            f.fill(&Path::circle(polar(center, r + 7.0, a), 3.0), c.ink);
        }

        // Centre: what is hovered, else the focused app, else the total.
        let line = |dy: f32| Point::new(center.x, center.y + dy);
        if let Some(i) = *hovered {
            let s = self.pieces[i];
            let key = &self.keys[s.key];
            f.fill_text(text(key.name.clone(), line(-34.0), c.ink2, size::BODY, MEDIUM, Alignment::Center));
            f.fill_text(text(fmt_duration(s.end - s.start), line(2.0), c.ink, size::DIAL * 0.8, DISPLAY, Alignment::Center));
            let range = format!("{} – {}", clock(self.date, s.start), clock(self.date, s.end));
            f.fill_text(text(range, line(36.0), c.ink3, size::CAPTION, TEXT, Alignment::Center));
        } else if let Some(k) = self.focus.and_then(|k| self.keys.get(k)) {
            f.fill_text(text(k.name.clone(), line(-36.0), c.ink2, size::BODY, MEDIUM, Alignment::Center));
            f.fill_text(text(fmt_duration(k.secs), line(0.0), c.ink, size::DIAL, DISPLAY, Alignment::Center));
            let pct = (k.secs * 100).checked_div(self.total).unwrap_or(0);
            let share = if pct == 0 { "Under 1% of screen time".to_string() } else { format!("{pct}% of screen time") };
            f.fill_text(text(share, line(36.0), c.ink3, size::CAPTION, TEXT, Alignment::Center));
        } else if let Some(msg) = &self.empty {
            f.fill_text(text(msg.clone(), line(0.0), c.ink2, size::HEADING, MEDIUM, Alignment::Center));
        } else {
            f.fill_text(text(fmt_duration(self.total), line(-6.0), c.ink, size::DIAL, DISPLAY, Alignment::Center));
            let label = if self.now.is_some() { "screen time today" } else { "screen time" };
            f.fill_text(text(label, line(30.0), c.ink3, size::CAPTION, TEXT, Alignment::Center));
        }
        vec![f.into_geometry()]
    }

    fn mouse_interaction(&self, hovered: &Option<usize>, _: Rectangle, _: mouse::Cursor) -> mouse::Interaction {
        if hovered.is_some() {
            mouse::Interaction::Pointer
        } else {
            mouse::Interaction::default()
        }
    }
}

// --------------------------------------------------------------- week row

/// One day in the week row.
pub struct Day {
    pub date: NaiveDate,
    pub total: i64,
    pub length: i64,
    pub now: Option<i64>,
    /// Pieces with their colours already resolved.
    pub pieces: Vec<(i64, i64, Color)>,
}

pub struct WeekRow {
    pub days: Vec<Day>,
    pub today: NaiveDate,
    pub selected: Option<NaiveDate>,
    pub c: Colors,
}

pub const WEEK_HEIGHT: f32 = 118.0;

impl WeekRow {
    fn slot_at(&self, p: Point, size: Size) -> Option<usize> {
        let n = self.days.len().max(1) as f32;
        let i = (p.x / (size.width / n)).floor();
        (i >= 0.0 && (i as usize) < self.days.len()).then_some(i as usize)
    }
}

impl canvas::Program<Msg> for WeekRow {
    type State = Option<usize>;

    fn update(&self, hovered: &mut Option<usize>, event: &Event, bounds: Rectangle, cursor: mouse::Cursor) -> Option<canvas::Action<Msg>> {
        let at = cursor.position_in(bounds).and_then(|p| self.slot_at(p, bounds.size()));
        match event {
            Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)) => {
                let date = self.days[at?].date;
                let pick = (self.selected != Some(date)).then_some(date);
                Some(canvas::Action::publish(Msg::SelectDay(pick)).and_capture())
            }
            Event::Mouse(mouse::Event::CursorMoved { .. } | mouse::Event::CursorLeft) if at != *hovered => {
                *hovered = at;
                Some(canvas::Action::request_redraw())
            }
            _ => None,
        }
    }

    fn draw(&self, hovered: &Option<usize>, renderer: &Renderer, _: &Theme, bounds: Rectangle, _: mouse::Cursor) -> Vec<Geometry> {
        let c = self.c;
        let mut f = Frame::new(renderer, bounds.size());
        let slot = bounds.width / self.days.len().max(1) as f32;
        let r = (slot / 2.0 - 9.0).clamp(14.0, 25.0);
        let band = 5.0;
        for (i, d) in self.days.iter().enumerate() {
            let x = i as f32 * slot;
            let selected = self.selected == Some(d.date);
            if Some(i) == *hovered || selected {
                let bg = if selected { c.surface } else { style::alpha(c.surface, 0.7) };
                f.fill(&Path::rounded_rectangle(Point::new(x + 2.0, 0.0), Size::new(slot - 4.0, WEEK_HEIGHT), 12.0.into()), bg);
            }
            let center = Point::new(x + slot / 2.0, 12.0 + r);
            let mid = r - band / 2.0;
            // A day with no data is the quiet track; a lived day is darker.
            let base = if d.total > 0 { c.track_past } else { c.track };
            f.stroke(&Path::circle(center, mid), Stroke::default().with_color(base).with_width(band));
            if let Some(now) = d.now {
                // The rest of today hasn't happened yet.
                arc(&mut f, center, mid, angle(now, d.length), angle(d.length, d.length), band, c.track);
            }
            for &(s, e, color) in &d.pieces {
                let (a0, a1) = (angle(s, d.length), angle(e, d.length));
                arc(&mut f, center, mid, a0, a1.max(a0 + MIN_ARC * 2.0), band, color);
            }

            let label = if d.date == self.today { "Today".to_string() } else { d.date.format("%a").to_string() };
            let label_y = 12.0 + 2.0 * r + 16.0;
            let (lc, lf) = if selected { (c.ink, SEMIBOLD) } else { (c.ink2, MEDIUM) };
            f.fill_text(text(label, Point::new(center.x, label_y), lc, size::CAPTION, lf, Alignment::Center));
            // Empty days show no total at all, not "0s".
            if d.total > 0 {
                f.fill_text(text(fmt_duration(d.total), Point::new(center.x, label_y + 18.0), c.ink3, size::CAPTION, TEXT, Alignment::Center));
            }
        }
        vec![f.into_geometry()]
    }

    fn mouse_interaction(&self, hovered: &Option<usize>, _: Rectangle, _: mouse::Cursor) -> mouse::Interaction {
        if hovered.is_some() {
            mouse::Interaction::Pointer
        } else {
            mouse::Interaction::default()
        }
    }
}
