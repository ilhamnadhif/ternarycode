//! TernaryCode's terminal core: an alacritty grid fed by a local PTY, an OSC scanner for the
//! signals `vte` drops (OSC 9, 99, 777, 133), and the egui widget that draws and drives it.
//!
//! Most of this crate is a copy of TernaryShell's terminal (see `UPSTREAM.md`), developed on
//! its own from there.

pub use alacritty_terminal;

pub mod core;
pub mod fonts;
pub mod input;
pub mod layout;
pub mod osc;
pub mod prefs;
#[cfg(unix)]
pub mod pty;
#[cfg(windows)]
#[path = "pty_windows.rs"]
pub mod pty;
pub mod scheme;
pub mod search;
pub mod servers;
pub mod widget;

use std::sync::OnceLock;

static TRANSLATE: OnceLock<fn(&'static str) -> &'static str> = OnceLock::new();

/// Installs the app's translation function for the few labels this crate draws (the terminal's
/// context menu). Without it the English text is shown.
pub fn set_translator(f: fn(&'static str) -> &'static str) {
    let _ = TRANSLATE.set(f);
}

pub(crate) fn tr(en: &'static str) -> &'static str {
    TRANSLATE.get().map_or(en, |f| f(en))
}
