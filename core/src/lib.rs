//! Screentime core: tracking state machine, SQLite storage, reports,
//! export, settings and Windows probes.
//!
//! The CLI and the GUI both build on this crate. Readers need [`store::Store`]
//! (read-only), [`report`] and [`time`]; the tracker process uses [`daemon::run`].

pub mod config;
pub mod export;
pub mod names;
pub mod report;
pub mod store;
pub mod time;
pub mod tracker;

#[cfg(windows)]
pub mod daemon;
#[cfg(windows)]
pub mod win;
#[cfg(windows)]
pub mod win_ui;
