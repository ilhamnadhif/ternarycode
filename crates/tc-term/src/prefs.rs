//! Terminal preferences shared with the app's settings, and the small path helpers the font
//! loader needs.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CursorStyle {
    #[default]
    Block,
    Beam,
    Underline,
}

/// Monospace font for the terminal. `path == None` means a preset (or the bundled Hack font).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct FontChoice {
    pub name: String,
    pub path: Option<String>,
    #[serde(default)]
    pub index: u32,
}

pub fn home_dir() -> PathBuf {
    dirs::home_dir().unwrap_or_else(|| PathBuf::from("/"))
}

/// `~/x` → `$HOME/x`; anything else unchanged.
pub fn expand_tilde(path: &str) -> PathBuf {
    if path == "~" {
        return home_dir();
    }
    match path.strip_prefix("~/") {
        Some(rest) => home_dir().join(rest),
        None => PathBuf::from(path),
    }
}

/// `$HOME/x` → `~/x`, for display.
pub fn contract_tilde(path: &std::path::Path) -> String {
    let home = home_dir();
    match path.strip_prefix(&home) {
        Ok(rest) if rest.as_os_str().is_empty() => "~".into(),
        Ok(rest) => format!("~/{}", rest.display()),
        Err(_) => path.display().to_string(),
    }
}
