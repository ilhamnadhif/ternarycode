//! Icons: the bundled Phosphor glyphs (`icon_codes.rs`, used inside ordinary text such as
//! `format!("{} Refresh", icons::ARROWS_CLOCKWISE)`) and which file gets which icon.

use eframe::egui::Color32;

pub use super::icon_codes::*;
use super::style::Palette;

/// The icon and its color for an entry of a file list.
pub fn file_icon(name: &str, is_dir: bool, is_link: bool, p: &Palette) -> (&'static str, Color32) {
    if is_dir {
        return (FOLDER, p.folder);
    }
    if is_link {
        return (LINK_SIMPLE, p.text_muted);
    }
    let ext = name
        .rsplit_once('.')
        .map(|(_, e)| e.to_ascii_lowercase())
        .unwrap_or_default();
    let icon = match ext.as_str() {
        "rs" | "go" | "py" | "js" | "ts" | "jsx" | "tsx" | "c" | "h" | "cc" | "cpp" | "hpp"
        | "java" | "kt" | "rb" | "php" | "sh" | "bash" | "zsh" | "fish" | "lua" | "swift"
        | "json" | "yaml" | "yml" | "toml" | "xml" | "html" | "css" | "scss" | "sql" | "ini"
        | "conf" | "cfg" | "env" | "dockerfile" | "mk" => FILE_CODE,
        "zip" | "gz" | "tgz" | "xz" | "bz2" | "zst" | "tar" | "7z" | "rar" | "deb" | "rpm"
        | "jar" | "war" => FILE_ZIP,
        "png" | "jpg" | "jpeg" | "gif" | "webp" | "svg" | "ico" | "bmp" | "tiff" | "heic" => {
            FILE_IMAGE
        }
        "pdf" => return (FILE_PDF, p.err),
        "docx" | "docm" | "dotx" | "doc" | "odt" | "rtf" => return (FILE_DOC, p.info),
        "xlsx" | "xlsm" | "xlsb" | "xls" | "ods" => return (FILE_XLS, p.ok),
        "csv" | "tsv" => return (FILE_CSV, p.ok),
        "pptx" | "pptm" | "ppsx" | "ppt" | "odp" | "key" => return (FILE_PPT, p.warn),
        "md" | "markdown" => FILE_MD,
        "txt" | "log" | "rst" | "out" => FILE_TEXT,
        _ => FILE,
    };
    (icon, p.text_muted)
}
