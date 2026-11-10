//! egui user interface.

pub mod dialogs;
pub mod docview_ui;
pub mod editor_ui;
pub mod explorer_ui;
pub mod ide_ui;
pub mod focus;
pub mod grid;
pub mod icon_codes;
pub mod icons;
pub mod kit;
#[cfg(target_os = "macos")]
pub mod menu;
pub mod palette;
pub mod review;
pub mod settings;
pub mod sidebar;
pub mod status;
pub mod statusbar;
pub mod style;
pub mod update_ui;
pub mod view;
