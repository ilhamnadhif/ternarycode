//! A terminal's history, screen and modes as the bytes that draw them again. When the app
//! restarts in place for an update (the programs keep running, see `pty::LocalPty::adopt`), the
//! new app starts with an empty terminal and is fed these: the same lines, colours, cursor and
//! modes (bracketed paste, mouse reporting, the kitty keyboard flags…) as before.

use std::fmt::Write as _;

use alacritty_terminal::grid::{Dimensions, Grid};
use alacritty_terminal::index::{Column, Line};
use alacritty_terminal::term::TermMode;
use alacritty_terminal::term::cell::{Cell, Flags};
use alacritty_terminal::vte::ansi::{Color, NamedColor};

use crate::core::{Fed, Feeder, TermShared};
use crate::osc::Redraw;

/// What a terminal holds besides its cells: the title its program gave it, and where the
/// shell's prompt is (from its shell integration marks), which the bytes do not carry.
#[derive(Clone, Debug, Default, serde::Serialize, serde::Deserialize)]
pub struct Extras {
    pub title: Option<String>,
    at_prompt: bool,
    redraw: u8,
    start: Option<usize>,
    input: Option<usize>,
    #[serde(default)]
    input_col: usize,
}

pub fn extras(shared: &TermShared) -> Extras {
    let prompt = shared.prompt.lock();
    Extras {
        title: shared.notes.lock().title.clone(),
        at_prompt: prompt.at_prompt,
        redraw: match prompt.redraw {
            Redraw::Unknown => 0,
            Redraw::All => 1,
            Redraw::LastLine => 2,
        },
        start: prompt.start,
        input: prompt.input,
        input_col: prompt.input_col,
    }
}

/// Draws a [`snapshot`] in a new terminal of the size it was taken at, before anything else
/// is read into it, and puts its [`Extras`] back.
pub fn restore(feeder: &mut Feeder, bytes: &[u8], extras: Extras) -> Fed {
    let fed = feeder.feed(bytes);
    let shared = feeder.shared.clone();
    shared.notes.lock().title = extras.title;
    let mut prompt = shared.prompt.lock();
    prompt.at_prompt = extras.at_prompt;
    prompt.redraw = match extras.redraw {
        1 => Redraw::All,
        2 => Redraw::LastLine,
        _ => Redraw::Unknown,
    };
    prompt.start = extras.start;
    prompt.input = extras.input;
    prompt.input_col = extras.input_col;
    fed
}

#[derive(Clone, Copy, PartialEq)]
struct Style {
    fg: Color,
    bg: Color,
    flags: Flags,
    underline: Option<Color>,
}

const DEFAULT: Style = Style {
    fg: Color::Named(NamedColor::Foreground),
    bg: Color::Named(NamedColor::Background),
    flags: Flags::empty(),
    underline: None,
};

/// What is kept of a cell's flags: how it is drawn (not where it sits, or how it wraps).
const DRAWN: Flags = Flags::BOLD
    .union(Flags::DIM)
    .union(Flags::ITALIC)
    .union(Flags::ALL_UNDERLINES)
    .union(Flags::INVERSE)
    .union(Flags::HIDDEN)
    .union(Flags::STRIKEOUT);

fn color(out: &mut String, c: Color, base: u8, bright: u8, ext: u8) {
    match c {
        Color::Named(n) => {
            if ext == 58 {
                let i = n as usize;
                match n {
                    NamedColor::Foreground | NamedColor::BrightForeground | NamedColor::DimForeground | NamedColor::Background => {
                        out.push_str(";59");
                    }
                    _ if i < 16 => {
                        let _ = write!(out, ";58;5;{i}");
                    }
                    _ if (NamedColor::DimBlack as usize..=NamedColor::DimWhite as usize).contains(&i) => {
                        let _ = write!(out, ";58;5;{}", i - NamedColor::DimBlack as usize);
                    }
                    _ => {}
                }
                return;
            }
            let i = n as usize;
            let code = match n {
                NamedColor::Foreground | NamedColor::BrightForeground | NamedColor::DimForeground => 39,
                NamedColor::Background => 49,
                _ if i < 8 => base as usize + i,
                _ if i < 16 => bright as usize + i - 8,
                // The dim colours alacritty uses for dimmed text: their normal colour.
                _ if (NamedColor::DimBlack as usize..=NamedColor::DimWhite as usize).contains(&i) => base as usize + i - NamedColor::DimBlack as usize,
                _ => return,
            };
            // 39 and 49 are the defaults whichever layer: the background's is 49.
            let code = if code == 39 && base == 40 { 49 } else { code };
            let _ = write!(out, ";{code}");
        }
        Color::Indexed(i) => {
            let _ = write!(out, ";{ext};5;{i}");
        }
        Color::Spec(rgb) => {
            let _ = write!(out, ";{ext};2;{};{};{}", rgb.r, rgb.g, rgb.b);
        }
    }
}

fn sgr(out: &mut String, s: &Style) {
    out.push_str("\x1b[0");
    let f = s.flags;
    for (flag, code) in [(Flags::BOLD, ";1"), (Flags::DIM, ";2"), (Flags::ITALIC, ";3"), (Flags::INVERSE, ";7"), (Flags::HIDDEN, ";8"), (Flags::STRIKEOUT, ";9")] {
        if f.contains(flag) {
            out.push_str(code);
        }
    }
    if f.contains(Flags::DOUBLE_UNDERLINE) {
        out.push_str(";4:2");
    } else if f.contains(Flags::UNDERCURL) {
        out.push_str(";4:3");
    } else if f.contains(Flags::DOTTED_UNDERLINE) {
        out.push_str(";4:4");
    } else if f.contains(Flags::DASHED_UNDERLINE) {
        out.push_str(";4:5");
    } else if f.contains(Flags::UNDERLINE) {
        out.push_str(";4");
    }
    if s.fg != DEFAULT.fg {
        color(out, s.fg, 30, 90, 38);
    }
    if s.bg != DEFAULT.bg {
        color(out, s.bg, 40, 100, 48);
    }
    if let Some(u) = s.underline {
        color(out, u, 30, 90, 58);
    }
    out.push('m');
}

/// The lines of `grid` (its history too when `history`), each as it was drawn. A line that
/// wrapped onto the next is written to its last column, so it wraps the same way again.
fn dump(grid: &Grid<Cell>, history: bool, out: &mut String) {
    let cols = grid.columns();
    let top = if history { grid.topmost_line() } else { Line(0) };
    let bottom = grid.bottommost_line();
    let mut cur = DEFAULT;
    let mut line = top;
    while line <= bottom {
        let row = &grid[line];
        let wrapped = row[Column(cols - 1)].flags.contains(Flags::WRAPLINE);
        // Where the line's text ends (blanks after it are not written, unless it wrapped).
        let end = if wrapped {
            cols
        } else {
            (0..cols)
                .rev()
                .find(|&c| {
                    let cell = &row[Column(c)];
                    cell.c != ' ' || cell.bg != DEFAULT.bg || cell.flags.intersects(DRAWN) || cell.zerowidth().is_some()
                })
                .map_or(0, |c| c + 1)
        };
        for c in 0..end {
            let cell = &row[Column(c)];
            if cell.flags.intersects(Flags::WIDE_CHAR_SPACER | Flags::LEADING_WIDE_CHAR_SPACER) {
                continue;
            }
            let style = Style {
                fg: cell.fg,
                bg: cell.bg,
                flags: cell.flags & DRAWN,
                underline: cell.underline_color(),
            };
            if style != cur {
                sgr(out, &style);
                cur = style;
            }
            out.push(if cell.c == '\0' { ' ' } else { cell.c });
            if let Some(extra) = cell.zerowidth() {
                out.extend(extra.iter());
            }
        }
        if line < bottom && !wrapped {
            // The default colours before a new line: the line scrolled in takes the current
            // background.
            if cur != DEFAULT {
                out.push_str("\x1b[0m");
                cur = DEFAULT;
            }
            out.push_str("\r\n");
        }
        line += 1;
    }
    if cur != DEFAULT {
        out.push_str("\x1b[0m");
    }
}

fn cursor_to(out: &mut String, grid: &Grid<Cell>) {
    let p = grid.cursor.point;
    let _ = write!(out, "\x1b[{};{}H", p.line.0.max(0) + 1, p.column.0 + 1);
}

/// The modes that differ from a new terminal's, as the sequences that set them.
fn modes(out: &mut String, mode: TermMode) {
    let default = TermMode::default();
    let private = [
        (TermMode::SHOW_CURSOR, 25),
        (TermMode::APP_CURSOR, 1),
        (TermMode::ORIGIN, 6),
        (TermMode::LINE_WRAP, 7),
        (TermMode::MOUSE_REPORT_CLICK, 1000),
        (TermMode::MOUSE_DRAG, 1002),
        (TermMode::MOUSE_MOTION, 1003),
        (TermMode::FOCUS_IN_OUT, 1004),
        (TermMode::UTF8_MOUSE, 1005),
        (TermMode::SGR_MOUSE, 1006),
        (TermMode::ALTERNATE_SCROLL, 1007),
        (TermMode::URGENCY_HINTS, 1042),
        (TermMode::BRACKETED_PASTE, 2004),
    ];
    for (flag, n) in private {
        if mode.contains(flag) != default.contains(flag) {
            let _ = write!(out, "\x1b[?{n}{}", if mode.contains(flag) { 'h' } else { 'l' });
        }
    }
    for (flag, n) in [(TermMode::INSERT, 4), (TermMode::LINE_FEED_NEW_LINE, 20)] {
        if mode.contains(flag) != default.contains(flag) {
            let _ = write!(out, "\x1b[{n}{}", if mode.contains(flag) { 'h' } else { 'l' });
        }
    }
    if mode.contains(TermMode::APP_KEYPAD) {
        out.push_str("\x1b=");
    }
    // The kitty keyboard flags the program pushed (one push of all of them).
    let kitty = [
        (TermMode::DISAMBIGUATE_ESC_CODES, 1),
        (TermMode::REPORT_EVENT_TYPES, 2),
        (TermMode::REPORT_ALTERNATE_KEYS, 4),
        (TermMode::REPORT_ALL_KEYS_AS_ESC, 8),
        (TermMode::REPORT_ASSOCIATED_TEXT, 16),
    ]
    .iter()
    .filter(|(flag, _)| mode.contains(*flag))
    .map(|(_, bit)| bit)
    .sum::<u32>();
    if kitty > 0 {
        let _ = write!(out, "\x1b[>{kitty}u");
    }
}

/// The bytes that draw this terminal again, in a new terminal of the same size: the history and
/// screen, the alternate screen when a full-screen program shows it, the cursor and the modes.
/// Taken as the old app leaves: it switches the terminal back to its main screen to read it.
pub fn snapshot(feeder: &mut Feeder) -> Vec<u8> {
    // A synchronized update still held back is drawn first.
    if feeder.sync_deadline().is_some() {
        feeder.flush_sync();
    }
    let mut out = String::new();
    let shared = feeder.shared.clone();
    // Where the cursor is on the screen that shows.
    let mut cursor = String::new();
    let (alt, mode) = {
        let term = shared.term.lock();
        let mode = *term.mode();
        let alt = mode.contains(TermMode::ALT_SCREEN).then(|| {
            let mut screen = String::new();
            dump(term.grid(), false, &mut screen);
            cursor_to(&mut screen, term.grid());
            cursor_to(&mut cursor, term.grid());
            screen
        });
        (alt, mode)
    };
    if alt.is_some() {
        // To the main screen (the cursor saved on the way in comes back with it).
        feeder.feed(b"\x1b[?1049l");
    }
    {
        let term = shared.term.lock();
        dump(term.grid(), true, &mut out);
        cursor_to(&mut out, term.grid());
        if alt.is_none() {
            cursor_to(&mut cursor, term.grid());
        }
    }
    if let Some(screen) = alt {
        out.push_str("\x1b[?1049h\x1b[H");
        out.push_str(&screen);
    }
    modes(&mut out, mode);
    // Origin mode, set just now, put the cursor at the top: back where it was (no scroll
    // region is drawn again, so its positions are the screen's).
    if mode.contains(TermMode::ORIGIN) {
        out.push_str(&cursor);
    }
    out.into_bytes()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::{GridSize, new_terminal, term_config};
    use crate::scheme::SCHEMES;

    fn terminal(cols: usize, rows: usize) -> Feeder {
        let (shared, rx) = new_terminal(term_config(1000, Default::default(), false), GridSize { cols, rows }, (&SCHEMES[0]).into());
        Feeder::new(shared, rx)
    }

    fn text(f: &Feeder) -> Vec<String> {
        let term = f.shared.term.lock();
        let grid = term.grid();
        let mut lines = Vec::new();
        let mut line = grid.topmost_line();
        while line <= grid.bottommost_line() {
            let row = &grid[line];
            lines.push((0..grid.columns()).map(|c| row[Column(c)].c).collect::<String>().trim_end().to_string());
            line += 1;
        }
        lines
    }

    /// A terminal fed its snapshot shows the same lines (history included), colours, cursor and
    /// modes as the one it was taken from.
    #[test]
    fn a_snapshot_draws_the_same_terminal_again() {
        let mut a = terminal(20, 5);
        let mut src = String::new();
        for i in 0..12 {
            let _ = write!(src, "line {i} \x1b[1;31mred\x1b[0m \x1b[38;2;1;2;3mrgb\x1b[0m\r\n");
        }
        // A line longer than the width (it wraps), a wide character, then the prompt.
        src.push_str("abcdefghijklmnopqrstuvwxyz 漢字\r\n% ");
        a.feed(src.as_bytes());
        a.feed(b"\x1b[?2004h\x1b[?1h\x1b[>1u");
        let bytes = snapshot(&mut a);
        let mut b = terminal(20, 5);
        b.feed(&bytes);
        assert_eq!(text(&a), text(&b));
        let (ta, tb) = (a.shared.term.lock(), b.shared.term.lock());
        assert_eq!(ta.grid().cursor.point, tb.grid().cursor.point);
        assert_eq!(*ta.mode(), *tb.mode());
        let red = |t: &alacritty_terminal::Term<crate::core::Listener>| t.grid()[Line(0)][Column(7)].fg;
        assert_eq!(red(&ta), red(&tb));
    }

    /// A full-screen program's screen comes back over the main one, which keeps its history.
    #[test]
    fn the_alternate_screen_comes_back_over_the_main_one() {
        let mut a = terminal(20, 4);
        a.feed(b"one\r\ntwo\r\nthree\r\nfour\r\nfive\r\n% vim");
        a.feed(b"\x1b[?1049h\x1b[H~ editing\r\n~");
        let main_before = {
            let mut probe = terminal(20, 4);
            probe.feed(b"one\r\ntwo\r\nthree\r\nfour\r\nfive\r\n% vim");
            text(&probe)
        };
        let bytes = snapshot(&mut a);
        let mut b = terminal(20, 4);
        b.feed(&bytes);
        assert!(b.shared.term.lock().mode().contains(TermMode::ALT_SCREEN));
        assert_eq!(text(&b)[0], "~ editing");
        b.feed(b"\x1b[?1049l");
        assert_eq!(text(&b), main_before);
    }
}
