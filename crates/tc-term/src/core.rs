//! One terminal: an alacritty `Term` shared between the PTY reader task (which feeds it the
//! program's output) and the UI (which draws it and sends input).
//!
//! Events raised by the parser are queued through a channel and handled by the reader right
//! after `advance` (outside the lock): replies such as device attributes, cursor position and
//! colour queries go straight back to the program even when the window is not repainting.

use std::sync::Arc;
use std::sync::mpsc;
use std::time::Instant;

use alacritty_terminal::event::{Event, EventListener, WindowSize};
use alacritty_terminal::grid::{Dimensions, Row};
use alacritty_terminal::index::{Column, Line, Point};
use alacritty_terminal::sync::FairMutex;
use alacritty_terminal::term::cell::Cell;
use alacritty_terminal::term::{Config, Term, TermMode};
use alacritty_terminal::vte::ansi::{
    ClearMode, CursorShape, CursorStyle, Handler, KeyboardModes, Processor, Rgb,
};
use egui::Color32;
use parking_lot::Mutex;

use super::scheme::Scheme;
use crate::input::KeyModes;
use crate::osc::{OscScanner, OscSignal, PromptMark, Redraw};
use crate::prefs::CursorStyle as CursorPref;

pub const MIN_COLS: usize = 2;
pub const MIN_ROWS: usize = 1;

#[derive(Clone)]
pub struct Listener(mpsc::Sender<Event>);

impl EventListener for Listener {
    fn send_event(&self, event: Event) {
        // Never touch the Term here: `advance` holds its lock while emitting events.
        let _ = self.0.send(event);
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GridSize {
    pub cols: usize,
    pub rows: usize,
}

impl Dimensions for GridSize {
    fn total_lines(&self) -> usize {
        self.rows
    }
    fn screen_lines(&self) -> usize {
        self.rows
    }
    fn columns(&self) -> usize {
        self.cols
    }
}

/// State the UI reads that the parser produces (title, OSC 52 copies).
#[derive(Default)]
pub struct UiNotes {
    pub title: Option<String>,
    pub clipboard: Option<String>,
    /// "Clear Scrollback" was chosen on the alternate screen: clear when it is left.
    pub clear_history_pending: bool,
}

/// Colours used to answer OSC 4/10/11 queries (kept in sync with the active scheme).
#[derive(Clone)]
pub struct QueryPalette {
    pub ansi: [Color32; 16],
    pub fg: Color32,
    pub bg: Color32,
    pub cursor: Color32,
}

impl From<&Scheme> for QueryPalette {
    fn from(s: &Scheme) -> Self {
        Self {
            ansi: s.ansi,
            fg: s.foreground,
            bg: s.background,
            cursor: s.cursor,
        }
    }
}

/// Where the shell's prompt is, from its shell integration marks (OSC 133), recorded by the
/// reader as the marks are parsed so it matches the grid. A resize clears the prompt before the
/// grid reflows it (a shell redraws its prompt where its old width put it, leaving pieces of the
/// old one behind) and keeps the cursor on the same row of it, so the shell redraws it cleanly.
#[derive(Default)]
pub struct PromptState {
    /// The shell waits at its prompt (a prompt or input mark, no command since).
    pub at_prompt: bool,
    /// How the shell redraws its prompt (from the prompt mark's `redraw=` option).
    pub redraw: Redraw,
    /// Line (history lines + screen line) where the prompt starts, and where the input starts.
    pub start: Option<usize>,
    pub input: Option<usize>,
    /// The prompt was cleared by a resize the shell has not heard of yet, and what was there.
    cleared: Option<ClearedPrompt>,
    /// History lines the terminal keeps (its own setting is private to alacritty).
    limit: usize,
    /// The main screen's history has room past `limit` (`Feeder::make_room`): while a chunk is
    /// parsed, or while a full-screen program started in it keeps the main screen out of reach.
    raised: bool,
}

impl PromptState {
    /// The `lines` oldest history lines were dropped: the anchors move up with the rest, and are
    /// forgotten when their own line went.
    pub(crate) fn dropped(&mut self, lines: usize) {
        self.start = self.start.and_then(|a| a.checked_sub(lines));
        self.input = self.input.and_then(|a| a.checked_sub(lines));
    }

    fn forget(&mut self) {
        self.start = None;
        self.input = None;
    }

    /// Puts the history back to `limit` after it was given room, dropping the oldest lines.
    fn trim(&mut self, term: &mut Term<Listener>) {
        let over = term.grid().history_size().saturating_sub(self.limit);
        term.grid_mut().update_history(self.limit);
        self.dropped(over);
        self.raised = false;
    }
}

/// The rows a resize cleared, kept to put back when the size settles where it was (the shell
/// then gets no SIGWINCH and would not redraw).
struct ClearedPrompt {
    rows: Vec<Row<Cell>>,
    cols: usize,
    cursor_col: usize,
    /// Rows from the first cleared one down to the cursor.
    offset: usize,
}

pub struct TermShared {
    pub term: FairMutex<Term<Listener>>,
    pub notes: Mutex<UiNotes>,
    pub prompt: Mutex<PromptState>,
    /// Cell size in pixels and grid size, for CSI 14 t and the PTY's size.
    pub window: Mutex<WindowSize>,
    pub palette: Mutex<QueryPalette>,
    /// The options the terminal has (alacritty does not say), for `clear_keyboard_modes`.
    config: Mutex<Config>,
}

pub type SharedTerm = Arc<TermShared>;

/// The terminal's options. The kitty keyboard protocol is always on: alacritty empties its
/// stacks whenever the option changes, so every `Config` given to a terminal has it.
pub fn term_config(scrollback: usize, cursor: CursorPref, blink: bool) -> Config {
    let shape = match cursor {
        CursorPref::Block => CursorShape::Block,
        CursorPref::Beam => CursorShape::Beam,
        CursorPref::Underline => CursorShape::Underline,
    };
    Config {
        scrolling_history: scrollback,
        default_cursor_style: CursorStyle {
            shape,
            blinking: blink,
        },
        kitty_keyboard: true,
        ..Config::default()
    }
}

/// Creates a terminal and the receiving end of its event queue (owned by the reader).
pub fn new_terminal(
    config: Config,
    size: GridSize,
    palette: QueryPalette,
) -> (SharedTerm, mpsc::Receiver<Event>) {
    let (tx, rx) = mpsc::channel();
    let limit = config.scrolling_history;
    let config = Config {
        kitty_keyboard: true,
        ..config
    };
    let term = Term::new(config.clone(), &size, Listener(tx));
    let shared = Arc::new(TermShared {
        term: FairMutex::new(term),
        notes: Mutex::new(UiNotes::default()),
        prompt: Mutex::new(PromptState {
            limit,
            ..Default::default()
        }),
        window: Mutex::new(WindowSize {
            num_lines: size.rows as u16,
            num_cols: size.cols as u16,
            cell_width: 8,
            cell_height: 16,
        }),
        palette: Mutex::new(palette),
        config: Mutex::new(config),
    });
    (shared, rx)
}

/// Empties both screens' keyboard mode stacks: alacritty does that when the protocol is turned
/// off in its options, and has no other way to.
fn clear_keyboard_modes(shared: &TermShared, term: &mut Term<Listener>) {
    let config = shared.config.lock().clone();
    term.set_options(Config {
        kitty_keyboard: false,
        ..config.clone()
    });
    term.set_options(config);
}

/// Keyboard mode pushes allowed before the stacks are emptied (`KeyboardPushes`), so that
/// neither ever holds more than this and one.
const MAX_PUSHES: usize = 4000;

/// Counts the kitty keyboard mode pushes (`CSI > flags u`) in the output since the stacks were
/// last emptied. alacritty 0.26 keeps up to 4096 per screen; one push more makes it take the
/// oldest from the title stack instead, which panics when that is empty (the pane's reader would
/// stop). Matches what `vte` takes for the sequence: control characters and bytes it ignores
/// may sit inside it, and an ESC anywhere starts a new one.
#[derive(Default)]
struct KeyboardPushes {
    state: PushState,
    count: usize,
}

#[derive(Clone, Copy, Default, PartialEq, Eq)]
enum PushState {
    #[default]
    Ground,
    Esc,
    Csi,
    Params,
}

impl KeyboardPushes {
    /// Scans `bytes` up to the push that reaches `MAX_PUSHES`. Returns how many bytes it went
    /// through, and whether the stacks are to be emptied there.
    fn scan(&mut self, bytes: &[u8]) -> (usize, bool) {
        let mut i = 0;
        while i < bytes.len() {
            if self.state == PushState::Ground {
                let Some(p) = memchr::memchr(0x1b, &bytes[i..]) else {
                    break;
                };
                i += p;
            }
            let b = bytes[i];
            i += 1;
            self.state = match (self.state, b) {
                (_, 0x1b) => PushState::Esc,
                (_, 0x18 | 0x1a) => PushState::Ground,
                (state, 0x00..=0x1f | 0x7f..) => state,
                (PushState::Esc, b'[') => PushState::Csi,
                (PushState::Csi, b'>') => PushState::Params,
                (PushState::Params, b'0'..=b'9' | b';' | b':') => PushState::Params,
                (PushState::Params, b'u') => {
                    self.count += 1;
                    if self.count >= MAX_PUSHES {
                        self.count = 0;
                        self.state = PushState::Ground;
                        return (i, true);
                    }
                    PushState::Ground
                }
                _ => PushState::Ground,
            };
        }
        (bytes.len(), false)
    }
}

fn rgb(c: Color32) -> Rgb {
    Rgb {
        r: c.r(),
        g: c.g(),
        b: c.b(),
    }
}

/// Most bytes parsed per hold of the terminal lock. A pane flooding output (`cat` of a big
/// file) releases the lock between chunks, so the UI and other panes never wait long.
pub const FEED_CHUNK: usize = 64 * 1024;

/// What one `Feeder::feed` produced.
#[derive(Debug, Default)]
pub struct Fed {
    /// Replies to the parser's queries, to write back to the program.
    pub reply: Vec<u8>,
    /// The screen changed (not everything was held back by a synchronized update).
    pub repaint: bool,
    /// OSC signals `vte` does not report (notifications, shell integration, the directory).
    pub signals: Vec<OscSignal>,
    /// The program rang the bell.
    pub bell: bool,
}

/// Feeds program output into a terminal and answers the parser's queries.
pub struct Feeder {
    pub shared: SharedTerm,
    parser: Processor,
    osc: OscScanner,
    pushes: KeyboardPushes,
    events: mpsc::Receiver<Event>,
    bell: bool,
}

impl Feeder {
    pub fn new(shared: SharedTerm, events: mpsc::Receiver<Event>) -> Self {
        Self {
            shared,
            parser: Processor::new(),
            osc: OscScanner::new(),
            pushes: KeyboardPushes::default(),
            events,
            bell: false,
        }
    }

    /// Parses `bytes` in chunks of at most `FEED_CHUNK`, releasing the terminal lock between
    /// them. The OSC scanner sees the bytes first; the parser gets them unchanged, stopping
    /// after each prompt mark to note where the cursor is (`PromptState`).
    pub fn feed(&mut self, bytes: &[u8]) -> Fed {
        let mut found = Vec::new();
        self.osc.scan_at(bytes, &mut found);
        let mut repaint = false;
        let mut done = 0;
        for (end, signal) in &found {
            if matches!(signal, OscSignal::Prompt(_) | OscSignal::PromptRedraw(_)) && *end > done {
                repaint |= self.advance(&bytes[done..*end]);
                done = *end;
            }
            match signal {
                OscSignal::Prompt(mark) => self.note_prompt(mark),
                OscSignal::PromptRedraw(r) => self.shared.prompt.lock().redraw = *r,
                _ => {}
            }
        }
        repaint |= self.advance(&bytes[done..]);
        let signals = found.into_iter().map(|(_, s)| s).collect();
        let reply = self.drain_events();
        Fed {
            reply,
            repaint,
            signals,
            bell: std::mem::take(&mut self.bell),
        }
    }

    /// Parses `bytes`; returns whether the screen changed.
    fn advance(&mut self, mut bytes: &[u8]) -> bool {
        let mut repaint = false;
        while !bytes.is_empty() {
            // A chunk, or up to the keyboard mode push after which the stacks are emptied.
            let (len, full) = self.pushes.scan(&bytes[..bytes.len().min(FEED_CHUNK)]);
            let (chunk, rest) = bytes.split_at(len);
            bytes = rest;
            let mut term = self.shared.term.lock();
            let room = self.make_room(&mut term);
            self.parser.advance(&mut *term, chunk);
            self.count_dropped(&mut term, room);
            if !term.mode().contains(TermMode::ALT_SCREEN) {
                let pending = std::mem::take(&mut self.shared.notes.lock().clear_history_pending);
                if pending {
                    self.shared.prompt.lock().dropped(term.grid().history_size());
                    term.grid_mut().clear_history();
                }
            }
            if full {
                // `MAX_PUSHES` keyboard mode pushes since the stacks were last emptied: they are
                // emptied, keeping the flags in effect. The pushes a synchronized update holds
                // were counted too: they are parsed first.
                if self.parser.sync_bytes_count() > 0 {
                    let room = self.make_room(&mut term);
                    self.parser.stop_sync(&mut *term);
                    self.count_dropped(&mut term, room);
                }
                let flags = KeyModes::of(*term.mode()).kitty;
                clear_keyboard_modes(&self.shared, &mut term);
                if flags != 0 {
                    term.push_keyboard_mode(KeyboardModes::from_bits_truncate(flags));
                }
            }
            drop(term);
            repaint |= self.parser.sync_bytes_count() < chunk.len();
        }
        repaint
    }

    /// While the shell waits at its prompt, gives the history room for a screen of new lines and
    /// returns its size before. A full history drops its oldest line for each one scrolled in:
    /// the prompt would move up without the history growing, and its anchors (`PromptState`)
    /// would point below it. With room the lines are counted (`count_dropped`), then dropped.
    fn make_room(&self, term: &mut Term<Listener>) -> Option<usize> {
        let mut prompt = self.shared.prompt.lock();
        if term.mode().contains(TermMode::ALT_SCREEN)
            || !prompt.at_prompt
            || (prompt.start.is_none() && prompt.input.is_none())
        {
            return None;
        }
        let room = prompt.limit + term.screen_lines();
        term.grid_mut().update_history(room);
        prompt.raised = true;
        Some(term.grid().history_size())
    }

    /// After `make_room` (its result is `before`): the history goes back to its limit, the
    /// anchors moving with its lines.
    fn count_dropped(&self, term: &mut Term<Listener>, before: Option<usize>) {
        let mut prompt = self.shared.prompt.lock();
        if !prompt.raised {
            return;
        }
        if term.mode().contains(TermMode::ALT_SCREEN) {
            // A full-screen program took over: the main screen's history is out of reach until
            // it leaves, and its lines go uncounted.
            prompt.forget();
            return;
        }
        let now = term.grid().history_size();
        // Cleared (CSI 3 J, a reset), out of room (a whole screen scrolled, the prompt too), or
        // back from a full-screen program: the lines were not all counted.
        if before.is_none_or(|before| now < before) || now >= prompt.limit + term.screen_lines() {
            prompt.forget();
        }
        prompt.trim(term);
    }

    /// Records a prompt mark at the cursor (the parser has just passed it). Inside a
    /// synchronized update the grid lags behind the bytes: the position is not known then.
    fn note_prompt(&mut self, mark: &PromptMark) {
        let term = self.shared.term.lock();
        let mut prompt = self.shared.prompt.lock();
        let held = self.parser.sync_bytes_count() > 0;
        let alt = term.mode().contains(TermMode::ALT_SCREEN);
        let at = term.grid().history_size() + term.grid().cursor.point.line.0.max(0) as usize;
        match mark {
            PromptMark::PromptStart | PromptMark::CommandStart if held || alt => {
                prompt.at_prompt = false;
            }
            PromptMark::PromptStart => {
                prompt.at_prompt = true;
                prompt.start = Some(at);
                prompt.input = None;
            }
            PromptMark::CommandStart => {
                prompt.at_prompt = true;
                prompt.input = Some(at);
            }
            PromptMark::CommandExecuted | PromptMark::CommandFinished(_) => prompt.at_prompt = false,
        }
    }

    /// Deadline of a pending synchronized update (DEC 2026), if any.
    pub fn sync_deadline(&self) -> Option<Instant> {
        self.parser.sync_timeout().sync_timeout()
    }

    /// Flushes a synchronized update whose deadline passed.
    pub fn flush_sync(&mut self) -> Vec<u8> {
        {
            let mut term = self.shared.term.lock();
            // The held bytes are parsed now: their lines are counted as in `advance`.
            let room = self.make_room(&mut term);
            self.parser.stop_sync(&mut *term);
            self.count_dropped(&mut term, room);
        }
        self.drain_events()
    }

    fn drain_events(&mut self) -> Vec<u8> {
        let mut reply = Vec::new();
        while let Ok(event) = self.events.try_recv() {
            match event {
                Event::PtyWrite(s) => reply.extend_from_slice(s.as_bytes()),
                Event::ColorRequest(index, format) => {
                    let custom = self.shared.term.lock().colors()[index];
                    let color = custom.unwrap_or_else(|| {
                        let p = self.shared.palette.lock();
                        rgb(match index {
                            0..=15 => p.ansi[index],
                            16..=255 => super::scheme::xterm_256(index as u8),
                            256 => p.fg,
                            257 => p.bg,
                            _ => p.cursor,
                        })
                    });
                    reply.extend_from_slice(format(color).as_bytes());
                }
                Event::TextAreaSizeRequest(format) => {
                    let ws = *self.shared.window.lock();
                    reply.extend_from_slice(format(ws).as_bytes());
                }
                Event::Title(t) => self.shared.notes.lock().title = Some(t),
                Event::ResetTitle => self.shared.notes.lock().title = None,
                Event::Bell => self.bell = true,
                Event::ClipboardStore(_, text) => self.shared.notes.lock().clipboard = Some(text),
                _ => {}
            }
        }
        reply
    }
}

/// The last `max` non-empty lines of the visible screen, for compact summaries.
pub fn tail_text(shared: &TermShared, max: usize) -> Vec<String> {
    use alacritty_terminal::index::{Column, Line};
    let term = shared.term.lock();
    let mut lines: Vec<String> = (0..term.screen_lines() as i32)
        .map(|l| {
            let row = &term.grid()[Line(l)];
            (0..term.columns())
                .map(|c| row[Column(c)].c)
                .collect::<String>()
                .trim_end()
                .to_string()
        })
        .filter(|l| !l.trim().is_empty())
        .collect();
    let skip = lines.len().saturating_sub(max);
    lines.drain(..skip);
    lines
}

/// Approximate memory held by the scrollback, in bytes (cells are about 24 bytes).
pub fn scrollback_bytes(shared: &TermShared) -> usize {
    let term = shared.term.lock();
    term.grid().history_size() * term.columns() * 24
}

/// Sets the number of scrollback lines kept, higher or lower (lowering drops the oldest).
pub fn limit_scrollback(shared: &TermShared, lines: usize, cursor: CursorPref, blink: bool) {
    let mut term = shared.term.lock();
    let mut prompt = shared.prompt.lock();
    let before = term.grid().history_size();
    let config = term_config(lines, cursor, blink);
    term.set_options(config.clone());
    *shared.config.lock() = config;
    prompt.dropped(before.saturating_sub(term.grid().history_size()));
    prompt.limit = lines;
    prompt.raised = false;
}

/// Most rows a prompt may span for the resize to clear it (more means the recorded position is
/// stale).
const PROMPT_ROWS: usize = 12;

/// Resizes the grid (clamped to the minimum) and records the pixel size. Returns whether the
/// grid size changed, i.e. the PTY must be told. When the width changes while the shell sits
/// at a prompt it said it redraws (`PromptState`), the prompt is cleared first and the cursor
/// kept on the same row of it: the shell's redraw after SIGWINCH then lands on empty rows.
pub fn resize(shared: &TermShared, size: GridSize, cell_w: f32, cell_h: f32) -> bool {
    let size = GridSize {
        cols: size.cols.max(MIN_COLS),
        rows: size.rows.max(MIN_ROWS),
    };
    let mut window = shared.window.lock();
    window.cell_width = cell_w.round() as u16;
    window.cell_height = cell_h.round() as u16;
    let mut term = shared.term.lock();
    if term.columns() == size.cols && term.screen_lines() == size.rows {
        return false;
    }
    let mut prompt = shared.prompt.lock();
    let reflow = term.columns() != size.cols;
    let offset = if reflow {
        clear_prompt(&mut term, &mut prompt)
    } else {
        None
    };
    let cursor_at = |term: &Term<Listener>| {
        term.grid().history_size() + term.grid().cursor.point.line.0.max(0) as usize
    };
    let before = cursor_at(&term);
    term.resize(size);
    if let Some(offset) = offset {
        // The cleared rows moved with the reflow: the anchor is `offset` rows above the cursor.
        let at = term.grid().history_size()
            + (term.grid().cursor.point.line.0.max(0) as usize).saturating_sub(offset);
        match prompt.redraw {
            Redraw::LastLine => prompt.input = Some(at),
            _ => prompt.start = Some(at),
        }
    } else if !reflow && !term.mode().contains(TermMode::ALT_SCREEN) {
        // Only the height changed: rows went between the screen and the history, the cursor
        // with its line. Those a full history had no room for were dropped.
        prompt.dropped(before.saturating_sub(cursor_at(&term)));
    }
    window.num_cols = size.cols as u16;
    window.num_lines = size.rows as u16;
    true
}

/// Clears the prompt the cursor sits in, from its first row (its input line for a shell that
/// redraws only that) to the end of the screen, keeping what was there the first time; puts
/// the cursor at the start of its row. Returns the cursor's row offset from the first cleared
/// row, or nothing when the prompt is not to be cleared.
fn clear_prompt(term: &mut Term<Listener>, prompt: &mut PromptState) -> Option<usize> {
    if !prompt.at_prompt || term.mode().contains(TermMode::ALT_SCREEN) {
        return None;
    }
    let anchor = match prompt.redraw {
        Redraw::All => prompt.start,
        Redraw::LastLine => prompt.input,
        Redraw::Unknown => None,
    }?;
    let history = term.grid().history_size();
    let cursor = term.grid().cursor.point;
    let row = anchor.checked_sub(history)?;
    let cur = cursor.line.0.max(0) as usize;
    if row > cur || cur - row >= PROMPT_ROWS {
        return None;
    }
    let offset = cur - row;
    if prompt.cleared.is_none() {
        let last = (row + PROMPT_ROWS).min(term.screen_lines());
        prompt.cleared = Some(ClearedPrompt {
            rows: (row..last).map(|r| term.grid()[Line(r as i32)].clone()).collect(),
            cols: term.columns(),
            cursor_col: cursor.column.0,
            offset,
        });
    }
    term.goto(row as i32, 0);
    term.clear_screen(ClearMode::Below);
    term.goto((row + offset) as i32, 0);
    Some(offset)
}

/// The PTY size settled after a resize: `same` when it ended where it was, so the shell gets no
/// SIGWINCH and will not redraw a prompt the resize cleared: what was there is put back (the
/// width is the same as when it was cleared, so it fits as it did).
pub fn settle_prompt(shared: &TermShared, same: bool) {
    let mut term = shared.term.lock();
    let mut prompt = shared.prompt.lock();
    let Some(saved) = prompt.cleared.take() else {
        return;
    };
    if !same || saved.cols != term.columns() || term.mode().contains(TermMode::ALT_SCREEN) {
        return;
    }
    let anchor = match prompt.redraw {
        Redraw::LastLine => prompt.input,
        _ => prompt.start,
    };
    let Some(row) = anchor.and_then(|a| a.checked_sub(term.grid().history_size())) else {
        return;
    };
    let lines = term.screen_lines();
    for (i, r) in saved.rows.into_iter().enumerate() {
        if row + i < lines {
            term.grid_mut()[Line((row + i) as i32)] = r;
        }
    }
    let line = (row + saved.offset).min(lines - 1);
    term.grid_mut().cursor.point = Point::new(Line(line as i32), Column(saved.cursor_col.min(term.columns() - 1)));
}

/// Writes a local status line (e.g. "connection closed") into the terminal in dim text.
pub fn write_status(feeder: &mut Feeder, text: &str) {
    let line = format!("\r\n\x1b[0;2m{text}\x1b[0m\r\n");
    let _ = feeder.feed(line.as_bytes());
}

/// Ends a session in this terminal before showing `text`: shows any pending synchronized
/// output, drops half-received escape sequences, and switches off the modes a full-screen
/// program may have left on (alternate screen, mouse and focus reporting, bracketed paste,
/// application cursor keys, the keyboard protocol, hidden cursor, scroll region). The last output
/// stays visible and a reconnect starts from a clean terminal.
pub fn end_session(feeder: &mut Feeder, text: &str) {
    {
        let mut term = feeder.shared.term.lock();
        feeder.parser.stop_sync(&mut *term);
    }
    feeder.parser = Processor::new();
    feeder.osc.reset();
    // Back on the main screen first: the message goes under its last output.
    let _ = feeder.feed(b"\x1b[?1049l");
    let (line, column) = {
        let term = feeder.shared.term.lock();
        let at = term.grid().cursor.point;
        (at.line.0.max(0) + 1, at.column.0 + 1)
    };
    const RESET: &str = "\x1b[r\x1b[?6l\x1b[4l\x1b(B\x1b[?1l\x1b>\x1b[?25h\x1b[?7h\
        \x1b[?1000l\x1b[?1002l\x1b[?1003l\x1b[?1005l\x1b[?1006l\x1b[?1004l\x1b[?2004l\x1b[0m";
    let _ = feeder.feed(RESET.as_bytes());
    // No keyboard mode is left on either screen's stack: the next program gets legacy keys.
    clear_keyboard_modes(&feeder.shared, &mut feeder.shared.term.lock());
    feeder.pushes = KeyboardPushes::default();
    // Resetting the scroll region and origin mode moved the cursor to the top: put it back.
    let _ = feeder.feed(format!("\x1b[{line};{column}H").as_bytes());
    write_status(feeder, text);
    let _ = feeder.drain_events();
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scheme::SCHEMES;
    use alacritty_terminal::index::{Column, Line};

    fn make() -> Feeder {
        let (shared, rx) = new_terminal(
            Config::default(),
            GridSize { cols: 40, rows: 10 },
            (&SCHEMES[0]).into(),
        );
        Feeder::new(shared, rx)
    }

    fn row_text(f: &Feeder, line: i32) -> String {
        let term = f.shared.term.lock();
        let row = &term.grid()[Line(line)];
        (0..term.columns())
            .map(|c| row[Column(c)].c)
            .collect::<String>()
            .trim_end()
            .to_string()
    }

    #[test]
    fn feeds_text_and_answers_queries() {
        let mut f = make();
        let Fed { reply, repaint, .. } =
            f.feed(b"\x1b]0;my title\x07hello\x1b[6n\x1b[c\x1b]11;?\x07");
        assert!(repaint);
        assert_eq!(row_text(&f, 0), "hello");
        let reply = String::from_utf8(reply).unwrap();
        assert!(reply.contains("\x1b[1;6R"), "{reply:?}");
        assert!(reply.contains("\x1b[?6c"));
        assert!(reply.contains("\x1b]11;rgb:2323/2323/2323"), "{reply:?}");
        assert_eq!(f.shared.notes.lock().title.as_deref(), Some("my title"));
    }

    #[test]
    fn resize_and_sync_updates() {
        let mut f = make();
        assert!(resize(&f.shared, GridSize { cols: 20, rows: 5 }, 8.0, 16.0));
        assert!(!resize(
            &f.shared,
            GridSize { cols: 20, rows: 5 },
            8.0,
            16.0
        ));
        let reply = f.feed(b"\x1b[14t").reply;
        assert_eq!(reply, b"\x1b[4;80;160t");
        f.feed(b"\x1b[?2026h");
        let repaint = f.feed(b"synced").repaint;
        assert!(!repaint, "buffered by a synchronized update");
        assert!(f.sync_deadline().is_some());
        f.flush_sync();
        assert_eq!(row_text(&f, 0), "synced");
    }

    #[test]
    fn end_session_resets_modes() {
        use alacritty_terminal::term::TermMode;
        let mut f = make();
        // A full-screen app with mouse reporting, cut off in the middle of an OSC sequence.
        f.feed(
            b"\x1b[?1049h\x1b[?1000h\x1b[?1006h\x1b[?1004h\x1b[?2004h\x1b[?25l\x1b[?1h\x1b]0;half",
        );
        end_session(&mut f, "closed");
        let mode = *f.shared.term.lock().mode();
        assert!(!mode.contains(TermMode::ALT_SCREEN));
        assert!(!mode.intersects(TermMode::MOUSE_MODE | TermMode::FOCUS_IN_OUT));
        assert!(!mode.contains(TermMode::BRACKETED_PASTE | TermMode::APP_CURSOR));
        assert!(mode.contains(TermMode::SHOW_CURSOR));
        // The status line is not swallowed by the unfinished OSC.
        let term = f.shared.term.lock();
        let text: String = (0..term.screen_lines() as i32)
            .map(|l| {
                let row = &term.grid()[Line(l)];
                (0..term.columns())
                    .map(|c| row[Column(c)].c)
                    .collect::<String>()
            })
            .collect();
        assert!(text.contains("closed"), "{text:?}");
    }

    /// What Shift+Enter sends to the program in this terminal now.
    fn shift_enter(f: &Feeder) -> Vec<u8> {
        use crate::input::{KeyEvent, Mods, encode_event};
        let modes = KeyModes::of(*f.shared.term.lock().mode());
        let shift = Mods {
            shift: true,
            ..Default::default()
        };
        encode_event(&KeyEvent::press(egui::Key::Enter, shift), None, modes).unwrap()
    }

    #[test]
    fn the_keyboard_protocol_follows_the_program() {
        let mut f = make();
        assert_eq!(shift_enter(&f), b"\r");
        f.feed(b"\x1b[>1u");
        assert_eq!(shift_enter(&f), b"\x1b[13;2u");
        assert_eq!(f.feed(b"\x1b[?u").reply, b"\x1b[?1u");
        // The alternate screen has a stack of its own.
        f.feed(b"\x1b[?1049h");
        assert_eq!(shift_enter(&f), b"\r");
        f.feed(b"\x1b[>11u");
        assert_eq!(f.feed(b"\x1b[?u").reply, b"\x1b[?11u");
        f.feed(b"\x1b[?1049l");
        assert_eq!(f.feed(b"\x1b[?u").reply, b"\x1b[?1u");
        // Popped: legacy keys again.
        f.feed(b"\x1b[<u");
        assert_eq!(shift_enter(&f), b"\r");
        assert_eq!(f.feed(b"\x1b[?u").reply, b"\x1b[?0u");
        // A new scrollback limit keeps the protocol on, and its stack.
        f.feed(b"\x1b[>1u");
        limit_scrollback(&f.shared, 50, CursorPref::Block, false);
        assert_eq!(shift_enter(&f), b"\x1b[13;2u");
    }

    #[test]
    fn end_session_leaves_no_keyboard_mode() {
        let mut f = make();
        // A program pushed on both screens and was killed before popping.
        f.feed(b"\x1b[>1u\x1b[?1049h\x1b[>11u");
        end_session(&mut f, "closed");
        assert_eq!(shift_enter(&f), b"\r");
        assert_eq!(f.feed(b"\x1b[?u").reply, b"\x1b[?0u");
        f.feed(b"\x1b[?1049h");
        assert_eq!(f.feed(b"\x1b[?u").reply, b"\x1b[?0u");
    }

    /// Endless keyboard mode pushes (alacritty panics past 4096) leave the terminal working with
    /// the flags in effect: in pieces, with bytes `vte` skips inside them, in a synchronized
    /// update.
    #[test]
    fn endless_keyboard_mode_pushes_are_survived() {
        let mut f = make();
        f.feed(&b"\x1b[>1u".repeat(10_000));
        assert_eq!(shift_enter(&f), b"\x1b[13;2u");
        let mut data = b"\x1b[?2026h".to_vec();
        data.extend(b"\x1b[>\x0711u".repeat(10_000));
        data.extend_from_slice(b"\x1b[?2026l");
        for piece in data.chunks(7) {
            f.feed(piece);
        }
        assert_eq!(f.feed(b"\x1b[?u").reply, b"\x1b[?11u");
        f.feed(b"\r\nstill here");
        let line = f.shared.term.lock().grid().cursor.point.line.0;
        assert_eq!(row_text(&f, line), "still here");
    }

    /// Output larger than one chunk is parsed completely, and the OSC signals inside it (even
    /// one straddling a chunk boundary) are reported along with the bell.
    #[test]
    fn large_output_is_fed_in_chunks_with_signals() {
        let mut f = make();
        let mut data = Vec::new();
        data.extend(std::iter::repeat_n(b'x', FEED_CHUNK - 3));
        data.extend_from_slice(b"\x1b]9;agent waits\x07\x07\r\n");
        data.extend(std::iter::repeat_n(b'y', FEED_CHUNK * 2));
        data.extend_from_slice(b"\r\nend");
        let fed = f.feed(&data);
        assert!(fed.repaint);
        assert!(fed.bell);
        assert_eq!(
            fed.signals,
            vec![OscSignal::Notify {
                title: None,
                body: "agent waits".into()
            }]
        );
        let term = f.shared.term.lock();
        let cursor_line = term.grid().cursor.point.line.0;
        drop(term);
        assert_eq!(row_text(&f, cursor_line), "end");
    }

    /// The message goes under the session's last output, not over the top of the screen.
    #[test]
    fn end_session_writes_below_the_output() {
        let mut f = make();
        f.feed(b"one\r\ntwo\r\nuser@host % ");
        end_session(&mut f, "[closed]");
        assert_eq!(
            (0..4).map(|l| row_text(&f, l)).collect::<Vec<_>>(),
            ["one", "two", "user@host %", "[closed]"]
        );
    }

    fn screen(f: &Feeder) -> Vec<String> {
        let rows = f.shared.term.lock().screen_lines() as i32;
        (0..rows).map(|l| row_text(f, l)).filter(|t| !t.is_empty()).collect()
    }

    fn cursor(f: &Feeder) -> (i32, usize) {
        let p = f.shared.term.lock().grid().cursor.point;
        (p.line.0, p.column.0)
    }

    fn size(cols: usize) -> GridSize {
        GridSize { cols, rows: 10 }
    }

    /// zsh as its integration draws it: output, then a prompt marked A (redraw=1) … B.
    const ZSH: &[u8] = b"line one\r\nline two\r\n\x1b]133;A;redraw=1\x07user@host project % \x1b]133;B\x07ls";

    #[test]
    fn a_resize_clears_the_prompt_the_shell_redraws() {
        let mut f = make();
        f.feed(ZSH);
        assert!(f.shared.prompt.lock().at_prompt);
        resize(&f.shared, size(30), 8.0, 16.0);
        // The output stays; the prompt row is empty with the cursor at its start.
        assert_eq!(screen(&f), ["line one", "line two"]);
        assert_eq!(cursor(&f), (2, 0));
        // zsh's redraw after SIGWINCH lands there: one prompt, nothing left of the old one.
        settle_prompt(&f.shared, false);
        f.feed(b"\r\x1b[J\x1b]133;A;redraw=1\x07user@host project % \x1b]133;B\x07ls");
        assert_eq!(screen(&f), ["line one", "line two", "user@host project % ls"]);
        // Wider again: the same.
        resize(&f.shared, size(60), 8.0, 16.0);
        assert_eq!(screen(&f), ["line one", "line two"]);
        assert_eq!(cursor(&f), (2, 0));
    }

    #[test]
    fn a_prompt_cleared_by_a_resize_that_came_back_is_put_back() {
        let mut f = make();
        f.feed(ZSH);
        resize(&f.shared, size(20), 8.0, 16.0);
        resize(&f.shared, size(40), 8.0, 16.0);
        // The PTY ends at its old size: no SIGWINCH, so no redraw; the prompt comes back.
        settle_prompt(&f.shared, true);
        assert_eq!(screen(&f), ["line one", "line two", "user@host project % ls"]);
        assert_eq!(cursor(&f), (2, 22));
    }

    #[test]
    fn a_multi_line_prompt_keeps_the_cursor_row_within_it() {
        let mut f = make();
        // Two prompt lines; the cursor on the second, as zsh expects to move up one row.
        f.feed(b"out\r\n\x1b]133;A;redraw=1\x07~/project main\r\n> \x1b]133;B\x07");
        resize(&f.shared, size(25), 8.0, 16.0);
        assert_eq!(screen(&f), ["out"]);
        assert_eq!(cursor(&f), (2, 0));
        // bash redraws only the last line (redraw=last): the first one stays.
        let mut f = make();
        f.feed(b"out\r\n\x1b]133;A;redraw=last\x07~/project main\r\n$ \x1b]133;B\x07");
        resize(&f.shared, size(25), 8.0, 16.0);
        assert_eq!(screen(&f), ["out", "~/project main"]);
        assert_eq!(cursor(&f), (2, 0));
    }

    /// A terminal of 40×10 keeping 20 history lines, filled with output: from here on every line
    /// scrolled in drops the oldest.
    fn full() -> Feeder {
        let (shared, rx) = new_terminal(term_config(20, CursorPref::Block, false), size(40), (&SCHEMES[0]).into());
        let mut f = Feeder::new(shared, rx);
        for i in 0..100 {
            f.feed(format!("out {i}\r\n").as_bytes());
        }
        assert_eq!(f.shared.term.lock().grid().history_size(), 20);
        f
    }

    #[test]
    fn a_full_scrollback_keeps_the_prompt_anchored() {
        // A two-line prompt drawn on the last row scrolls the screen as it is drawn.
        let mut f = full();
        f.feed(b"\x1b]133;A;redraw=1\x07~/project main\r\n> \x1b]133;B\x07ls");
        assert_eq!(f.shared.term.lock().grid().history_size(), 20, "back to its limit");
        resize(&f.shared, size(30), 8.0, 16.0);
        let rows = screen(&f);
        assert!(!rows.iter().any(|r| r.contains("project") || r.contains('>')), "{rows:?}");
        assert_eq!(rows.last().map(String::as_str), Some("out 99"));
        // On the prompt's second row, as zsh expects to move up one.
        assert_eq!(cursor(&f), (9, 0));
        // Typed input wrapping on the last row, then a shorter screen: the same.
        let mut f = full();
        f.feed(b"\x1b]133;A;redraw=1\x07user@host % \x1b]133;B\x07");
        f.feed(&[b'x'; 50]);
        resize(&f.shared, GridSize { cols: 40, rows: 6 }, 8.0, 16.0);
        resize(&f.shared, GridSize { cols: 30, rows: 6 }, 8.0, 16.0);
        let rows = screen(&f);
        assert!(!rows.iter().any(|r| r.contains("user@host") || r.contains('x')), "{rows:?}");
        assert_eq!(rows.last().map(String::as_str), Some("out 99"));
        assert_eq!(cursor(&f), (5, 0));
        // Lines held by a synchronized update that timed out: counted when they are shown.
        let mut f = full();
        f.feed(b"\x1b]133;A;redraw=1\x07~/project main\r\n> \x1b]133;B\x07\x1b[?2026h\r\n\r\n");
        f.flush_sync();
        resize(&f.shared, size(30), 8.0, 16.0);
        assert!(!screen(&f).iter().any(|r| r.contains("project")), "{:?}", screen(&f));
    }

    #[test]
    fn a_prompt_scrolled_away_is_not_cleared() {
        // A whole screen of output with the shell still at its prompt (a background job): the
        // prompt went into the history, and nothing on the screen is taken for it.
        let mut f = full();
        f.feed(b"\x1b]133;A;redraw=1\x07user@host % \x1b]133;B\x07");
        for i in 0..12 {
            f.feed(format!("\r\njob {i}").as_bytes());
        }
        resize(&f.shared, size(30), 8.0, 16.0);
        let rows = screen(&f);
        assert_eq!(rows.len(), 10, "{rows:?}");
        assert_eq!(rows.last().map(String::as_str), Some("job 11"));
        // A full-screen program started at the prompt (a picker on a key binding): the history
        // is back at its limit when it leaves, and the prompt is no longer known.
        let mut f = full();
        f.feed(b"\x1b]133;A;redraw=1\x07user@host % \x1b]133;B\x07\r\n\r\n\x1b[?1049hpicker");
        f.feed(b"\x1b[?1049l");
        assert_eq!(f.shared.term.lock().grid().history_size(), 20);
        resize(&f.shared, size(30), 8.0, 16.0);
        assert!(screen(&f).iter().any(|r| r.contains("user@host")));
        // Clearing the scrollback drops the anchors with the lines.
        let mut f = full();
        f.feed(b"\x1b]133;A;redraw=1\x07user@host % \x1b]133;B\x07");
        crate::widget::clear_scrollback(&f.shared);
        resize(&f.shared, size(30), 8.0, 16.0);
        assert!(!screen(&f).iter().any(|r| r.contains("user@host")));
        assert_eq!(cursor(&f), (9, 0));
    }

    #[test]
    fn the_scrollback_limit_goes_both_ways() {
        let mut f = full();
        limit_scrollback(&f.shared, 5, CursorPref::Block, false);
        assert_eq!(f.shared.term.lock().grid().history_size(), 5);
        limit_scrollback(&f.shared, 50, CursorPref::Block, false);
        for i in 0..100 {
            f.feed(format!("more {i}\r\n").as_bytes());
        }
        assert_eq!(f.shared.term.lock().grid().history_size(), 50);
    }

    #[test]
    fn nothing_is_cleared_while_a_command_runs_or_without_redraw() {
        // A command started (C): its output is left to reflow.
        let mut f = make();
        f.feed(ZSH);
        f.feed(b"\r\n\x1b]133;C\x07working...");
        resize(&f.shared, size(30), 8.0, 16.0);
        assert!(screen(&f).iter().any(|l| l.contains("working...")));
        assert!(screen(&f).iter().any(|l| l.contains("user@host")));
        // A shell that does not say how it redraws (no redraw= option).
        let mut f = make();
        f.feed(b"\x1b]133;A\x07user@host % \x1b]133;B\x07");
        resize(&f.shared, size(30), 8.0, 16.0);
        assert_eq!(screen(&f), ["user@host %"]);
        // Only the height changed: no reflow, nothing to clear.
        let mut f = make();
        f.feed(ZSH);
        resize(&f.shared, GridSize { cols: 40, rows: 12 }, 8.0, 16.0);
        assert!(screen(&f).iter().any(|l| l.contains("user@host project % ls")));
    }
}
