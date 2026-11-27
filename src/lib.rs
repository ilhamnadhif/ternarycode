//! TernaryCode: a terminal-first workspace for running many projects and AI coding agents in
//! one window.

pub mod actions;
pub mod ade;
pub mod app;
pub mod automations;
pub mod browser;
pub mod browser_agent;
#[cfg(target_os = "macos")]
pub mod browser_dialog;
#[cfg(target_os = "macos")]
pub mod browser_host;
#[cfg(target_os = "macos")]
pub mod browser_input;
#[cfg(target_os = "linux")]
pub mod browser_linux;
#[cfg(target_os = "macos")]
pub mod browser_popup;
#[cfg(windows)]
pub use browser_win as browser_popup;
#[cfg(windows)]
pub mod browser_win;
// The same calls as macOS's `browser_dialog` and `browser_input`, in each system's module.
#[cfg(windows)]
pub use browser_win as browser_dialog;
#[cfg(windows)]
pub use browser_win as browser_input;
#[cfg(target_os = "linux")]
pub use browser_linux as browser_dialog;
#[cfg(target_os = "linux")]
pub use browser_linux as browser_input;
#[cfg(target_os = "linux")]
pub use browser_linux as browser_popup;
pub mod cli;
pub mod companion;
pub mod companion_relay;
#[cfg(debug_assertions)]
pub mod devtest;
#[cfg(debug_assertions)]
mod devtest_run;
pub mod docview;
pub mod editor;
pub mod explorer;
#[cfg(unix)]
pub mod handoff;
pub mod history;
pub mod i18n;
pub mod instance;
pub mod lsp;
#[cfg(target_os = "macos")]
pub mod notify_native;
pub mod onboarding;
pub mod pane;
pub mod paths;
pub mod platform;
pub mod profiles;
pub mod project;
pub mod qr;
pub mod ship;
pub mod ui;
pub mod update;
pub mod watch;
pub mod worktree;
