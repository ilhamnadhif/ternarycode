//! Positions: the editor counts characters; LSP counts lines and, within a line, UTF-16 code
//! units (or bytes, when both sides agree on UTF-8). Lines end at `\n`; a `\r` before it is
//! not part of the line.

use std::path::{Path, PathBuf};

use lsp_types::{Position, Range, Uri};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Encoding {
    #[default]
    Utf16,
    Utf8,
}

/// Where each line starts (as a char index and a byte offset), to turn positions into
/// indices and back without walking the whole text. Rebuild it when the text changes.
#[derive(Clone, Debug, Default)]
pub struct LineIndex {
    starts: Vec<(usize, usize)>,
    chars: usize,
    bytes: usize,
}

impl LineIndex {
    pub fn new(text: &str) -> Self {
        let mut starts = vec![(0, 0)];
        let mut chars = 0;
        for (b, c) in text.char_indices() {
            chars += 1;
            if c == '\n' {
                starts.push((chars, b + 1));
            }
        }
        Self {
            starts,
            chars,
            bytes: text.len(),
        }
    }

    pub fn lines(&self) -> usize {
        self.starts.len()
    }

    /// The line holding char index `at` (0-based).
    pub fn line_of(&self, at: usize) -> usize {
        self.starts.partition_point(|(c, _)| *c <= at).saturating_sub(1)
    }

    /// Char index where line `line` starts (the end of the text past the last line).
    pub fn line_start(&self, line: usize) -> usize {
        self.starts.get(line).map_or(self.chars, |(c, _)| *c)
    }

    /// The text of a line, without its line break.
    pub fn line_text<'a>(&self, text: &'a str, line: usize) -> &'a str {
        let Some(&(_, b)) = self.starts.get(line) else {
            return "";
        };
        let end = self.starts.get(line + 1).map_or(self.bytes, |(_, e)| *e);
        let s = &text[b.min(text.len())..end.min(text.len())];
        let s = s.strip_suffix('\n').unwrap_or(s);
        s.strip_suffix('\r').unwrap_or(s)
    }

    pub fn position(&self, text: &str, at: usize, enc: Encoding) -> Position {
        let at = at.min(self.chars);
        let line = self.line_of(at);
        let col_chars = at - self.line_start(line);
        let units: usize = self
            .line_text(text, line)
            .chars()
            .take(col_chars)
            .map(|c| match enc {
                Encoding::Utf16 => c.len_utf16(),
                Encoding::Utf8 => c.len_utf8(),
            })
            .sum();
        Position::new(line as u32, units as u32)
    }

    /// The char index of a position (past the end of its line: the line's end; past the last
    /// line: the end of the text).
    pub fn char_index(&self, text: &str, pos: Position, enc: Encoding) -> usize {
        let line = pos.line as usize;
        if line >= self.starts.len() {
            return self.chars;
        }
        let mut units = 0usize;
        let mut chars = 0usize;
        for c in self.line_text(text, line).chars() {
            if units >= pos.character as usize {
                break;
            }
            units += match enc {
                Encoding::Utf16 => c.len_utf16(),
                Encoding::Utf8 => c.len_utf8(),
            };
            chars += 1;
        }
        self.line_start(line) + chars
    }

    pub fn range(&self, text: &str, from: usize, to: usize, enc: Encoding) -> Range {
        Range::new(self.position(text, from, enc), self.position(text, to, enc))
    }

    /// A range as char indices.
    pub fn chars(&self, text: &str, range: &Range, enc: Encoding) -> (usize, usize) {
        (self.char_index(text, range.start, enc), self.char_index(text, range.end, enc))
    }
}

/// Char index → byte offset in `text`.
pub fn byte_of(text: &str, at: usize) -> usize {
    text.char_indices().nth(at).map_or(text.len(), |(b, _)| b)
}

/// `file://` URI of a path (percent-encoded). Windows: `file:///C:/dir/file`, and
/// `file://server/share/…` for network paths; the `\\?\` form of a path is taken apart too.
#[cfg(windows)]
pub fn uri(path: &Path) -> Uri {
    let mut p = path.to_string_lossy().replace('\\', "/");
    if let Some(rest) = p.strip_prefix("//?/UNC/") {
        p = format!("//{rest}");
    } else if let Some(rest) = p.strip_prefix("//?/") {
        p = rest.to_string();
    }
    let (host, p) = match p.strip_prefix("//") {
        Some(rest) => match rest.find('/') {
            Some(i) => (rest[..i].to_string(), rest[i..].to_string()),
            None => (rest.to_string(), "/".to_string()),
        },
        None if p.starts_with('/') => (String::new(), p),
        None => (String::new(), format!("/{p}")),
    };
    let mut s = format!("file://{host}");
    for b in p.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'/' | b'-' | b'_' | b'.' | b'~' | b':') {
            s.push(b as char);
        } else {
            s.push_str(&format!("%{b:02X}"));
        }
    }
    s.parse().unwrap_or_else(|_| "file:///".parse().expect("a valid uri"))
}

/// `file://` URI of a path (percent-encoded).
#[cfg(not(windows))]
pub fn uri(path: &Path) -> Uri {
    let mut s = String::from("file://");
    // The path's own bytes (not UTF-8 on Linux, sometimes): every byte round-trips.
    for b in path.as_os_str().as_encoded_bytes().iter().copied() {
        if b.is_ascii_alphanumeric() || matches!(b, b'/' | b'-' | b'_' | b'.' | b'~') {
            s.push(b as char);
        } else {
            s.push_str(&format!("%{b:02X}"));
        }
    }
    s.parse().unwrap_or_else(|_| "file:///".parse().expect("a valid uri"))
}

/// The path of a `file://` URI.
pub fn path(uri: &Uri) -> Option<PathBuf> {
    let s = uri.as_str();
    let rest = s.strip_prefix("file://")?;
    // `file://host/path`: the path starts at the first slash.
    let slash = rest.find('/')?;
    #[cfg(windows)]
    let host = &rest[..slash];
    let rest = &rest[slash..];
    let bytes = rest.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && i + 2 < bytes.len()
            && let Ok(v) = u8::from_str_radix(std::str::from_utf8(&bytes[i + 1..i + 3]).ok()?, 16)
        {
            out.push(v);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStringExt;
        Some(PathBuf::from(std::ffi::OsString::from_vec(out)))
    }
    #[cfg(windows)]
    {
        // `/C:/dir` → `C:\dir`; a host is a network share (`\\server\share\…`).
        let text = String::from_utf8_lossy(&out).replace('/', "\\");
        let b = text.as_bytes();
        let text = if b.len() >= 3 && b[0] == b'\\' && b[1].is_ascii_alphabetic() && b[2] == b':' {
            text[1..].to_string()
        } else if !host.is_empty() && !host.eq_ignore_ascii_case("localhost") {
            format!("\\\\{host}{text}")
        } else {
            text
        };
        Some(PathBuf::from(text))
    }
    #[cfg(not(any(unix, windows)))]
    Some(PathBuf::from(String::from_utf8_lossy(&out).into_owned()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn positions_count_utf16_units() {
        // "é" is one unit, "😀" two (one char each); CRLF line ends.
        let text = "ab\r\né😀x\nlast";
        let ix = LineIndex::new(text);
        assert_eq!(ix.lines(), 3);
        assert_eq!(ix.line_text(text, 0), "ab");
        assert_eq!(ix.line_text(text, 1), "é😀x");
        // 'x' is char 6 (a b \r \n é 😀 x).
        assert_eq!(ix.position(text, 6, Encoding::Utf16), Position::new(1, 3));
        assert_eq!(ix.position(text, 6, Encoding::Utf8), Position::new(1, 6));
        assert_eq!(ix.char_index(text, Position::new(1, 3), Encoding::Utf16), 6);
        assert_eq!(ix.char_index(text, Position::new(1, 6), Encoding::Utf8), 6);
        // Past the end of a line: its end; past the last line: the end of the text.
        assert_eq!(ix.char_index(text, Position::new(0, 99), Encoding::Utf16), 2);
        assert_eq!(ix.char_index(text, Position::new(9, 0), Encoding::Utf16), text.chars().count());
        assert_eq!(ix.line_of(0), 0);
        assert_eq!(ix.line_of(4), 1);
        assert_eq!(byte_of(text, 6), 10);
    }

    #[test]
    fn uris_round_trip() {
        let p = Path::new("/Users/me/my project/é#1.rs");
        let u = uri(p);
        assert_eq!(u.as_str(), "file:///Users/me/my%20project/%C3%A9%231.rs");
        assert_eq!(path(&u).unwrap(), p);
    }

    #[cfg(windows)]
    #[test]
    fn windows_uris_have_the_drive_after_three_slashes() {
        let p = Path::new(r"C:\Users\me\my project\a.rs");
        let u = uri(p);
        assert_eq!(u.as_str(), "file:///C:/Users/me/my%20project/a.rs");
        assert_eq!(path(&u).unwrap(), p);
        assert_eq!(uri(Path::new(r"\\?\C:\x\y.rs")).as_str(), "file:///C:/x/y.rs");
        let share = uri(Path::new(r"\\server\share\z.rs"));
        assert_eq!(share.as_str(), "file://server/share/z.rs");
        assert_eq!(path(&share).unwrap(), Path::new(r"\\server\share\z.rs"));
        // As servers send them: the colon encoded, the drive lowercase.
        let sent: Uri = "file:///c%3A/Users/me/b.rs".parse().unwrap();
        assert_eq!(path(&sent).unwrap(), Path::new(r"c:\Users\me\b.rs"));
    }
}
