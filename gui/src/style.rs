//! Look: warm neutral paper and ink. App colours (from their icons) are the
//! only strong colour; controls and selection are drawn in ink.
//!
//! Type is Bricolage Grotesque (OFL-1.1): text cuts at a small optical size,
//! one display cut for the dial. Tabular figures are baked into the default
//! digits, so every duration lines up. See assets/fonts.

use iced::font::{Family, Weight};
use iced::widget::{button, container, scrollable, toggler};
use iced::{border, Background, Border, Color, Font, Shadow, Theme};

pub const FONT_FILES: [&[u8]; 4] = [
    include_bytes!("../assets/fonts/Bricolage-Regular.ttf"),
    include_bytes!("../assets/fonts/Bricolage-Medium.ttf"),
    include_bytes!("../assets/fonts/Bricolage-SemiBold.ttf"),
    include_bytes!("../assets/fonts/BricolageDisplay-SemiBold.ttf"),
];

pub const TEXT: Font = Font::with_name("Bricolage Grotesque");
pub const MEDIUM: Font = Font { weight: Weight::Medium, ..TEXT };
pub const SEMIBOLD: Font = Font { weight: Weight::Semibold, ..TEXT };
pub const DISPLAY: Font = Font { family: Family::Name("Bricolage Grotesque Display"), weight: Weight::Semibold, ..Font::DEFAULT };

/// Type scale (logical px). One size per role.
pub mod size {
    pub const DIAL: f32 = 44.0;
    pub const TITLE: f32 = 34.0;
    pub const HEADING: f32 = 16.0;
    pub const BODY: f32 = 14.0;
    pub const CAPTION: f32 = 12.5;
}

/// Spacing scale (logical px): inside a group, between groups, page margin.
pub const GAP: f32 = 12.0;
pub const SECTION: f32 = 32.0;
pub const MARGIN: f32 = 24.0;

#[derive(Debug, Clone, Copy)]
pub struct Colors {
    pub dark: bool,
    pub bg: Color,
    /// Hover and selected rows.
    pub surface: Color,
    /// Pressed.
    pub surface_strong: Color,
    /// Separators and outlines.
    pub hairline: Color,
    pub ink: Color,
    pub ink2: Color,
    /// Tertiary text; still 4.5:1 on `bg` and `surface`.
    pub ink3: Color,
    /// The dial's track: the rest of the day, and the part already lived.
    pub track: Color,
    pub track_past: Color,
}

const fn rgb(hex: u32) -> Color {
    Color::from_rgb(
        ((hex >> 16) & 0xFF) as f32 / 255.0,
        ((hex >> 8) & 0xFF) as f32 / 255.0,
        (hex & 0xFF) as f32 / 255.0,
    )
}

pub const LIGHT: Colors = Colors {
    dark: false,
    bg: rgb(0xF4F3F0),
    surface: rgb(0xEAE8E3),
    surface_strong: rgb(0xE0DDD7),
    hairline: rgb(0xDDD9D2),
    ink: rgb(0x1C1A17),
    ink2: rgb(0x57534D),
    ink3: rgb(0x6B665F),
    track: rgb(0xEAE7E2),
    track_past: rgb(0xDCD8D1),
};

pub const DARK: Colors = Colors {
    dark: true,
    bg: rgb(0x181716),
    surface: rgb(0x23211F),
    surface_strong: rgb(0x2D2B28),
    hairline: rgb(0x33302C),
    ink: rgb(0xEEECE7),
    ink2: rgb(0xB0ABA3),
    ink3: rgb(0x948F87),
    track: rgb(0x211F1D),
    track_past: rgb(0x2F2C29),
};

pub fn colors(dark: bool) -> Colors {
    if dark {
        DARK
    } else {
        LIGHT
    }
}

pub fn theme(c: &Colors) -> Theme {
    Theme::custom(
        if c.dark { "Screentime Dark" } else { "Screentime Light" }.to_string(),
        iced::theme::Palette {
            background: c.bg,
            text: c.ink,
            primary: c.ink,
            success: c.ink,
            warning: c.ink,
            danger: c.ink,
        },
    )
}

pub const fn alpha(c: Color, a: f32) -> Color {
    Color { a, ..c }
}

// ------------------------------------------------------------------ styles

pub fn window(c: Colors) -> impl Fn(&Theme) -> container::Style {
    move |_| container::Style { background: Some(c.bg.into()), text_color: Some(c.ink), ..Default::default() }
}

pub fn fill(color: Color, radius: f32) -> impl Fn(&Theme) -> container::Style {
    move |_| container::Style { background: Some(color.into()), border: border::rounded(radius), ..Default::default() }
}

/// Hollow ring (the "paused" status mark).
pub fn ring(color: Color) -> impl Fn(&Theme) -> container::Style {
    move |_| container::Style {
        border: Border { radius: 99.0.into(), width: 1.5, color },
        ..Default::default()
    }
}

fn pressable(c: Colors, status: button::Status, base: Option<Color>) -> Option<Background> {
    match status {
        button::Status::Hovered => Some(c.surface.into()),
        button::Status::Pressed => Some(c.surface_strong.into()),
        _ => base.map(Background::Color),
    }
}

/// A navigation tab: text only, ink when active. The underline is drawn by
/// the view so it can sit on the header's baseline.
pub fn tab(c: Colors, active: bool) -> impl Fn(&Theme, button::Status) -> button::Style {
    move |_, status| button::Style {
        background: None,
        text_color: if active || matches!(status, button::Status::Hovered | button::Status::Pressed) { c.ink } else { c.ink3 },
        border: Border::default(),
        shadow: Shadow::default(),
        snap: true,
    }
}

/// A full-width list row: tinted on hover, a step darker when pressed, and
/// tinted while selected.
pub fn row(c: Colors, selected: bool) -> impl Fn(&Theme, button::Status) -> button::Style {
    move |_, status| button::Style {
        background: pressable(c, status, selected.then_some(c.surface)),
        text_color: c.ink,
        border: border::rounded(10.0),
        shadow: Shadow::default(),
        snap: true,
    }
}

/// Plain text button (back links, steppers, choices).
pub fn quiet(c: Colors, active: bool) -> impl Fn(&Theme, button::Status) -> button::Style {
    move |_, status| button::Style {
        background: pressable(c, status, active.then_some(c.surface)),
        text_color: match status {
            button::Status::Disabled => c.ink3,
            _ if active => c.ink,
            _ => c.ink2,
        },
        border: border::rounded(8.0),
        shadow: Shadow::default(),
        snap: true,
    }
}

/// Outlined button for real actions (Export, Start tracking).
pub fn outline(c: Colors) -> impl Fn(&Theme, button::Status) -> button::Style {
    move |_, status| button::Style {
        background: pressable(c, status, None),
        text_color: c.ink,
        border: Border { radius: 8.0.into(), width: 1.0, color: c.hairline },
        shadow: Shadow::default(),
        snap: true,
    }
}

pub fn toggle(c: Colors) -> impl Fn(&Theme, toggler::Status) -> toggler::Style {
    move |_, status| {
        let (on, hovered) = match status {
            toggler::Status::Active { is_toggled } => (is_toggled, false),
            toggler::Status::Hovered { is_toggled } => (is_toggled, true),
            toggler::Status::Disabled { is_toggled } => (is_toggled, false),
        };
        let off = if hovered { c.surface_strong } else { c.hairline };
        toggler::Style {
            background: if on { c.ink } else { off }.into(),
            background_border_width: 0.0,
            background_border_color: Color::TRANSPARENT,
            foreground: if on { c.bg } else { rgb(0xFFFFFF) }.into(),
            foreground_border_width: 0.0,
            foreground_border_color: Color::TRANSPARENT,
            text_color: Some(c.ink),
            border_radius: None,
            padding_ratio: 0.12,
        }
    }
}

pub fn scroll(c: Colors) -> impl Fn(&Theme, scrollable::Status) -> scrollable::Style {
    move |theme, status| {
        let mut s = scrollable::default(theme, status);
        let rail = scrollable::Rail {
            background: None,
            border: border::rounded(4.0),
            scroller: scrollable::Scroller {
                background: match status {
                    scrollable::Status::Active { .. } => Color::TRANSPARENT,
                    _ => alpha(c.ink3, 0.5),
                }
                .into(),
                border: border::rounded(4.0),
            },
        };
        s.vertical_rail = rail;
        s.horizontal_rail = rail;
        s.container = container::Style::default();
        s
    }
}
