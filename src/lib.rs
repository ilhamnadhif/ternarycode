//! TernaryCode: a terminal-first workspace for running many projects and AI coding agents in
//! one window.

pub mod actions;
pub mod app;
pub mod browser;
pub mod browser_agent;
#[cfg(target_os = "macos")]
pub mod browser_dialog;
#[cfg(target_os = "macos")]
pub mod browser_popup;
pub mod cli;
#[cfg(debug_assertions)]
pub mod devtest;
#[cfg(debug_assertions)]
mod devtest_run;
pub mod docview;
pub mod editor;
pub mod explorer;
#[cfg(unix)]
pub mod handoff;
pub mod i18n;
pub mod instance;
pub mod lsp;
#[cfg(target_os = "macos")]
pub mod notify_native;
pub mod pane;
pub mod paths;
pub mod platform;
pub mod project;
pub mod ui;
pub mod update;
pub mod watch;
pub mod worktree;
