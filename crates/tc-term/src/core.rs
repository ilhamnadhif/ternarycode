//! One terminal: an alacritty `Term` shared between the SSH reader task (which feeds it bytes)
//! and the UI (which draws it and sends input).
//!
//! Events raised by the parser are queued through a channel and handled by the reader right
//! after `advance` (outside the lock): replies such as device attributes, cursor position and
//! colour queries go straight back to the remote even when the window is not repainting.

use std::sync::Arc;
use std::sync::mpsc;
use std::time::Instant;

use alacritty_terminal::event::{Event, EventListener, WindowSize};
use alacritty_terminal::grid::{Dimensions, Row};
use alacritty_terminal::index::{Column, Line, Point};
use alacritty_terminal::sync::FairMutex;
use alacritty_terminal::term::cell::Cell;
use alacritty_terminal::term::{Config, Term, TermMode};
use alacritty_terminal::vte::ansi::{ClearMode, CursorShape, CursorStyle, Handler, Processor, Rgb};
use egui::Color32;
use parking_lot::Mutex;

use super::scheme::Scheme;
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

/// State the UI reads that the parser produces (title, bell, OSC 52 copies).
#[derive(Default)]
pub struct UiNotes {
    pub title: Option<String>,
    pub bell_at: Option<Instant>,
    pub clipboard: Option<String>,
    /// Set whenever new output arrived; the UI uses it to mark inactive tabs.
    pub activity: bool,
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
    /// Cell size in pixels and grid size, for CSI 14 t and the SSH window-change request.
    pub window: Mutex<WindowSize>,
    pub palette: Mutex<QueryPalette>,
}

pub type SharedTerm = Arc<TermShared>;

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
    let term = Term::new(config, &size, Listener(tx));
    let shared = Arc::new(TermShared {
        term: FairMutex::new(term),
        notes: Mutex::new(UiNotes::default()),
        prompt: Mutex::new(PromptState::default()),
        window: Mutex::new(WindowSize {
            num_lines: size.rows as u16,
            num_cols: size.cols as u16,
            cell_width: 8,
            cell_height: 16,
        }),
        palette: Mutex::new(palette),
    });
    (shared, rx)
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
    /// OSC signals `vte` does not report (notifications, shell integration, hooks).
    pub signals: Vec<OscSignal>,
    /// The program rang the bell.
    pub bell: bool,
}

/// Feeds program output into a terminal and answers the parser's queries.
pub struct Feeder {
    pub shared: SharedTerm,
    parser: Processor,
    osc: OscScanner,
    events: mpsc::Receiver<Event>,
    bell: bool,
}

impl Feeder {
    pub fn new(shared: SharedTerm, events: mpsc::Receiver<Event>) -> Self {
        Self {
            shared,
            parser: Processor::new(),
            osc: OscScanner::new(),
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
        if repaint {
            self.shared.notes.lock().activity = true;
        }
        let reply = self.drain_events();
        Fed {
            reply,
            repaint,
            signals,
            bell: std::mem::take(&mut self.bell),
        }
    }

    /// Parses `bytes`; returns whether the screen changed.
    fn advance(&mut self, bytes: &[u8]) -> bool {
        let mut repaint = false;
        for chunk in bytes.chunks(FEED_CHUNK) {
            let mut term = self.shared.term.lock();
            self.parser.advance(&mut *term, chunk);
            if !term.mode().contains(TermMode::ALT_SCREEN) {
                let mut notes = self.shared.notes.lock();
                if std::mem::take(&mut notes.clear_history_pending) {
                    term.grid_mut().clear_history();
                }
            }
            drop(term);
            repaint |= self.parser.sync_bytes_count() < chunk.len();
        }
        repaint
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
            self.parser.stop_sync(&mut *term);
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
                Event::Bell => {
                    self.bell = true;
                    self.shared.notes.lock().bell_at = Some(Instant::now());
                }
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

/// Lowers the number of scrollback lines kept (the oldest go first).
pub fn limit_scrollback(shared: &TermShared, lines: usize, cursor: CursorPref, blink: bool) {
    let mut term = shared.term.lock();
    if term.grid().history_size() > lines {
        term.set_options(term_config(lines, cursor, blink));
    }
}

/// Most rows a prompt may span for the resize to clear it (more means the recorded position is
/// stale).
const PROMPT_ROWS: usize = 12;

/// Resizes the grid (clamped to the minimum) and records the pixel size. Returns whether the
/// grid size changed, i.e. the remote must be told. When the width changes while the shell sits
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
    let offset = if term.columns() != size.cols {
        clear_prompt(&mut term, &mut prompt)
    } else {
        None
    };
    term.resize(size);
    if let Some(offset) = offset {
        // The cleared rows moved with the reflow: the anchor is `offset` rows above the cursor.
        let at = term.grid().history_size()
            + (term.grid().cursor.point.line.0.max(0) as usize).saturating_sub(offset);
        match prompt.redraw {
            Redraw::LastLine => prompt.input = Some(at),
            _ => prompt.start = Some(at),
        }
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
/// application cursor keys, hidden cursor, scroll region). The last output stays visible and a
/// reconnect starts from a clean terminal.
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
