//! User settings shared by the tracker, the CLI and the GUI.
//!
//! A small `key = value` file next to the database. The GUI and CLI write it;
//! the tracker reads it at start, when signalled, and every flush.

use std::path::PathBuf;

use crate::store::data_dir;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ThemePref {
    #[default]
    System,
    Light,
    Dark,
}

impl ThemePref {
    pub const ALL: [ThemePref; 3] = [ThemePref::System, ThemePref::Light, ThemePref::Dark];

    pub fn as_str(self) -> &'static str {
        match self {
            ThemePref::System => "system",
            ThemePref::Light => "light",
            ThemePref::Dark => "dark",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            ThemePref::System => "System",
            ThemePref::Light => "Light",
            ThemePref::Dark => "Dark",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Config {
    /// Minutes without input before tracking stops (and that wait is removed).
    pub idle_minutes: u32,
    /// Tracking paused by the user.
    pub paused: bool,
    pub theme: ThemePref,
    /// The first-run "start at login?" question is still unanswered. Only a
    /// fresh install sets it; an existing config without the key never asks.
    pub ask_startup: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self { idle_minutes: 5, paused: false, theme: ThemePref::System, ask_startup: false }
    }
}

pub const IDLE_CHOICES: [u32; 7] = [1, 2, 3, 5, 10, 15, 30];

pub fn path() -> PathBuf {
    data_dir().join("config.ini")
}

impl Config {
    pub fn idle_secs(&self) -> u64 {
        u64::from(self.idle_minutes) * 60
    }

    /// Parse leniently: unknown keys and bad values fall back to defaults.
    pub fn parse(text: &str) -> Self {
        let mut c = Config::default();
        for line in text.lines() {
            let line = line.trim();
            if line.starts_with(['#', ';']) {
                continue;
            }
            let Some((k, v)) = line.split_once('=') else { continue };
            let v = v.trim();
            match k.trim() {
                "idle_minutes" => {
                    if let Ok(n) = v.parse::<u32>() {
                        c.idle_minutes = n.clamp(1, 240);
                    }
                }
                "paused" => c.paused = matches!(v, "true" | "1" | "yes"),
                "ask_startup" => c.ask_startup = matches!(v, "true" | "1" | "yes"),
                "theme" => {
                    c.theme = ThemePref::ALL.into_iter().find(|t| t.as_str() == v).unwrap_or_default()
                }
                _ => {}
            }
        }
        c
    }

    pub fn to_text(&self) -> String {
        format!(
            "# Screentime settings. Edited by the app; safe to edit by hand.\n\
             idle_minutes = {}\npaused = {}\ntheme = {}\n{}",
            self.idle_minutes,
            self.paused,
            self.theme.as_str(),
            if self.ask_startup { "ask_startup = true\n" } else { "" }
        )
    }

    pub fn load() -> Self {
        std::fs::read_to_string(path()).map(|t| Self::parse(&t)).unwrap_or_default()
    }

    /// Write atomically (temp file + rename) so the tracker never reads half a file.
    pub fn save(&self) -> std::io::Result<()> {
        let p = path();
        if let Some(dir) = p.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let tmp = p.with_extension("ini.tmp");
        std::fs::write(&tmp, self.to_text())?;
        std::fs::rename(tmp, p)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips() {
        for ask_startup in [false, true] {
            let c = Config { idle_minutes: 10, paused: true, theme: ThemePref::Dark, ask_startup };
            assert_eq!(Config::parse(&c.to_text()), c);
        }
    }

    #[test]
    fn bad_input_falls_back_to_defaults() {
        assert_eq!(Config::parse(""), Config::default());
        let c = Config::parse("idle_minutes = lots\ntheme = neon\nfoo = bar\npaused=1\n");
        assert_eq!(c, Config { paused: true, ..Config::default() });
        assert_eq!(Config::parse("idle_minutes=0").idle_minutes, 1);
    }
}
