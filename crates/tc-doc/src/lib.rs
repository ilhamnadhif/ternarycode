//! Documents the editor shows instead of text: PDF pages, Word and PowerPoint files as
//! formatted text, spreadsheets and CSV as tables, and images. Everything here is plain data
//! (no UI): the app reads a file with [`load`] off the UI thread and draws what it gets; PDF
//! pages are rendered on demand by a [`pdf::Renderer`] thread.
//!
//! Read-only: nothing here writes a file.

use std::path::Path;

pub mod img;
pub mod office;
pub mod pdf;
pub mod sheet;

/// An image as straight (not premultiplied) RGBA, row by row.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Rgba {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u8>,
}

/// What kind of document a file is, from its name.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Pdf,
    Word,
    Slides,
    Sheet,
    Csv,
    Image,
    /// A format known but not readable here (old binary Word and PowerPoint): offered to open
    /// in the default app.
    Unsupported,
}

impl Kind {
    pub fn of(path: &Path) -> Option<Self> {
        let ext = path.extension()?.to_string_lossy().to_ascii_lowercase();
        Some(match ext.as_str() {
            "pdf" => Self::Pdf,
            "docx" | "docm" | "dotx" | "dotm" => Self::Word,
            "pptx" | "pptm" | "ppsx" | "potx" => Self::Slides,
            "xlsx" | "xlsm" | "xlsb" | "xls" | "xla" | "xlam" | "ods" => Self::Sheet,
            "csv" | "tsv" => Self::Csv,
            "png" | "jpg" | "jpeg" | "gif" | "webp" | "bmp" | "tif" | "tiff" | "ico" => Self::Image,
            "doc" | "ppt" | "pps" | "odt" | "odp" | "rtf" | "pages" | "numbers" | "key" => Self::Unsupported,
            _ => return None,
        })
    }

    /// Its name for people ("PDF", "Word document").
    pub fn label(self) -> &'static str {
        match self {
            Self::Pdf => "PDF",
            Self::Word => "Word",
            Self::Slides => "PowerPoint",
            Self::Sheet => "Spreadsheet",
            Self::Csv => "CSV",
            Self::Image => "Image",
            Self::Unsupported => "Document",
        }
    }
}

/// A document read from a file (PDF pages are rendered apart, see [`pdf`]).
#[derive(Debug)]
pub enum Doc {
    Text(office::RichDoc),
    Slides(office::Slides),
    Sheet(sheet::Workbook),
    Image(Rgba),
}

/// Files bigger than this are not read (a viewer would take too much memory).
pub const MAX_BYTES: u64 = 200 * 1024 * 1024;

/// Reads a document (not a PDF: see [`pdf::Renderer`]). Slow for big files: call it off the
/// UI thread.
pub fn load(path: &Path) -> Result<Doc, String> {
    let kind = Kind::of(path).ok_or("not a document")?;
    let len = std::fs::metadata(path).map_err(|e| e.to_string())?.len();
    if len > MAX_BYTES {
        return Err(format!("the file is too big to show ({} MB)", len / 1024 / 1024));
    }
    match kind {
        Kind::Word => office::read_docx(path).map(Doc::Text),
        Kind::Slides => office::read_pptx(path).map(Doc::Slides),
        Kind::Sheet => sheet::read_workbook(path).map(Doc::Sheet),
        Kind::Csv => sheet::read_csv(path).map(Doc::Sheet),
        Kind::Image => {
            let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
            img::decode(&bytes, img::MAX_SIDE).map(Doc::Image)
        }
        Kind::Pdf => Err("PDF pages are rendered by pdf::Renderer".into()),
        Kind::Unsupported => Err("this format cannot be shown here".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kinds_come_from_the_extension() {
        assert_eq!(Kind::of(Path::new("/a/B.PDF")), Some(Kind::Pdf));
        assert_eq!(Kind::of(Path::new("x.docx")), Some(Kind::Word));
        assert_eq!(Kind::of(Path::new("x.xls")), Some(Kind::Sheet));
        assert_eq!(Kind::of(Path::new("x.tsv")), Some(Kind::Csv));
        assert_eq!(Kind::of(Path::new("x.doc")), Some(Kind::Unsupported));
        assert_eq!(Kind::of(Path::new("x.rs")), None);
        assert_eq!(Kind::of(Path::new("Makefile")), None);
    }
}
