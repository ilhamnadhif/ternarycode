//! The secondary editor (FR-E1–E3, E6): open files of a project, edited as text, saved only when
//! the file on disk is still the one that was loaded; a change made outside (an agent) reloads a
//! clean buffer and puts a choice in front of a dirty one.

use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::SystemTime;

use eframe::egui::{self, Color32, FontId, text::LayoutJob};
use syntect::highlighting::Theme;
use syntect::parsing::SyntaxSet;

/// Files larger than this open read-only (FR-E6).
pub const READ_ONLY_ABOVE: u64 = 10 * 1024 * 1024;
/// Only this much of a large file is shown.
pub const SHOW_AT_MOST: usize = 2 * 1024 * 1024;
/// Highlighting stops above this size (plain text is shown).
pub const HIGHLIGHT_AT_MOST: usize = 1024 * 1024;

pub struct OpenFile {
    pub path: PathBuf,
    pub text: String,
    /// The text as loaded or last saved (dirty = `text != saved`).
    pub saved: String,
    pub mtime: Option<SystemTime>,
    pub read_only: bool,
    pub truncated: bool,
    /// The file changed on disk while the buffer had unsaved edits.
    pub conflict: bool,
    /// Show the difference between the disk and the buffer.
    pub compare: bool,
    /// That difference, worked out when Compare is pressed (not on every frame).
    pub compare_diff: Option<String>,
    /// Line and column of the caret (1-based), for the editor's status line.
    pub caret: (usize, usize),
    /// The character index `caret` was worked out for.
    pub caret_index: Option<usize>,
    /// Number of lines (for the width of the line numbers).
    pub lines: usize,
    /// Lines end with CR LF.
    pub crlf: bool,
    /// Not UTF-8: shown with replacement marks (and so read-only).
    pub lossy: bool,
    /// Bumped whenever `text` changes (search results follow it).
    pub rev: u64,
    pub goto_line: Option<usize>,
    /// Put the caret here (a char range, selected) on the next frame.
    pub select: Option<(usize, usize)>,
    /// Where lines start, for the revision it was made for (language server positions).
    pub index: Option<(u64, tc_lsp::pos::LineIndex)>,
    pub syntax: String,
    /// Syntax colours, kept per line so an edit colours only what it changed.
    pub highlight: Highlight,
    /// A document (PDF, Word, sheet, image…) shown instead of text.
    pub doc: Option<Box<crate::docview::DocView>>,
    /// The agent at work when the file last changed on disk under a clean buffer, and when.
    pub changed_by: Option<(String, std::time::Instant)>,
}

impl OpenFile {
    pub fn dirty(&self) -> bool {
        self.text != self.saved
    }

    pub fn name(&self) -> String {
        self.path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default()
    }
}

#[derive(Default)]
pub struct Editor {
    pub visible: bool,
    pub files: Vec<OpenFile>,
    pub active: usize,
    pub width: f32,
    /// Files closed lately, newest first (offered when nothing is open).
    pub recent: Vec<PathBuf>,
    /// The find bar (⌘F while the editor has the keyboard).
    pub find: Option<Find>,
    /// Give the text the keyboard on the next frame (after the find bar closed; doing it in
    /// the same frame would hand it that frame's Enter or Esc).
    pub refocus: bool,
}

/// Search in the open file. A query `:42` goes to line 42 instead.
#[derive(Default)]
pub struct Find {
    pub query: String,
    /// Letters must match in case too (Aa).
    pub case: bool,
    /// Put the keyboard in the field on the next frame.
    pub focus: bool,
    pub matches: Vec<Match>,
    pub current: Option<usize>,
    /// Query, file and text revision the matches were found for.
    key: Option<(String, PathBuf, u64, bool)>,
}

/// One occurrence: its line (0-based), its columns in that line and its characters in the
/// whole text (for the selection).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Match {
    pub line: usize,
    pub col: (usize, usize),
    pub chars: (usize, usize),
}

/// At most this many occurrences are found (and highlighted).
pub const MAX_MATCHES: usize = 10_000;

impl Find {
    /// Brings the matches up to date with the query and text; returns whether they changed.
    pub fn refresh(&mut self, file: &OpenFile) -> bool {
        let key = (self.query.clone(), file.path.clone(), file.rev, self.case);
        if self.key.as_ref() == Some(&key) {
            return false;
        }
        self.matches = if self.query.starts_with(':') { Vec::new() } else { find_all_case(&file.text, &self.query, self.case) };
        self.current = None;
        self.key = Some(key);
        true
    }

    /// The match after (or before) the current one, or after the caret when there is none.
    pub fn step(&mut self, forward: bool, caret: usize) -> Option<Match> {
        if self.matches.is_empty() {
            self.current = None;
            return None;
        }
        let n = self.matches.len();
        let i = match self.current {
            Some(c) if forward => (c + 1) % n,
            Some(c) => (c + n - 1) % n,
            None if forward => self.matches.iter().position(|m| m.chars.0 >= caret).unwrap_or(0),
            None => self.matches.iter().rposition(|m| m.chars.0 < caret).unwrap_or(n - 1),
        };
        self.current = Some(i);
        Some(self.matches[i])
    }

    /// The line a `:N` query asks for (0-based).
    pub fn goto_line(&self) -> Option<usize> {
        self.query.strip_prefix(':')?.trim().parse::<usize>().ok().map(|n| n.saturating_sub(1))
    }
}

/// Every occurrence of `query` in `text`, ignoring ASCII case, in order.
pub fn find_all(text: &str, query: &str) -> Vec<Match> {
    find_all_case(text, query, false)
}

/// Every occurrence of `query` in `text` (in case too when `case`), in order.
pub fn find_all_case(text: &str, query: &str, case: bool) -> Vec<Match> {
    let mut out = Vec::new();
    if query.is_empty() {
        return out;
    }
    let hay = text.as_bytes();
    let fold = |b: u8| if case { b } else { b.to_ascii_lowercase() };
    let needle: Vec<u8> = query.bytes().map(fold).collect();
    let (mut line, mut line_start_char, mut chars, mut at) = (0usize, 0usize, 0usize, 0usize);
    let mut i = 0;
    while i + needle.len() <= hay.len() && out.len() < MAX_MATCHES {
        if hay[i..i + needle.len()].iter().zip(&needle).all(|(a, b)| fold(*a) == *b)
            && text.is_char_boundary(i)
            && text.is_char_boundary(i + needle.len())
        {
            // Count characters and lines between the last position and this match.
            for c in text[at..i].chars() {
                chars += 1;
                if c == '\n' {
                    line += 1;
                    line_start_char = chars;
                }
            }
            let len = text[i..i + needle.len()].chars().count();
            let col = chars - line_start_char;
            out.push(Match {
                line,
                col: (col, col + len),
                chars: (chars, chars + len),
            });
            at = i;
            i += needle.len().max(1);
        } else {
            i += 1;
        }
    }
    out
}

/// How many closed files [`Editor::recent`] keeps.
const RECENT: usize = 6;

impl Editor {
    /// Closes file `i` (the caller checks it has no unsaved edits).
    pub fn close(&mut self, i: usize) {
        if i >= self.files.len() {
            return;
        }
        let f = self.files.remove(i);
        self.recent.retain(|p| *p != f.path);
        self.recent.insert(0, f.path);
        self.recent.truncate(RECENT);
        if self.active > i || self.active >= self.files.len() {
            self.active = self.active.saturating_sub(1);
        }
    }

    /// Recently closed files that are not open now.
    pub fn recent_closed(&self) -> impl Iterator<Item = &PathBuf> {
        self.recent.iter().filter(|p| !self.files.iter().any(|f| f.path == **p))
    }

    pub fn file(&self) -> Option<&OpenFile> {
        self.files.get(self.active)
    }

    pub fn file_mut(&mut self) -> Option<&mut OpenFile> {
        self.files.get_mut(self.active)
    }
}

pub enum OpenError {
    Binary,
    Io(String),
}

fn mtime(path: &Path) -> Option<SystemTime> {
    std::fs::metadata(path).and_then(|m| m.modified()).ok()
}

/// Reads a file for the editor.
pub fn load(path: &Path) -> Result<OpenFile, OpenError> {
    let meta = std::fs::metadata(path).map_err(|e| OpenError::Io(e.to_string()))?;
    let read_only = meta.len() > READ_ONLY_ABOVE || meta.permissions().readonly();
    let mut bytes = Vec::new();
    {
        use std::io::Read;
        let f = std::fs::File::open(path).map_err(|e| OpenError::Io(e.to_string()))?;
        f.take(SHOW_AT_MOST as u64 + 1)
            .read_to_end(&mut bytes)
            .map_err(|e| OpenError::Io(e.to_string()))?;
    }
    if bytes.iter().take(8192).any(|b| *b == 0) {
        return Err(OpenError::Binary);
    }
    let truncated = bytes.len() > SHOW_AT_MOST;
    bytes.truncate(SHOW_AT_MOST);
    // Text that is not UTF-8 (Latin-1, say) shows with replacement marks; saving would write
    // those marks into the file, so it opens read-only. (A cut at 2 MB may split a character:
    // that file is read-only anyway.)
    let lossy = std::str::from_utf8(&bytes).is_err() && !truncated;
    let text = String::from_utf8_lossy(&bytes).into_owned();
    let (lines, crlf) = (count_lines(&text), text.contains("\r\n"));
    Ok(OpenFile {
        path: path.to_path_buf(),
        saved: text.clone(),
        text,
        mtime: Some(meta.modified().unwrap_or(SystemTime::UNIX_EPOCH)),
        read_only: read_only || truncated || lossy,
        truncated,
        conflict: false,
        compare: false,
        compare_diff: None,
        caret: (1, 1),
        caret_index: None,
        lines,
        crlf,
        lossy,
        rev: 0,
        goto_line: None,
        select: None,
        index: None,
        syntax: syntax_for(path),
        highlight: Highlight::default(),
        doc: None,
        changed_by: None,
    })
}

impl OpenFile {
    /// A document shown by `DocView` (read in the background), not as text.
    pub fn document(path: PathBuf, ctx: &eframe::egui::Context) -> Self {
        Self {
            doc: Some(Box::new(crate::docview::DocView::open(path.clone(), ctx))),
            mtime: mtime(&path),
            path,
            text: String::new(),
            saved: String::new(),
            read_only: true,
            truncated: false,
            conflict: false,
            compare: false,
            compare_diff: None,
            caret: (1, 1),
            caret_index: None,
            lines: 0,
            crlf: false,
            lossy: false,
            rev: 0,
            goto_line: None,
            select: None,
            index: None,
            syntax: String::new(),
            highlight: Highlight::default(),
            changed_by: None,
        }
    }
}

pub enum SaveError {
    /// The file changed on disk since it was loaded.
    ChangedOnDisk,
    Io(String),
}

/// Saves the buffer, unless the file on disk changed since it was loaded (then nothing is
/// written: the user decides). Keeps the file's permissions.
pub fn save(file: &mut OpenFile, force: bool) -> Result<(), SaveError> {
    if file.read_only {
        return Err(SaveError::Io(crate::i18n::t("This file is read-only.").into()));
    }
    // By content, whatever the time stamp says: one that counts in seconds (or less finely)
    // misses an agent's write made in the same tick as the last load or save.
    if !force && file.path.exists() {
        let disk = std::fs::read(&file.path).unwrap_or_default();
        if !is_saved(file, &disk) {
            file.conflict = true;
            return Err(SaveError::ChangedOnDisk);
        }
    }
    // A CRLF file stays one: lines typed here end with LF in the buffer.
    let data = if file.crlf { to_crlf(&file.text) } else { std::borrow::Cow::Borrowed(file.text.as_str()) };
    write_atomic(&file.path, data.as_bytes()).map_err(|e| SaveError::Io(e.to_string()))?;
    file.saved = file.text.clone();
    file.mtime = mtime(&file.path);
    file.conflict = false;
    file.compare = false;
    file.compare_diff = None;
    Ok(())
}

/// Called when the file may have changed on disk. A clean buffer follows the disk; a dirty one
/// is marked as in conflict. Returns whether anything changed.
pub fn on_disk_change(file: &mut OpenFile) -> bool {
    let now = mtime(&file.path);
    // An editable file is compared by content even when the time stamp is the same (a coarse
    // one gives an agent's write the stamp of the last load or save).
    let editable = file.doc.is_none() && !file.truncated && !file.read_only;
    if now == file.mtime && !editable {
        return false;
    }
    if let Some(doc) = file.doc.as_mut() {
        doc.reload = true;
        file.mtime = now;
        return true;
    }
    // A big file shows its first part only: read that part again, not the whole file.
    if file.truncated || file.read_only {
        if let Ok(fresh) = load(&file.path) {
            file.lines = fresh.lines;
            file.crlf = fresh.crlf;
            file.lossy = fresh.lossy;
            file.rev += 1;
            file.text = fresh.text;
            file.saved = fresh.saved;
            file.truncated = fresh.truncated;
            file.read_only = fresh.read_only;
            file.mtime = fresh.mtime;
        }
        return true;
    }
    let Ok(disk) = std::fs::read_to_string(&file.path) else {
        return false;
    };
    if is_saved(file, disk.as_bytes()) {
        file.mtime = now;
        return false;
    }
    if file.dirty() {
        file.conflict = true;
        file.compare_diff = None;
    } else {
        file.lines = count_lines(&disk);
        file.rev += 1;
        file.crlf = disk.contains("\r\n");
        file.text = disk.clone();
        file.saved = disk;
        file.mtime = now;
    }
    true
}

/// Throws the edits away and takes the file from disk.
pub fn reload(file: &mut OpenFile) {
    if let Ok(disk) = std::fs::read_to_string(&file.path) {
        file.lines = count_lines(&disk);
        file.rev += 1;
        file.crlf = disk.contains("\r\n");
        file.text = disk.clone();
        file.saved = disk;
        file.mtime = mtime(&file.path);
    }
    file.conflict = false;
    file.compare = false;
    file.compare_diff = None;
}

/// Whether `disk` is the text as loaded or last saved (a CRLF file's is written with CR LF on
/// every line, while lines typed since loading end with LF in `saved`).
fn is_saved(file: &OpenFile, disk: &[u8]) -> bool {
    disk == file.saved.as_bytes() || (file.crlf && disk == to_crlf(&file.saved).as_bytes())
}

/// `text` with every line ending in CR LF (a lone LF gets its CR).
pub fn to_crlf(text: &str) -> std::borrow::Cow<'_, str> {
    let b = text.as_bytes();
    if !(0..b.len()).any(|i| b[i] == b'\n' && (i == 0 || b[i - 1] != b'\r')) {
        return std::borrow::Cow::Borrowed(text);
    }
    let mut out = String::with_capacity(text.len() + text.len() / 16);
    let mut prev = '\0';
    for c in text.chars() {
        if c == '\n' && prev != '\r' {
            out.push('\r');
        }
        out.push(c);
        prev = c;
    }
    std::borrow::Cow::Owned(out)
}

/// Writes `data` to `path` through a temporary file beside it and a rename, so a reader (an
/// agent, a build) or a crash meets the old text or the new, never half of it. A symbolic link
/// stays one (the file it points at is written), a file keeps its permissions and a new one
/// gets the usual ones. Where no file can be made beside it, it is written in place.
pub fn write_atomic(path: &Path, data: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let resolved;
    let path = if std::fs::symlink_metadata(path).is_ok_and(|m| m.file_type().is_symlink()) {
        resolved = std::fs::canonicalize(path)?;
        resolved.as_path()
    } else {
        path
    };
    let perms = std::fs::metadata(path).ok().map(|m| m.permissions());
    // A rename would replace a file nobody may write.
    if perms.as_ref().is_some_and(|p| p.readonly()) {
        return Err(std::io::ErrorKind::PermissionDenied.into());
    }
    let dir = path.parent().unwrap_or(Path::new("."));
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let n = COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let tmp = dir.join(format!(".{name}.{}-{n}.tc-tmp", std::process::id()));
    let Ok(mut f) = std::fs::OpenOptions::new().write(true).create_new(true).open(&tmp) else {
        return std::fs::write(path, data);
    };
    let result = (|| {
        f.write_all(data)?;
        drop(f);
        #[cfg(unix)]
        if let Some(p) = perms {
            std::fs::set_permissions(&tmp, p)?;
        }
        std::fs::rename(&tmp, path)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    result
}

/// Lines in `text` (a text without a final newline still has its last line).
pub fn count_lines(text: &str) -> usize {
    text.bytes().filter(|b| *b == b'\n').count() + 1
}

/// The difference between the file on disk and the buffer, for Compare.
pub fn compare_diff(file: &OpenFile) -> String {
    let disk = std::fs::read_to_string(&file.path).unwrap_or_default();
    let text = if file.crlf { to_crlf(&file.text) } else { std::borrow::Cow::Borrowed(file.text.as_str()) };
    tc_agent::integrate::diff_lines(&disk, &text)
}

/// A readable name for the language of a file (shown in the editor's status line): its own
/// for the files whose colours borrow another language's (a .ts file is coloured as
/// JavaScript, but it is TypeScript).
pub fn language_for(path: &Path, syntax: &str) -> String {
    let ext = path.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
    let name = path.file_name().map(|n| n.to_string_lossy().to_lowercase()).unwrap_or_default();
    let own = match ext.as_str() {
        "ts" | "mts" | "cts" => Some("TypeScript"),
        "tsx" => Some("TypeScript React"),
        "jsx" => Some("JavaScript React"),
        "toml" => Some("TOML"),
        "lock" if name == "cargo.lock" => Some("TOML"),
        "zsh" => Some("Zsh"),
        "fish" => Some("Fish"),
        "jsonc" => Some("JSON with Comments"),
        _ => None,
    };
    own.map(str::to_string).unwrap_or_else(|| language_name(syntax))
}

/// A readable name for a syntax.
pub fn language_name(syntax: &str) -> String {
    let h = highlighting();
    h.syntaxes
        .find_syntax_by_extension(syntax)
        .or_else(|| h.syntaxes.find_syntax_by_token(syntax))
        .map(|s| s.name.clone())
        .unwrap_or_else(|| crate::i18n::t("Plain Text").to_string())
}

/// The syntect syntax name for a file.
pub fn syntax_for(path: &Path) -> String {
    let ext = path
        .extension()
        .map(|e| e.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    // Extensions the default syntax set lacks map to the closest one it has.
    match ext.as_str() {
        "ts" | "tsx" | "mts" | "cts" | "jsx" | "mjs" | "cjs" => "js".to_string(),
        "zsh" | "fish" => "sh".to_string(),
        "jsonc" => "json".to_string(),
        "toml" | "lock" | "ini" | "cfg" | "conf" => "ini".to_string(),
        _ if name == "dockerfile" || name == "makefile" => name.clone(),
        _ => ext,
    }
}

struct Highlighting {
    syntaxes: SyntaxSet,
}

/// The syntaxes and themes, loaded when the editor first colours something and let go when it
/// has not been shown for a while (`release_highlighting`): the regexes syntect compiles while
/// colouring take tens of MB (about 25 MB for Markdown, 22 MB for Rust).
fn store() -> &'static std::sync::Mutex<Option<std::sync::Arc<Highlighting>>> {
    static H: OnceLock<std::sync::Mutex<Option<std::sync::Arc<Highlighting>>>> = OnceLock::new();
    H.get_or_init(|| std::sync::Mutex::new(None))
}

fn highlighting() -> std::sync::Arc<Highlighting> {
    let mut h = store().lock().unwrap_or_else(|e| e.into_inner());
    h.get_or_insert_with(|| {
        std::sync::Arc::new(Highlighting {
            syntaxes: SyntaxSet::load_defaults_newlines(),
        })
    })
    .clone()
}

/// Whether the syntaxes are loaded.
pub fn highlighting_loaded() -> bool {
    store().lock().map(|h| h.is_some()).unwrap_or(false)
}

/// Lets the syntaxes (and the regexes compiled from them) go; they load again when needed.
pub fn release_highlighting() {
    if let Ok(mut h) = store().lock() {
        *h = None;
    }
}

/// Syntax colours in TernaryCode's own palette (the ANSI colours of its terminal schemes):
/// keywords magenta, functions blue, types yellow, strings green, numbers orange, macros cyan,
/// comments faint.
fn theme(_h: &Highlighting, dark: bool) -> &'static Theme {
    static DARK: OnceLock<Theme> = OnceLock::new();
    static LIGHT: OnceLock<Theme> = OnceLock::new();
    let cell = if dark { &DARK } else { &LIGHT };
    cell.get_or_init(|| ternary_theme(dark))
}

fn ternary_theme(dark: bool) -> Theme {
    use std::str::FromStr;
    use syntect::highlighting::{Color as C, FontStyle, ScopeSelectors, StyleModifier, ThemeItem, ThemeSettings};
    let rgb = |v: u32| C { r: (v >> 16) as u8, g: (v >> 8) as u8, b: v as u8, a: 255 };
    let (text, kw, func, ty, string, num, com, mac) = if dark {
        (0xe6e6e6, 0xc58ad8, 0x6aa6e8, 0xe0be7a, 0x98c379, 0xe8a36a, 0x767676, 0x5fb8c2)
    } else {
        (0x1d1d1d, 0x8e44ad, 0x2563c9, 0x8a6100, 0x2f7d32, 0xb4561b, 0x8e8e8e, 0x16808a)
    };
    let item = |scope: &str, color: u32, style: Option<FontStyle>| ThemeItem {
        scope: ScopeSelectors::from_str(scope).unwrap_or_default(),
        style: StyleModifier { foreground: Some(rgb(color)), background: None, font_style: style },
    };
    Theme {
        name: Some("TernaryCode".into()),
        author: None,
        settings: ThemeSettings { foreground: Some(rgb(text)), ..Default::default() },
        scopes: vec![
            item("comment, punctuation.definition.comment", com, Some(FontStyle::ITALIC)),
            item("string, punctuation.definition.string", string, None),
            item("constant.numeric, constant.language, constant.character", num, None),
            item("keyword, storage, keyword.operator.word", kw, None),
            item("entity.name.type, entity.name.struct, entity.name.enum, entity.name.class, entity.name.trait, support.type, support.class, storage.type.primitive", ty, None),
            item("entity.name.function, support.function, meta.function-call variable.function, variable.function", func, None),
            item("support.macro, entity.name.macro, variable.macro, meta.macro", mac, None),
            item("keyword.operator, punctuation", text, None),
        ],
    }
}

/// The colours of one line of code (coloured on its own, from the start state): for diffs.
pub fn line_colors(line: &str, syntax: &str, dark: bool, plain: Color32) -> Vec<(Color32, String)> {
    let h = highlighting();
    let Some(syntax_ref) = h.syntaxes.find_syntax_by_extension(syntax).or_else(|| h.syntaxes.find_syntax_by_token(syntax)) else {
        return vec![(plain, line.to_string())];
    };
    let mut hl = syntect::easy::HighlightLines::new(syntax_ref, theme(&h, dark));
    match hl.highlight_line(line, &h.syntaxes) {
        Ok(ranges) => ranges
            .into_iter()
            .map(|(style, piece)| (Color32::from_rgb(style.foreground.r, style.foreground.g, style.foreground.b), piece.to_string()))
            .collect(),
        Err(_) => vec![(plain, line.to_string())],
    }
}

/// The layout of `text` with syntax colours (plain above `HIGHLIGHT_AT_MOST`), coloured whole:
/// what [`cached_layout`] must give (the tests compare the two).
#[cfg(test)]
pub fn layout(text: &str, syntax: &str, dark: bool, font: FontId, plain: Color32) -> LayoutJob {
    let mut job = LayoutJob::default();
    let h = highlighting();
    let syntax_ref = (text.len() <= HIGHLIGHT_AT_MOST)
        .then(|| {
            h.syntaxes
                .find_syntax_by_extension(syntax)
                .or_else(|| h.syntaxes.find_syntax_by_token(syntax))
        })
        .flatten();
    let Some(syntax_ref) = syntax_ref else {
        job.append(text, 0.0, egui::TextFormat::simple(font, plain));
        return job;
    };
    let mut hl = syntect::easy::HighlightLines::new(syntax_ref, theme(&h, dark));
    for line in syntect::util::LinesWithEndings::from(text) {
        match hl.highlight_line(line, &h.syntaxes) {
            Ok(ranges) => {
                for (style, piece) in ranges {
                    let c = style.foreground;
                    job.append(
                        piece,
                        0.0,
                        egui::TextFormat::simple(font.clone(), Color32::from_rgb(c.r, c.g, c.b)),
                    );
                }
            }
            Err(_) => job.append(line, 0.0, egui::TextFormat::simple(font.clone(), plain)),
        }
    }
    job
}

/// Syntax colours of a file kept per line: an edit colours again only from the line it
/// changed until the parser is back in the state it had there (usually a line or two later),
/// not the whole file (a 20 KB Markdown file took about 170 ms on every key).
#[derive(Default)]
pub struct Highlight {
    /// What the lines were coloured for (syntax, theme, font size).
    key: Option<(String, bool, u32)>,
    lines: Vec<HlLine>,
    /// The parser's state after the last line.
    end: Option<HlState>,
    /// The layout of the text it was made for (by the text's hash).
    job: Option<(u64, LayoutJob)>,
}

type HlState = (syntect::parsing::ParseState, syntect::highlighting::HighlightState);

/// Lines between two kept parser states (a state is a few hundred bytes to kilobytes; keeping
/// one per line cost megabytes on a long file, and colouring 16 lines again takes ~1 ms).
const CHECKPOINT: usize = 16;

struct HlLine {
    /// The line's text (hash and length), to tell whether it changed.
    hash: (u64, usize),
    /// The state the line starts in, kept on every `CHECKPOINT`th line.
    start: Option<HlState>,
    /// Colour and byte range of each piece of the line.
    spans: Vec<(Color32, usize, usize)>,
}

fn line_hash(line: &str) -> (u64, usize) {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    line.hash(&mut h);
    (h.finish(), line.len())
}

/// The layout of `text` with its syntax colours, coloured incrementally (`Highlight`).
pub fn cached_layout(
    cache: &mut Highlight,
    text: &str,
    syntax: &str,
    dark: bool,
    font: FontId,
    plain: Color32,
) -> LayoutJob {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    text.hash(&mut hasher);
    let text_key = hasher.finish();
    let key = (syntax.to_string(), dark, font.size.to_bits());
    if cache.key.as_ref() != Some(&key) {
        *cache = Highlight {
            key: Some(key),
            ..Default::default()
        };
    }
    if let Some((k, job)) = cache.job.as_ref()
        && *k == text_key
    {
        return job.clone();
    }
    let h = highlighting();
    let syntax_ref = (text.len() <= HIGHLIGHT_AT_MOST)
        .then(|| {
            h.syntaxes
                .find_syntax_by_extension(syntax)
                .or_else(|| h.syntaxes.find_syntax_by_token(syntax))
        })
        .flatten();
    let Some(syntax_ref) = syntax_ref else {
        let mut job = LayoutJob::default();
        job.append(text, 0.0, egui::TextFormat::simple(font, plain));
        cache.job = Some((text_key, job.clone()));
        return job;
    };
    let highlighter = syntect::highlighting::Highlighter::new(theme(&h, dark));
    let lines: Vec<&str> = syntect::util::LinesWithEndings::from(text).collect();
    let hashes: Vec<(u64, usize)> = lines.iter().map(|l| line_hash(l)).collect();

    // Lines up to the first change keep their colours; colouring starts again from the last
    // kept state at or before it (or from the end, for text added after the last line).
    let mut old = std::mem::take(&mut cache.lines);
    let first = old.iter().zip(&hashes).take_while(|(o, n)| o.hash == **n).count();
    let (resume, mut state) = if first > 0 && first == old.len() && cache.end.is_some() {
        (first, cache.end.clone().unwrap_or_else(|| initial(syntax_ref, &highlighter)))
    } else {
        old[..first.min(old.len())]
            .iter()
            .enumerate()
            .rev()
            .find_map(|(i, l)| l.start.clone().map(|s| (i, s)))
            .unwrap_or_else(|| (0, initial(syntax_ref, &highlighter)))
    };
    let tail = old.split_off(first.min(old.len()));
    old.truncate(resume);
    let mut out = old;
    let delta = lines.len() as isize - (first + tail.len()) as isize;
    let mut reused = false;
    let mut j = resume;
    while j < lines.len() {
        // Past the changed lines, an old line with the same text that kept its starting state,
        // and the same state now: it and everything after it are as they were.
        if j > first {
            let k = j as isize - delta - first as isize;
            if k >= 0
                && let Some(cand) = tail.get(k as usize)
                && cand.hash == hashes[j]
                && cand.start.as_ref() == Some(&state)
            {
                out.extend(tail.into_iter().skip(k as usize));
                reused = true;
                break;
            }
        }
        let start = (j % CHECKPOINT == 0).then(|| state.clone());
        let line = lines[j];
        let spans = match state.0.parse_line(line, &h.syntaxes) {
            Ok(ops) => syntect::highlighting::RangedHighlightIterator::new(&mut state.1, &ops, line, &highlighter)
                .map(|(style, _, r)| {
                    let c = style.foreground;
                    (Color32::from_rgb(c.r, c.g, c.b), r.start, r.end)
                })
                .collect(),
            Err(_) => vec![(plain, 0, line.len())],
        };
        out.push(HlLine { hash: hashes[j], start, spans });
        j += 1;
    }
    if !reused {
        cache.end = Some(state);
    }
    cache.lines = out;

    let mut job = LayoutJob::default();
    for (line, hl) in lines.iter().zip(&cache.lines) {
        for &(color, a, b) in &hl.spans {
            if let Some(piece) = line.get(a..b) {
                job.append(piece, 0.0, egui::TextFormat::simple(font.clone(), color));
            }
        }
    }
    cache.job = Some((text_key, job.clone()));
    job
}

fn initial(syntax: &syntect::parsing::SyntaxReference, highlighter: &syntect::highlighting::Highlighter) -> HlState {
    (
        syntect::parsing::ParseState::new(syntax),
        syntect::highlighting::HighlightState::new(highlighter, syntect::parsing::ScopeStack::new()),
    )
}

/// Files of a project for the quick-open list: `git ls-files` (tracked and untracked, without
/// ignored ones) when it is a repository, else a bounded walk that skips build folders.
pub fn project_files(root: &Path, limit: usize) -> Vec<PathBuf> {
    let git = crate::platform::command("git")
        .arg("-C")
        .arg(root)
        .args(["ls-files", "-z", "--cached", "--others", "--exclude-standard"])
        .env("GIT_OPTIONAL_LOCKS", "0")
        .output();
    if let Ok(out) = git
        && out.status.success()
    {
        return out
            .stdout
            .split(|b| *b == 0)
            .filter(|s| !s.is_empty())
            .take(limit)
            .map(|s| PathBuf::from(String::from_utf8_lossy(s).into_owned()))
            .collect();
    }
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else { continue };
        for e in entries.flatten() {
            let p = e.path();
            let name = e.file_name().to_string_lossy().into_owned();
            if name.starts_with('.') || matches!(name.as_str(), "target" | "node_modules" | "dist" | "build") {
                continue;
            }
            if p.is_dir() {
                stack.push(p);
            } else if let Ok(rel) = p.strip_prefix(root) {
                out.push(rel.to_path_buf());
                if out.len() >= limit {
                    return out;
                }
            }
        }
    }
    out
}

/// A `path[:line[:col]]` reference found in terminal output, resolved against `cwd`.
pub fn parse_location(word: &str, cwd: &Path) -> Option<(PathBuf, Option<usize>)> {
    let word = word
        .trim_matches(|c: char| matches!(c, '(' | ')' | '[' | ']' | '"' | '\'' | ',' | '`' | '<' | '>'))
        .trim_end_matches(['.', ':']);
    // Windows: the drive's colon (`C:\src\main.rs:12`) belongs to the path.
    let b = word.as_bytes();
    let drive = if cfg!(windows) && b.len() > 2 && b[0].is_ascii_alphabetic() && b[1] == b':' && matches!(b[2], b'\\' | b'/') {
        2
    } else {
        0
    };
    let mut parts = word[drive..].splitn(3, ':');
    let path = format!("{}{}", &word[..drive], parts.next()?);
    let line = parts.next().and_then(|l| l.parse::<usize>().ok());
    if path.is_empty() {
        return None;
    }
    let p = crate::paths::expand_tilde(&path);
    let full = if p.is_absolute() { p } else { cwd.join(p) };
    full.is_file().then_some((full, line))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn edits_survive_outside_changes_and_saves_never_clobber() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.rs");
        std::fs::write(&path, "fn main() {}\n").unwrap();
        let mut f = load(&path).ok().unwrap();
        assert!(!f.dirty());
        // A clean buffer follows the disk.
        std::thread::sleep(std::time::Duration::from_millis(20));
        std::fs::write(&path, "fn main() { agent(); }\n").unwrap();
        assert!(on_disk_change(&mut f));
        assert_eq!(f.text, "fn main() { agent(); }\n");
        assert!(!f.conflict);
        // A dirty buffer is never overwritten: it is marked, and saving refuses.
        f.text.push_str("// mine\n");
        std::thread::sleep(std::time::Duration::from_millis(20));
        std::fs::write(&path, "fn main() { agent2(); }\n").unwrap();
        assert!(on_disk_change(&mut f));
        assert!(f.conflict);
        assert!(f.text.ends_with("// mine\n"));
        assert!(matches!(save(&mut f, false), Err(SaveError::ChangedOnDisk)));
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "fn main() { agent2(); }\n");
        // The user chooses: keep mine.
        assert!(save(&mut f, true).is_ok());
        assert!(std::fs::read_to_string(&path).unwrap().ends_with("// mine\n"));
        assert!(!f.conflict && !f.dirty());
    }

    #[test]
    fn a_change_in_the_same_tick_is_not_overwritten() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.rs");
        std::fs::write(&path, "one\n").unwrap();
        let mut f = load(&path).ok().unwrap();
        let stamp = std::fs::metadata(&path).unwrap().modified().unwrap();
        // An agent writes, and the time stamp reads as before (coarse file system clock).
        std::fs::write(&path, "agent\n").unwrap();
        std::fs::File::options().write(true).open(&path).unwrap().set_modified(stamp).unwrap();
        // The watcher's event: the buffer had no edits, so it follows.
        assert!(on_disk_change(&mut f));
        assert_eq!(f.text, "agent\n");
        // Another write in the same tick, not seen yet, while there are edits: saving refuses.
        std::fs::write(&path, "agent 2\n").unwrap();
        std::fs::File::options().write(true).open(&path).unwrap().set_modified(stamp).unwrap();
        f.text = "mine\n".into();
        assert!(matches!(save(&mut f, false), Err(SaveError::ChangedOnDisk)));
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "agent 2\n");
    }

    #[test]
    fn saving_keeps_permissions_and_links_and_leaves_no_temporary_file() {
        let dir = tempfile::tempdir().unwrap();
        let real = dir.path().join("run.sh");
        std::fs::write(&real, "echo 1\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&real, std::fs::Permissions::from_mode(0o755)).unwrap();
            let link = dir.path().join("link.sh");
            std::os::unix::fs::symlink(&real, &link).unwrap();
            let mut f = load(&link).ok().unwrap();
            f.text = "echo 2\n".into();
            assert!(save(&mut f, false).is_ok());
            assert!(std::fs::symlink_metadata(&link).unwrap().file_type().is_symlink());
            assert_eq!(std::fs::read_to_string(&real).unwrap(), "echo 2\n");
            assert_eq!(std::fs::metadata(&real).unwrap().permissions().mode() & 0o777, 0o755);
            // A new file gets what any new file gets here (not private).
            let plain = dir.path().join("plain");
            std::fs::write(&plain, "").unwrap();
            let new = dir.path().join("new.txt");
            write_atomic(&new, b"x").unwrap();
            let mode = |p: &Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode(&new), mode(&plain));
            // A file nobody may write is not replaced by the rename.
            std::fs::set_permissions(&plain, std::fs::Permissions::from_mode(0o444)).unwrap();
            assert!(write_atomic(&plain, b"x").is_err());
            assert_eq!(std::fs::read(&plain).unwrap(), b"");
        }
        let mut f = load(&real).ok().unwrap();
        f.text = "echo 3\n".into();
        assert!(save(&mut f, false).is_ok());
        assert_eq!(std::fs::read_to_string(&real).unwrap(), "echo 3\n");
        let names: Vec<String> = std::fs::read_dir(dir.path()).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
        assert!(!names.iter().any(|n| n.ends_with(".tc-tmp")), "{names:?}");
    }

    #[test]
    fn crlf_files_stay_crlf() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.txt");
        std::fs::write(&path, "a\r\nb\r\n").unwrap();
        let mut f = load(&path).ok().unwrap();
        assert!(f.crlf);
        // A line typed here ends with LF; on disk every line ends with CR LF.
        f.text.push_str("c\n");
        assert!(save(&mut f, false).is_ok());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "a\r\nb\r\nc\r\n");
        assert!(!f.dirty());
        // What was written is what was saved: no conflict the next time.
        f.text.push('d');
        assert!(save(&mut f, false).is_ok());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "a\r\nb\r\nc\r\nd");
        // Converted to LF outside: the buffer follows, and so does its line ending.
        std::thread::sleep(std::time::Duration::from_millis(20));
        std::fs::write(&path, "a\nb\n").unwrap();
        assert!(on_disk_change(&mut f));
        assert!(!f.crlf);
        std::fs::write(&path, "x\r\n").unwrap();
        reload(&mut f);
        assert!(f.crlf);
        assert_eq!(to_crlf("a\nb\r\nc"), "a\r\nb\r\nc");
        assert!(matches!(to_crlf("a\r\n"), std::borrow::Cow::Borrowed(_)));
    }

    #[test]
    fn find_ignores_case_and_steps_around_the_caret() {
        let text = "fn Main() {}\nlet main = 1; // é main\n";
        let m = find_all(text, "MAIN");
        assert_eq!(m.len(), 3);
        assert_eq!(m[0], Match { line: 0, col: (3, 7), chars: (3, 7) });
        assert_eq!(m[1].line, 1);
        assert_eq!(m[1].col, (4, 8));
        // After the multi-byte é, columns still count characters.
        assert_eq!(m[2].col, (19, 23));
        assert_eq!(&text.chars().skip(m[2].chars.0).take(4).collect::<String>(), "main");
        let mut f = Find { query: "main".into(), matches: m, ..Default::default() };
        // From a caret in line 1: the next is the second match, then around.
        assert_eq!(f.step(true, 14).map(|m| m.line), Some(1));
        assert_eq!(f.current, Some(1));
        assert_eq!(f.step(true, 0).map(|m| m.col.0), Some(19));
        assert_eq!(f.step(true, 0).map(|m| m.col.0), Some(3));
        assert_eq!(f.step(false, 0).map(|m| m.col.0), Some(19));
        assert!(find_all("abc", "").is_empty());
        let g = Find { query: ":42".into(), ..Default::default() };
        assert_eq!(g.goto_line(), Some(41));
    }

    #[test]
    fn closing_remembers_recent_files_and_keeps_the_active_one() {
        let dir = tempfile::tempdir().unwrap();
        let mut e = Editor::default();
        for n in ["a", "b", "c"] {
            let p = dir.path().join(n);
            std::fs::write(&p, n).unwrap();
            e.files.push(load(&p).ok().unwrap());
        }
        e.active = 2;
        e.close(0);
        assert_eq!(e.active, 1);
        assert_eq!(e.file().unwrap().name(), "c");
        e.close(1);
        assert_eq!(e.active, 0);
        assert_eq!(e.recent, [dir.path().join("c"), dir.path().join("a")]);
    }

    #[test]
    fn binary_large_and_locations() {
        let dir = tempfile::tempdir().unwrap();
        let bin = dir.path().join("x.bin");
        std::fs::write(&bin, [0u8, 1, 2, 3]).unwrap();
        assert!(matches!(load(&bin), Err(OpenError::Binary)));
        let src = dir.path().join("src");
        std::fs::create_dir(&src).unwrap();
        std::fs::write(src.join("main.rs"), "x").unwrap();
        assert_eq!(
            parse_location("src/main.rs:42:7", dir.path()),
            Some((src.join("main.rs"), Some(42)))
        );
        assert_eq!(
            parse_location("(src/main.rs)", dir.path()),
            Some((src.join("main.rs"), None))
        );
        assert!(parse_location("nope.rs:1", dir.path()).is_none());
        // Windows: a drive is not a line number.
        #[cfg(windows)]
        {
            let abs = format!("{}:3", src.join("main.rs").display());
            assert_eq!(parse_location(&abs, Path::new(r"C:\")), Some((src.join("main.rs"), Some(3))));
        }
        assert_eq!(syntax_for(Path::new("a.tsx")), "js");
        assert_eq!(syntax_for(Path::new("Cargo.toml")), "ini");
    }

    #[test]
    fn highlighting_colours_rust() {
        let job = layout("fn main() { let x = 1; }\n", "rs", true, FontId::monospace(12.0), Color32::WHITE);
        let colors: std::collections::HashSet<_> = job.sections.iter().map(|s| s.format.color).collect();
        assert!(colors.len() > 2, "{colors:?}");
    }
}


#[cfg(test)]
mod bench_highlight {
    /// `TC_BENCH_FILE=path cargo test --lib bench_highlight -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn time_highlighting_a_file() {
        let Ok(path) = std::env::var("TC_BENCH_FILE") else { return };
        let text = std::fs::read_to_string(&path).unwrap();
        let ext = std::path::Path::new(&path).extension().unwrap().to_string_lossy().to_string();
        let font = eframe::egui::FontId::monospace(13.0);
        let rss = || tc_agent::proc::own_resident_bytes().unwrap_or(0) as f64 / 1048576.0;
        eprintln!("BENCH rss start: {:.1} MB", rss());
        let _ = super::highlighting();
        eprintln!("BENCH rss syntax set loaded: {:.1} MB", rss());
        let _ = super::layout(&text, &ext, true, font.clone(), eframe::egui::Color32::WHITE);
        let t = std::time::Instant::now();
        for _ in 0..5 {
            let _ = super::layout(&text, &ext, true, font.clone(), eframe::egui::Color32::WHITE);
        }
        eprintln!("BENCH layout {} bytes: {:?} per call", text.len(), t.elapsed() / 5);
        let mb = || tc_agent::proc::own_resident_bytes().unwrap_or(0) as f64 / 1048576.0;
        eprintln!("BENCH rss after highlighting: {:.1} MB", mb());
        // Typing in the middle: one char at a time, incremental.
        let mut cache = super::Highlight::default();
        let mut text = text;
        let _ = super::cached_layout(&mut cache, &text, &ext, true, font.clone(), eframe::egui::Color32::WHITE);
        let at = text.len() / 2;
        let at = (0..=at).rev().find(|i| text.is_char_boundary(*i)).unwrap();
        let t = std::time::Instant::now();
        for i in 0..20 {
            text.insert(at + i, 'x');
            let _ = super::cached_layout(&mut cache, &text, &ext, true, font.clone(), eframe::egui::Color32::WHITE);
        }
        eprintln!("BENCH incremental: {:?} per key", t.elapsed() / 20);
        // The result is the same as colouring from scratch.
        let full = super::layout(&text, &ext, true, font.clone(), eframe::egui::Color32::WHITE);
        let inc = super::cached_layout(&mut cache, &text, &ext, true, font.clone(), eframe::egui::Color32::WHITE);
        assert_eq!(full.text, inc.text);
        let colors = |j: &eframe::egui::text::LayoutJob| j.sections.iter().map(|s| (s.byte_range.clone(), s.format.color)).collect::<Vec<_>>();
        assert_eq!(colors(&full), colors(&inc));
    }
}

#[cfg(test)]
mod incremental_highlight {
    use eframe::egui::{Color32, FontId, text::LayoutJob};

    fn colors(j: &LayoutJob) -> Vec<(String, Color32)> {
        j.sections.iter().map(|s| (format!("{:?}", s.byte_range), s.format.color)).collect()
    }

    #[test]
    fn edits_colour_the_same_as_colouring_from_scratch() {
        let font = FontId::monospace(13.0);
        let mut text: String = (0..200).map(|i| format!("fn f{i}() -> i32 {{ let s = \"x{i}\"; {i} }}\n")).collect();
        let mut cache = super::Highlight::default();
        let check = |text: &str, cache: &mut super::Highlight| {
            let inc = super::cached_layout(cache, text, "rs", true, font.clone(), Color32::WHITE);
            let full = super::layout(text, "rs", true, font.clone(), Color32::WHITE);
            assert_eq!(inc.text, full.text);
            assert_eq!(colors(&inc), colors(&full));
        };
        check(&text, &mut cache);
        // A char typed in the middle.
        let mid = text.len() / 2;
        text.insert(mid, 'z');
        check(&text, &mut cache);
        // A block comment opened: everything after it changes colour.
        text.insert_str(text.find("fn f50").unwrap(), "/* ");
        check(&text, &mut cache);
        // And closed again further down.
        text.insert_str(text.find("fn f60").unwrap(), "*/ ");
        check(&text, &mut cache);
        // Lines removed and added.
        let a = text.find("fn f10").unwrap();
        let b = text.find("fn f20").unwrap();
        text.replace_range(a..b, "");
        check(&text, &mut cache);
        text.insert_str(0, "// new first line\n// and another\n");
        check(&text, &mut cache);
        // Text appended at the end, then all of it removed.
        text.push_str("fn last() {}");
        check(&text, &mut cache);
        text.clear();
        check(&text, &mut cache);
    }
}
