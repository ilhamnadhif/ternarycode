//! The terminal widget: draws an alacritty grid with egui's painter and turns keyboard, mouse,
//! clipboard and IME events into bytes for the program on the PTY.

use std::time::Duration;

use alacritty_terminal::grid::{Dimensions, Scroll};
use alacritty_terminal::index::{Column, Line, Point, Side};
use alacritty_terminal::selection::{Selection, SelectionType};
use alacritty_terminal::term::cell::Flags;
use alacritty_terminal::term::color::Colors;
use alacritty_terminal::term::{self, TermMode};
use alacritty_terminal::vte::ansi::{Color, CursorShape, NamedColor};
use egui::emath::GuiRounding as _;
use egui::{
    self, Color32, CornerRadius, Event, EventFilter, FontFamily, FontId, Id, ImeEvent, Key,
    MouseWheelUnit, PointerButton, Pos2, Rect, Sense, Stroke, StrokeKind, Ui, Vec2, pos2, vec2,
};

use crate::core::{self as tcore, GridSize, TermShared};
use crate::input::{self, KeyModes, Mods, MouseAction, MouseButton};
use crate::scheme::{Scheme, xterm_256};
use crate::fonts;

pub const TERM_FILTER: EventFilter = EventFilter {
    tab: true,
    horizontal_arrows: true,
    vertical_arrows: true,
    escape: true,
};
pub const PAD: f32 = 10.0;
const DIM_FACTOR: f32 = 0.66;
/// The cursor blinks for this long after the last key or focus change.
const BLINK_SECS: f64 = 15.0;

#[derive(Clone, Copy)]
pub struct TermStyle<'a> {
    pub font_size: f32,
    pub cell: Vec2,
    pub scheme: &'a Scheme,
    pub real_bold: bool,
    pub real_italic: bool,
    pub option_as_meta: bool,
    pub copy_on_select: bool,
    pub cursor_blink: bool,
}

/// Per-pane widget state that survives between frames.
#[derive(Default)]
pub struct TermView {
    pub id: Option<Id>,
    preedit: String,
    scroll_accum: f32,
    reported_button: Option<MouseButton>,
    last_report_cell: Option<(usize, usize)>,
    pub grid: Option<GridSize>,
    /// Cell size in physical pixels last reported to the PTY.
    cell_px: Option<(u32, u32)>,
    blink_epoch: f64,
    was_focused: bool,
    /// The context menu is open (it must keep being shown even in mouse-reporting mode).
    menu_open: bool,
    /// Paste was chosen (`request_paste`): the paste event the next frame brings is a paste,
    /// not a typed Ctrl+V.
    paste_requested: bool,
    /// The link the context menu was opened on.
    menu_link: Option<Link>,
    /// The clipboard's text came since V was last released: the paste shortcut pasted text.
    pasted_since_v: bool,
}

impl TermView {
    /// Records that this pane no longer has focus (its tab was left): the program gets
    /// a focus-out report if it asked for them. Returns the bytes to send.
    pub fn blur(&mut self, shared: &TermShared, alive: bool) -> Vec<u8> {
        self.preedit.clear();
        if !std::mem::take(&mut self.was_focused) {
            return Vec::new();
        }
        let mut term = shared.term.lock();
        term.is_focused = false;
        if alive && term.mode().contains(TermMode::FOCUS_IN_OUT) {
            b"\x1b[O".to_vec()
        } else {
            Vec::new()
        }
    }

    /// Pastes the clipboard into this pane (the context menu's Paste) on every platform: on
    /// Windows and Linux a paste event without Shift is otherwise a typed Ctrl+V. The text
    /// comes with the next frame's events.
    pub fn request_paste(&mut self, ctx: &egui::Context) {
        self.paste_requested = true;
        ctx.send_viewport_cmd(egui::ViewportCommand::RequestPaste);
    }
}

impl TermView {
    /// Seconds since a key was typed into this terminal (or it got focus), at egui time `now`.
    pub fn since_input(&self, now: f64) -> f64 {
        now - self.blink_epoch
    }

    pub fn new(id: Id) -> Self {
        Self {
            id: Some(id),
            ..Default::default()
        }
    }
}

/// The widget id of a pane's terminal (stable, so focus can be moved to it).
pub fn pane_widget_id(pane: u64) -> Id {
    Id::new(("terminal", pane))
}

#[derive(Default)]
pub struct TermResponse {
    /// Bytes for the program (keys, paste, mouse reports, focus events).
    pub input: Vec<u8>,
    /// New grid size and pixel size when the pane was resized.
    pub resized: Option<(GridSize, u32, u32)>,
    pub focused: bool,
    /// The user clicked into the pane (make it the active pane).
    pub activated: bool,
    /// Enter pressed in a pane whose session ended.
    pub reconnect: bool,
    /// ⌘-click (Ctrl-click elsewhere) on a link: what it opens (a web address, an OSC 8
    /// hyperlink's address, or a word that may be a path).
    pub link: Option<String>,
    /// The paste shortcut was pressed with no text on the clipboard (an image, say): the bytes
    /// Ctrl+V sends, for an agent that reads the clipboard itself.
    pub paste_without_text: Option<Vec<u8>>,
}

/// The modifier held to open links: ⌘ or Ctrl on macOS, Ctrl elsewhere.
pub fn link_modifier(m: egui::Modifiers) -> bool {
    m.command || (cfg!(target_os = "macos") && m.ctrl)
}

pub fn cell_size(ctx: &egui::Context, font_size: f32) -> Vec2 {
    let font = FontId::new(font_size, FontFamily::Name(fonts::TERM_REGULAR.into()));
    ctx.fonts_mut(|f| vec2(f.glyph_width(&font, 'M'), f.row_height(&font)))
}

fn rgb(c: alacritty_terminal::vte::ansi::Rgb) -> Color32 {
    Color32::from_rgb(c.r, c.g, c.b)
}

fn mul(c: Color32, f: f32) -> Color32 {
    Color32::from_rgb(
        (c.r() as f32 * f) as u8,
        (c.g() as f32 * f) as u8,
        (c.b() as f32 * f) as u8,
    )
}

/// Palette lookup honouring OSC 4/10/11/12 overrides.
fn palette(idx: usize, colors: &Colors, scheme: &Scheme) -> Color32 {
    if let Some(c) = colors[idx] {
        return rgb(c);
    }
    match idx {
        0..=15 => scheme.ansi[idx],
        16..=255 => xterm_256(idx as u8),
        256 | 267 => scheme.foreground,
        257 => scheme.background,
        258 => scheme.cursor,
        259..=266 => mul(palette(idx - 259, colors, scheme), DIM_FACTOR),
        268 => mul(palette(256, colors, scheme), DIM_FACTOR),
        _ => scheme.foreground,
    }
}

fn resolve_fg(color: Color, flags: Flags, colors: &Colors, scheme: &Scheme) -> Color32 {
    let dim = flags.contains(Flags::DIM);
    match color {
        Color::Spec(c) => {
            if dim {
                mul(rgb(c), DIM_FACTOR)
            } else {
                rgb(c)
            }
        }
        Color::Named(n) => palette(if dim { n.to_dim() } else { n } as usize, colors, scheme),
        Color::Indexed(i) => {
            let i = i as usize;
            if dim && !flags.contains(Flags::BOLD) {
                match i {
                    0..=7 => palette(NamedColor::DimBlack as usize + i, colors, scheme),
                    8..=15 => palette(i - 8, colors, scheme),
                    // The 256-colour cube and greys have no dim entries: darkened like `Spec`.
                    _ => mul(palette(i, colors, scheme), DIM_FACTOR),
                }
            } else {
                palette(i, colors, scheme)
            }
        }
    }
}

/// `None` means the default background (not painted per cell).
fn resolve_bg(color: Color, colors: &Colors, scheme: &Scheme) -> Option<Color32> {
    match color {
        Color::Named(NamedColor::Background) => None,
        Color::Spec(c) => Some(rgb(c)),
        Color::Named(n) => Some(palette(n as usize, colors, scheme)),
        Color::Indexed(i) => Some(palette(i as usize, colors, scheme)),
    }
}

#[derive(Clone, Copy, PartialEq)]
struct RunStyle {
    fg: Color32,
    bold: bool,
    italic: bool,
    underline: u8, // 0 none, 1 single, 2 double, 3 curly/dotted/dashed
    underline_color: Option<Color32>,
    strike: bool,
}

struct TextRun {
    col: usize,
    text: String,
    style: RunStyle,
}

/// Box-drawing weights per direction: 0 none, 1 light, 2 heavy, 3 double. (up, right, down, left)
fn box_lines(c: char) -> Option<[u8; 4]> {
    Some(match c {
        '─' => [0, 1, 0, 1],
        '━' => [0, 2, 0, 2],
        '│' => [1, 0, 1, 0],
        '┃' => [2, 0, 2, 0],
        '┌' => [0, 1, 1, 0],
        '┐' => [0, 0, 1, 1],
        '└' => [1, 1, 0, 0],
        '┘' => [1, 0, 0, 1],
        '├' => [1, 1, 1, 0],
        '┤' => [1, 0, 1, 1],
        '┬' => [0, 1, 1, 1],
        '┴' => [1, 1, 0, 1],
        '┼' => [1, 1, 1, 1],
        '┏' => [0, 2, 2, 0],
        '┓' => [0, 0, 2, 2],
        '┗' => [2, 2, 0, 0],
        '┛' => [2, 0, 0, 2],
        '┣' => [2, 2, 2, 0],
        '┫' => [2, 0, 2, 2],
        '┳' => [0, 2, 2, 2],
        '┻' => [2, 2, 0, 2],
        '╋' => [2, 2, 2, 2],
        '═' => [0, 3, 0, 3],
        '║' => [3, 0, 3, 0],
        '╔' => [0, 3, 3, 0],
        '╗' => [0, 0, 3, 3],
        '╚' => [3, 3, 0, 0],
        '╝' => [3, 0, 0, 3],
        '╠' => [3, 3, 3, 0],
        '╣' => [3, 0, 3, 3],
        '╦' => [0, 3, 3, 3],
        '╩' => [3, 3, 0, 3],
        '╬' => [3, 3, 3, 3],
        '╒' => [0, 3, 1, 0],
        '╓' => [0, 1, 3, 0],
        '╕' => [0, 0, 1, 3],
        '╖' => [0, 0, 3, 1],
        '╘' => [1, 3, 0, 0],
        '╙' => [3, 1, 0, 0],
        '╛' => [1, 0, 0, 3],
        '╜' => [3, 0, 0, 1],
        '╞' => [1, 3, 1, 0],
        '╟' => [3, 1, 3, 0],
        '╡' => [1, 0, 1, 3],
        '╢' => [3, 0, 3, 1],
        '╤' => [0, 3, 1, 3],
        '╥' => [0, 1, 3, 1],
        '╧' => [1, 3, 0, 3],
        '╨' => [3, 1, 0, 1],
        '╪' => [1, 3, 1, 3],
        '╫' => [3, 1, 3, 1],
        '╴' => [0, 0, 0, 1],
        '╵' => [1, 0, 0, 0],
        '╶' => [0, 1, 0, 0],
        '╷' => [0, 0, 1, 0],
        _ => return None,
    })
}

/// Draws box-drawing and block characters as shapes so lines join seamlessly across cells.
/// Returns false for characters it does not handle.
fn draw_special(painter: &egui::Painter, c: char, r: Rect, color: Color32, ppp: f32) -> bool {
    let px = 1.0 / ppp;
    let snap = |v: f32| (v * ppp).round() / ppp;
    if let Some((dx, dy)) = match c {
        '╭' => Some((1.0, 1.0)),
        '╮' => Some((-1.0, 1.0)),
        '╰' => Some((1.0, -1.0)),
        '╯' => Some((-1.0, -1.0)),
        _ => None,
    } {
        // Rounded corner: straight to the arc, a quarter circle, straight to the other edge.
        let (cx, cy) = (snap(r.center().x), snap(r.center().y));
        let rad = (r.width().min(r.height()) / 2.0).floor();
        let center = pos2(cx + dx * rad, cy + dy * rad);
        let mut points = vec![pos2(cx, if dy > 0.0 { r.bottom() } else { r.top() })];
        for i in 0..=8 {
            let a = std::f32::consts::FRAC_PI_2 * i as f32 / 8.0;
            points.push(center + vec2(-dx * rad * a.cos(), -dy * rad * a.sin()));
        }
        points.push(pos2(if dx > 0.0 { r.right() } else { r.left() }, cy));
        painter.add(egui::Shape::line(points, Stroke::new(px.max(1.0), color)));
        return true;
    }
    if let Some(lines) = box_lines(c) {
        let cx = snap(r.center().x);
        let cy = snap(r.center().y);
        let light = px.max(1.0);
        // Distance of a double line's strokes from the center line.
        let gap = light * 1.5;
        let width = |wt: u8| if wt == 2 { light * 2.0 } else { light };
        // How far a perpendicular arm's nearest stroke sits from the center, and half its width.
        let near = |wt: u8| if wt == 3 { gap } else { 0.0 };
        let half = |wt: u8| width(wt) / 2.0;
        for (dir, &wt) in lines.iter().enumerate() {
            if wt == 0 {
                continue;
            }
            let opposite = lines[(dir + 2) % 4];
            // The two perpendicular arms: for a vertical arm right (+) then left (-), for a
            // horizontal arm down (+) then up (-).
            let (p_plus, p_minus) = if dir % 2 == 0 {
                (lines[1], lines[3])
            } else {
                (lines[2], lines[0])
            };
            let w = width(wt);
            // Each stroke: offset across the arm, and `t`, where it ends measured from the
            // center toward the arm's edge (negative = past the center).
            let mut strokes: Vec<(f32, f32)> = Vec::new();
            if wt == 3 {
                for (side, p, q) in [(1.0, p_plus, p_minus), (-1.0, p_minus, p_plus)] {
                    let t = if p > 0 {
                        near(p) - half(p) // stop at the perpendicular line on this side
                    } else if opposite > 0 {
                        -light / 2.0 // straight through
                    } else if q > 0 {
                        -(near(q) + half(q)) // outer stroke of a corner: to the far side
                    } else {
                        -light / 2.0
                    };
                    strokes.push((side * gap, t));
                }
            } else {
                let t = if opposite > 0 {
                    -w / 2.0
                } else if p_plus > 0 && p_minus > 0 {
                    near(p_plus).max(near(p_minus)) - half(p_plus).max(half(p_minus))
                } else if p_plus > 0 || p_minus > 0 {
                    let q = p_plus.max(p_minus);
                    -(near(q) + half(q))
                } else {
                    -w / 2.0
                };
                strokes.push((0.0, t));
            }
            for (off, t) in strokes {
                let rect = match dir {
                    0 => Rect::from_min_max(
                        pos2(cx + off - w / 2.0, r.top()),
                        pos2(cx + off + w / 2.0, cy - t),
                    ),
                    1 => Rect::from_min_max(
                        pos2(cx + t, cy + off - w / 2.0),
                        pos2(r.right(), cy + off + w / 2.0),
                    ),
                    2 => Rect::from_min_max(
                        pos2(cx + off - w / 2.0, cy + t),
                        pos2(cx + off + w / 2.0, r.bottom()),
                    ),
                    _ => Rect::from_min_max(
                        pos2(r.left(), cy + off - w / 2.0),
                        pos2(cx - t, cy + off + w / 2.0),
                    ),
                };
                painter.rect_filled(rect, CornerRadius::ZERO, color);
            }
        }
        return true;
    }
    let frac = |f: f32, vertical: bool| -> Rect {
        if vertical {
            Rect::from_min_max(pos2(r.left(), snap(r.bottom() - r.height() * f)), r.max)
        } else {
            Rect::from_min_max(r.min, pos2(snap(r.left() + r.width() * f), r.bottom()))
        }
    };
    let fill = |rect: Rect, col: Color32| {
        painter.rect_filled(rect, CornerRadius::ZERO, col);
    };
    match c {
        '█' => fill(r, color),
        '▀' => fill(
            Rect::from_min_max(r.min, pos2(r.right(), snap(r.center().y))),
            color,
        ),
        '▄' => fill(frac(0.5, true), color),
        '▌' => fill(frac(0.5, false), color),
        '▐' => fill(
            Rect::from_min_max(pos2(snap(r.center().x), r.top()), r.max),
            color,
        ),
        '░' => fill(r, color.gamma_multiply(0.25)),
        '▒' => fill(r, color.gamma_multiply(0.5)),
        '▓' => fill(r, color.gamma_multiply(0.75)),
        '▁'..='▇' => fill(frac((c as u32 - '▁' as u32 + 1) as f32 / 8.0, true), color),
        '▉'..='▏' => fill(
            frac((8 - (c as u32 - '▉' as u32 + 1)) as f32 / 8.0, false),
            color,
        ),
        _ => return false,
    }
    true
}

fn family(bold: bool, italic: bool) -> FontFamily {
    FontFamily::Name(
        match (bold, italic) {
            (false, false) => fonts::TERM_REGULAR,
            (true, false) => fonts::TERM_BOLD,
            (false, true) => fonts::TERM_ITALIC,
            (true, true) => fonts::TERM_BOLD_ITALIC,
        }
        .into(),
    )
}

/// Viewport cell under a screen position, clamped to the grid.
fn cell_at(pos: Pos2, origin: Pos2, cell: Vec2, size: GridSize) -> (usize, usize, Side) {
    let x = (pos.x - origin.x).max(0.0);
    let y = (pos.y - origin.y).max(0.0);
    let col = ((x / cell.x) as usize).min(size.cols - 1);
    let row = ((y / cell.y) as usize).min(size.rows - 1);
    let side = if x - col as f32 * cell.x > cell.x / 2.0 {
        Side::Right
    } else {
        Side::Left
    };
    (col, row, side)
}

/// The run of non-blank cells around a viewport cell: its first and end columns and its text.
fn word_at<T>(term: &term::Term<T>, offset: usize, col: usize, row: usize) -> Option<(usize, usize, String)> {
    let point = grid_point(offset, col, row);
    let cols = term.columns();
    if cols == 0 || col >= cols {
        return None;
    }
    let line = &term.grid()[point.line];
    let chars: Vec<char> = (0..cols).map(|c| line[Column(c)].c).collect();
    let word = |c: char| !c.is_whitespace() && c != '\0';
    if !word(chars[col]) {
        return None;
    }
    let (mut s, mut e) = (col, col);
    while s > 0 && word(chars[s - 1]) {
        s -= 1;
    }
    while e < cols && word(chars[e]) {
        e += 1;
    }
    Some((s, e, chars[s..e].iter().collect()))
}

/// What a link under the pointer is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LinkKind {
    /// A web address in the text.
    Web,
    /// An OSC 8 hyperlink: the text shown is not the address.
    Hyperlink,
    /// A path or file name; the app decides whether it is one.
    Path,
}

/// A link in the terminal: what opens, and the cells it covers.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Link {
    pub target: String,
    pub kind: LinkKind,
    /// Viewport row, first column and end column of each piece in view (a wrapped address
    /// has one per row).
    pub cells: Vec<(usize, usize, usize)>,
}

impl Link {
    /// The text shown is not simply the address (a hyperlink, or one cut over several rows).
    pub fn hidden(&self) -> bool {
        self.kind == LinkKind::Hyperlink || self.cells.len() > 1
    }
}

/// The link under a viewport cell: an OSC 8 hyperlink, a web address (also one wrapped over
/// several rows), or a word that looks like a path.
pub fn link_at<T>(term: &term::Term<T>, offset: usize, col: usize, row: usize) -> Option<Link> {
    let cols = term.columns();
    if cols == 0 || col >= cols || row >= term.screen_lines() {
        return None;
    }
    let point = grid_point(offset, col, row);
    if let Some(link) = term.grid()[point].hyperlink() {
        return Some(hyperlink_cells(term, offset, point, &link));
    }
    if let Some(link) = web_link(term, offset, point) {
        return Some(link);
    }
    let (s, e, word) = word_at(term, offset, col, row)?;
    // Next to a web address but not on it (its bracket, text glued to it): nothing.
    (!word.contains("://") && looks_like_link(&word)).then(|| Link {
        target: word.trim_matches(PATH_PUNCT).to_string(),
        kind: LinkKind::Path,
        cells: vec![(row, s, e)],
    })
}

/// Rows an OSC 8 hyperlink or a wrapped address is followed over, each way.
const LINK_ROWS: usize = 16;

/// The cells of the OSC 8 hyperlink at `point`: the run of cells carrying it, across rows.
fn hyperlink_cells<T>(
    term: &term::Term<T>,
    offset: usize,
    point: Point,
    link: &alacritty_terminal::term::cell::Hyperlink,
) -> Link {
    let cols = term.columns();
    let (top, bottom) = (term.topmost_line(), term.bottommost_line());
    let same = |line: Line, col: usize| term.grid()[line][Column(col)].hyperlink().as_ref() == Some(link);
    let (mut line, mut s) = (point.line, point.column.0);
    let mut cells = Vec::new();
    // Back to where it starts.
    for _ in 0..LINK_ROWS {
        while s > 0 && same(line, s - 1) {
            s -= 1;
        }
        if s == 0 && line > top && same(line - 1, cols - 1) {
            line -= 1;
            s = cols - 1;
        } else {
            break;
        }
    }
    // Then forward, a piece per row.
    for _ in 0..LINK_ROWS * 2 {
        let mut e = s;
        while e < cols && same(line, e) {
            e += 1;
        }
        cells.push((line, s, e));
        if e == cols && line < bottom && same(line + 1, 0) {
            line += 1;
            s = 0;
        } else {
            break;
        }
    }
    Link {
        target: link.uri().to_string(),
        kind: LinkKind::Hyperlink,
        cells: in_view(term, offset, cells),
    }
}

/// Pieces at grid lines → the ones in view, at viewport rows.
fn in_view<T>(term: &term::Term<T>, offset: usize, pieces: Vec<(Line, usize, usize)>) -> Vec<(usize, usize, usize)> {
    let rows = term.screen_lines() as i32;
    pieces
        .into_iter()
        .filter(|&(_, s, e)| e > s)
        .filter_map(|(line, s, e)| {
            let row = line.0 + offset as i32;
            (0..rows).contains(&row).then_some((row as usize, s, e))
        })
        .collect()
}

fn blank(c: char) -> bool {
    c.is_whitespace() || c == '\0'
}

/// A character that may be part of a web address.
fn url_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || "-._~:/?#[]@!$&'()*+,;=%".contains(c)
}

fn line_chars<T>(term: &term::Term<T>, line: Line) -> Vec<char> {
    let row = &term.grid()[line];
    (0..term.columns()).map(|c| row[Column(c)].c).collect()
}

fn wraps<T>(term: &term::Term<T>, line: Line) -> bool {
    let cols = term.columns();
    cols > 0 && term.grid()[line][Column(cols - 1)].flags.contains(Flags::WRAPLINE)
}

/// One row's part of a word: its line, first and end column, and its characters.
struct Piece {
    line: Line,
    s: usize,
    e: usize,
    chars: Vec<char>,
}

impl Piece {
    fn of(line: Line, chars: &[char], s: usize, e: usize) -> Self {
        Piece { line, s, e, chars: chars[s..e].to_vec() }
    }

    fn has_scheme(&self) -> bool {
        self.chars.windows(3).any(|w| w == [':', '/', '/'])
    }
}

/// The piece of a row that carries on an address cut at the end of the row above by the
/// program itself (Ink breaks long words at its width, then indents the rest): web-address
/// characters only, no new address, and more than a word (a word at the next row's start is
/// more likely text).
fn continues_url(chars: &[char]) -> bool {
    let body: &[char] = {
        let mut e = chars.len();
        while e > 0 && matches!(chars[e - 1], '.' | ',' | ';' | ':' | '!' | '?' | ')' | ']' | '\'' | '"') {
            e -= 1;
        }
        &chars[..e]
    };
    !body.is_empty()
        && chars.iter().all(|&c| url_char(c))
        && !chars.windows(3).any(|w| w == [':', '/', '/'])
        && body.iter().any(|c| !c.is_alphabetic())
}

/// The start of a row's first word, when only a small indent comes before it (where Ink
/// continues a word it broke).
const CONTINUE_INDENT: usize = 4;
/// How close to the right edge a row's last word must end to be continued on the next row.
const EDGE: usize = 2;

/// A web address around `point`, followed over rows the terminal wrapped and rows a program
/// broke at the edge itself, without the punctuation around it.
fn web_link<T>(term: &term::Term<T>, offset: usize, point: Point) -> Option<Link> {
    let cols = term.columns();
    let (top, bottom) = (term.topmost_line(), term.bottommost_line());
    let chars = line_chars(term, point.line);
    let col = point.column.0;
    if blank(chars[col]) {
        return None;
    }
    let (mut s, mut e) = (col, col + 1);
    while s > 0 && !blank(chars[s - 1]) {
        s -= 1;
    }
    while e < cols && !blank(chars[e]) {
        e += 1;
    }
    let mut pieces = vec![Piece::of(point.line, &chars, s, e)];
    // Rows above it belongs to.
    while pieces.len() < LINK_ROWS {
        let first = &pieces[0];
        let line = first.line;
        if line <= top {
            break;
        }
        let here = line_chars(term, line);
        let above = line_chars(term, line - 1);
        let Some(ae) = above.iter().rposition(|&c| !blank(c)).map(|i| i + 1) else { break };
        let mut as_ = ae;
        while as_ > 0 && !blank(above[as_ - 1]) {
            as_ -= 1;
        }
        let soft = wraps(term, line - 1) && first.s == 0 && ae == cols;
        let hard = !first.has_scheme()
            && !wraps(term, line - 1)
            && first.s <= CONTINUE_INDENT
            && here[..first.s].iter().all(|&c| blank(c))
            && continues_url(&first.chars)
            && ae + EDGE >= cols
            && above[as_..ae].iter().skip_while(|c| matches!(c, '(' | '[' | '<' | '"' | '\'' | '`')).all(|&c| url_char(c));
        if !(soft || hard) {
            break;
        }
        pieces.insert(0, Piece::of(line - 1, &above, as_, ae));
    }
    let joined: Vec<char> = pieces.iter().flat_map(|p| p.chars.iter().copied()).collect();
    if !joined.windows(3).any(|w| w == [':', '/', '/']) {
        return None;
    }
    // Rows below it goes on to.
    while pieces.len() < LINK_ROWS * 2 {
        let last = pieces.last().unwrap();
        let line = last.line;
        if line >= bottom {
            break;
        }
        let here = line_chars(term, line);
        let below = line_chars(term, line + 1);
        let Some(bs) = below.iter().position(|&c| !blank(c)) else { break };
        let mut be = bs;
        while be < cols && !blank(below[be]) {
            be += 1;
        }
        let soft = wraps(term, line) && last.e == cols && bs == 0;
        let hard = !wraps(term, line)
            && last.e + EDGE >= cols
            && here[last.e..].iter().all(|&c| blank(c))
            && bs <= CONTINUE_INDENT
            && continues_url(&below[bs..be]);
        if !(soft || hard) {
            break;
        }
        pieces.push(Piece::of(line + 1, &below, bs, be));
    }
    let all: Vec<char> = pieces.iter().flat_map(|p| p.chars.iter().copied()).collect();
    let (us, ue) = url_span(&all)?;
    // The address's characters back on the rows they are drawn on.
    let mut cells = Vec::new();
    let mut at = 0;
    for p in &pieces {
        let (a, b) = (us.max(at), ue.min(at + p.chars.len()));
        if a < b {
            cells.push((p.line, p.s + (a - at), p.s + (b - at)));
        }
        at += p.chars.len();
    }
    // Only when the pointer is on the address itself (not on text glued to it).
    let on_it = cells.iter().any(|&(l, a, b)| l == point.line && (a..b).contains(&col));
    on_it.then(|| Link {
        target: all[us..ue].iter().collect(),
        kind: LinkKind::Web,
        cells: in_view(term, offset, cells),
    })
}

/// Where a web address is in a word: from `http://` or `https://` to its end, without the
/// punctuation after it (a closing bracket stays when the address opened one, as in
/// `…/wiki/Rust_(language)`). None without a host.
pub fn url_span(chars: &[char]) -> Option<(usize, usize)> {
    let starts = |i: usize, p: &str| p.chars().enumerate().all(|(k, c)| chars.get(i + k).is_some_and(|x| x.eq_ignore_ascii_case(&c)));
    let s = (0..chars.len()).find(|&i| starts(i, "http://") || starts(i, "https://"))?;
    let host = s + if starts(s, "https://") { 8 } else { 7 };
    // Up to a character an address never has.
    let mut e = (s..chars.len()).find(|&i| !url_char(chars[i])).unwrap_or(chars.len());
    loop {
        if e <= host {
            return None;
        }
        let c = chars[e - 1];
        let count = |ch: char| chars[s..e].iter().filter(|&&x| x == ch).count();
        let unbalanced = match c {
            ')' => count('(') < count(')'),
            ']' => count('[') < count(']'),
            _ => false,
        };
        if unbalanced || matches!(c, '.' | ',' | ';' | ':' | '!' | '?' | '\'' | '*') {
            e -= 1;
        } else {
            break;
        }
    }
    let host_end = (host..e).find(|&i| matches!(chars[i], '/' | '?' | '#')).unwrap_or(e);
    chars[host..host_end].iter().any(|c| c.is_ascii_alphanumeric()).then_some((s, e))
}

/// Punctuation around a path in text (quotes, brackets, a list's comma).
const PATH_PUNCT: &[char] = &['"', '\'', '`', '(', ')', '[', ']', '<', '>', ',', ';'];

/// Whether a word reads as something ⌘-click opens: a web address, or a path or file name
/// (`src/app.rs:120`, `./run.sh`, `Cargo.toml`). The app decides what it really is.
pub fn looks_like_link(word: &str) -> bool {
    let w = word.trim_matches(PATH_PUNCT);
    if w.starts_with("http://") || w.starts_with("https://") {
        return w.len() > "https://".len();
    }
    let path = w.split(':').next().unwrap_or("");
    if path.is_empty() || path.chars().all(|c| c == '.' || c == '/') {
        return false;
    }
    if path.contains('/') {
        return true;
    }
    // A file name: letters before a dot and an extension after it (not a number like 1.5).
    match path.rsplit_once('.') {
        Some((stem, ext)) => {
            !stem.is_empty()
                && !ext.is_empty()
                && ext.len() <= 8
                && ext.chars().all(|c| c.is_ascii_alphanumeric())
                && ext.chars().any(|c| c.is_ascii_alphabetic())
        }
        None => false,
    }
}

/// At most `max` characters of `s`, with an ellipsis when cut.
fn shorten(s: &str, max: usize) -> String {
    match s.char_indices().nth(max) {
        Some((i, _)) => format!("{}…", &s[..i]),
        None => s.to_string(),
    }
}

fn grid_point(offset: usize, col: usize, row: usize) -> Point {
    term::viewport_to_point(offset, Point::new(row, Column(col)))
}

fn mods_of(m: egui::Modifiers) -> Mods {
    Mods {
        shift: m.shift,
        alt: m.alt,
        ctrl: m.ctrl,
    }
}

/// A Ctrl+key egui made a clipboard event of (Windows, Linux), as the program gets the key.
fn ctrl_key(key: Key, modifiers: egui::Modifiers, modes: KeyModes) -> Vec<u8> {
    let bytes = if modes.kitty == 0 {
        // The control character alone, whatever else is held.
        input::encode_key(key, Mods { ctrl: true, ..Mods::default() }, modes)
    } else {
        let mods = Mods { ctrl: true, ..mods_of(modifiers) };
        input::encode_event(&input::KeyEvent::press(key, mods), None, modes)
    };
    bytes.unwrap_or_default()
}

#[allow(clippy::too_many_arguments)]
pub fn show(
    ui: &mut Ui,
    rect: Rect,
    view: &mut TermView,
    shared: &TermShared,
    style: &TermStyle,
    active: bool,
    input_enabled: bool,
    alive: bool,
) -> TermResponse {
    show_inner(ui, rect, view, shared, style, active, input_enabled, alive, None, Crop::default(), true)
}

/// The grid a terminal drawn in `rect` gets (what `show` resizes it to). Callers use it to
/// start a program at its final size, so it does not see a resize right after starting.
pub fn grid_for(rect: Rect, cell: Vec2) -> GridSize {
    // A cell of no size (a font of size 0) would make an endless grid.
    let cell = vec2(cell.x.max(1.0), cell.y.max(1.0));
    let count = |avail: f32, size: f32, min: usize| {
        let n = (avail / size).floor();
        if n.is_finite() && n > 0.0 { (n as usize).clamp(min, 4096) } else { min }
    };
    GridSize {
        cols: count(rect.width() - 2.0 * PAD, cell.x, tcore::MIN_COLS),
        rows: count(rect.height() - 2.0 * PAD, cell.y, tcore::MIN_ROWS),
    }
}

/// The scale at which the terminal's current grid fits into `rect` (1.0 = the style's size).
pub fn fit_scale(rect: Rect, shared: &TermShared, style: &TermStyle) -> (GridSize, f32) {
    let size = {
        let term = shared.term.lock();
        GridSize {
            cols: term.columns(),
            rows: term.screen_lines(),
        }
    };
    let avail = rect.size() - vec2(2.0 * PAD, 2.0 * PAD);
    let scale = (avail.x / (size.cols as f32 * style.cell.x))
        .min(avail.y / (size.rows as f32 * style.cell.y))
        .clamp(0.05, 1.0);
    (size, scale)
}

/// Draws the terminal at its current grid size (columns × rows), scaled down to fit `rect` but
/// never below `min_scale`, so the text stays readable. The grid and the PTY are never resized,
/// so programs on it don't redraw when the view changes (the Grid, FR-V3). When the grid does
/// not fit at that size it is cut: the left columns, and the rows around the cursor (the latest
/// output, or the input line of a full-screen program). Input works as in `show`.
#[allow(clippy::too_many_arguments)]
pub fn show_readable(
    ui: &mut Ui,
    rect: Rect,
    view: &mut TermView,
    shared: &TermShared,
    style: &TermStyle,
    input_enabled: bool,
    alive: bool,
    min_scale: f32,
    wheel: bool,
) -> TermResponse {
    let (size, fit) = fit_scale(rect, shared, style);
    let scale = fit.max(min_scale).min(1.0);
    let scaled = TermStyle {
        font_size: style.font_size * scale,
        cell: style.cell * scale,
        ..*style
    };
    let visible = (((rect.height() - 2.0 * PAD) / scaled.cell.y).floor() as usize).max(1);
    let crop = if visible >= size.rows {
        Crop::default()
    } else {
        let term = shared.term.lock();
        if term.grid().display_offset() > 0 {
            // Scrolled back through the history: the top of what is in view.
            Crop { rows: Some((0..visible).collect()), cut_above: false }
        } else {
            let blank = blank_rows(&term);
            let cursor = term.grid().cursor.point.line.0.max(0) as usize;
            let bottom = blank.iter().rposition(|b| !b).unwrap_or(0);
            let (rows, cut_above) = rows_shown(&blank, visible, cursor, bottom);
            Crop { rows: Some(rows), cut_above }
        }
    };
    show_inner(ui, rect, view, shared, &scaled, false, input_enabled, alive, Some(size), crop, wheel)
}

/// The rows a cut view shows (`show_readable`).
#[derive(Clone, Default)]
struct Crop {
    /// Screen rows drawn, top to bottom (none: every row, as the rect holds them).
    rows: Option<Vec<usize>>,
    /// Rows were cut above: the first one listed is drawn half under the top edge, faded.
    cut_above: bool,
}

/// For each screen row: nothing on it (no character, no background colour).
fn blank_rows<T>(term: &term::Term<T>) -> Vec<bool> {
    let cols = term.columns();
    (0..term.screen_lines())
        .map(|l| {
            let row = &term.grid()[Line(l as i32)];
            (0..cols).all(|c| {
                let cell = &row[Column(c)];
                matches!(cell.c, ' ' | '\0')
                    && cell.bg == Color::Named(NamedColor::Background)
                    && !cell.flags.contains(Flags::INVERSE)
            })
        })
        .collect()
}

/// The screen rows a view of `visible` rows shows when the screen has more. Empty space goes
/// first: rows below the cursor and the last text, then runs of blank rows, narrowed evenly
/// (an agent with its input pinned to the bottom and empty space above it keeps its header).
/// What still does not fit is cut above, keeping the cursor and what is drawn under it
/// (`rows_to_skip`); the row above the first one shown is then listed too, to be drawn half
/// under the top edge. Returns the rows and whether rows were cut above.
pub fn rows_shown(blank: &[bool], visible: usize, cursor: usize, bottom: usize) -> (Vec<usize>, bool) {
    let rows = blank.len();
    if visible >= rows {
        return ((0..rows).collect(), false);
    }
    let last = cursor.max(bottom).min(rows.saturating_sub(1));
    let empty = |r: usize| blank[r] && r != cursor;
    // Runs of empty rows up to the last row with something on it.
    let mut runs: Vec<(usize, usize)> = Vec::new(); // start, length
    let mut r = 0;
    while r <= last {
        if empty(r) {
            let start = r;
            while r <= last && empty(r) {
                r += 1;
            }
            runs.push((start, r - start));
        } else {
            r += 1;
        }
    }
    let filled = last + 1 - runs.iter().map(|(_, n)| n).sum::<usize>();
    // The most rows each run keeps (at least one, as a gap) with everything fitting.
    let longest = runs.iter().map(|(_, n)| *n).max().unwrap_or(0);
    let cap = (1..=longest.max(1))
        .rev()
        .find(|&cap| filled + runs.iter().map(|(_, n)| (*n).min(cap)).sum::<usize>() <= visible)
        .unwrap_or(1);
    let kept: Vec<usize> = (0..=last)
        .filter(|&r| {
            !empty(r)
                || runs
                    .iter()
                    .any(|&(start, n)| r >= start && r < start + n && r - start < n.min(cap))
        })
        .collect();
    if kept.len() <= visible {
        return (kept, false);
    }
    let at = |row: usize| kept.iter().position(|&r| r == row).unwrap_or(kept.len() - 1);
    let skip = rows_to_skip(kept.len(), visible, at(cursor), at(bottom.min(last)));
    if skip == 0 {
        return (kept[..visible].to_vec(), false);
    }
    (kept[skip - 1..skip + visible].to_vec(), true)
}

/// Rows hidden above a view of `visible` rows: the cursor's row is always in it, and as much
/// as fits of what is drawn below it (`bottom`, the last row with text: an agent's input box
/// border and status line sit under its cursor). None when all rows fit.
pub fn rows_to_skip(rows: usize, visible: usize, cursor: usize, bottom: usize) -> usize {
    if visible >= rows {
        return 0;
    }
    let end = cursor.max(bottom.min(cursor + visible - 1));
    (end + 1).saturating_sub(visible).min(rows - visible)
}

/// A gradient from `color` at the outer edge (top, or right when `right`) to nothing, over
/// text cut at that edge.
fn fade(painter: &egui::Painter, r: Rect, color: Color32, right: bool) {
    let clear = Color32::TRANSPARENT;
    let (tl, tr, bl, br) = if right {
        (clear, color, clear, color)
    } else {
        (color, color, clear, clear)
    };
    let mut mesh = egui::Mesh::default();
    mesh.colored_vertex(r.left_top(), tl);
    mesh.colored_vertex(r.right_top(), tr);
    mesh.colored_vertex(r.left_bottom(), bl);
    mesh.colored_vertex(r.right_bottom(), br);
    mesh.add_triangle(0, 1, 2);
    mesh.add_triangle(1, 3, 2);
    painter.add(egui::Shape::mesh(mesh));
}

#[allow(clippy::too_many_arguments)]
fn show_inner(
    ui: &mut Ui,
    rect: Rect,
    view: &mut TermView,
    shared: &TermShared,
    style: &TermStyle,
    active: bool,
    input_enabled: bool,
    alive: bool,
    fixed: Option<GridSize>,
    crop: Crop,
    wheel: bool,
) -> TermResponse {
    let mut out = TermResponse::default();
    let id = *view.id.get_or_insert_with(|| {
        Id::new((
            "terminal",
            rect.min.x as i64,
            rect.min.y as i64,
            ui.next_auto_id(),
        ))
    });
    let ppp = ui.ctx().pixels_per_point();
    let cell = style.cell;
    // A cut view draws its listed rows in slots, whole rows only. Cut above, the last slot sits
    // on the bottom padding and the first (the row above) fades out under the top edge.
    let (origin_y, clip) = match &crop.rows {
        Some(rows) if crop.cut_above => {
            let bottom = rect.max.y - PAD;
            (
                bottom - rows.len() as f32 * cell.y,
                Rect::from_min_max(rect.min, pos2(rect.max.x, bottom)),
            )
        }
        Some(rows) => (
            rect.min.y + PAD,
            Rect::from_min_max(rect.min, pos2(rect.max.x, (rect.min.y + PAD + rows.len() as f32 * cell.y).min(rect.max.y))),
        ),
        None => (rect.min.y + PAD, rect),
    };
    let origin = pos2(rect.min.x + PAD, origin_y).round_to_pixels(ppp);
    let size = fixed.unwrap_or_else(|| grid_for(rect, cell));
    // Screen row → slot it is drawn in, and back (pointer positions).
    let slots: Option<Vec<Option<usize>>> = crop.rows.as_ref().map(|rows| {
        let mut slots = vec![None; size.rows];
        for (slot, &r) in rows.iter().enumerate() {
            if r < size.rows {
                slots[r] = Some(slot);
            }
        }
        slots
    });
    let slot_of = |row: usize| match &slots {
        Some(s) => s.get(row).copied().flatten(),
        None => Some(row),
    };
    let at = |p: Pos2| {
        let (col, slot, side) = match &crop.rows {
            Some(rows) => cell_at(p, origin, cell, GridSize { cols: size.cols, rows: rows.len().max(1) }),
            None => cell_at(p, origin, cell, size),
        };
        let row = crop.rows.as_ref().map_or(slot, |rows| rows.get(slot).copied().unwrap_or(slot));
        (col, row.min(size.rows - 1), side)
    };
    // Pixel sizes go out in physical pixels everywhere (the PTY's size, CSI 14 t),
    // and are re-sent when only the font changes.
    let cell_px = ((cell.x * ppp).round() as u32, (cell.y * ppp).round() as u32);
    // Also when the terminal was resized elsewhere (the Grid sizes it to its tile).
    let actual = fixed.is_none().then(|| {
        let term = shared.term.lock();
        GridSize { cols: term.columns(), rows: term.screen_lines() }
    });
    if fixed.is_none() && (view.grid != Some(size) || view.cell_px != Some(cell_px) || actual != Some(size)) {
        view.grid = Some(size);
        view.cell_px = Some(cell_px);
        tcore::resize(shared, size, cell_px.0 as f32, cell_px.1 as f32);
        out.resized = Some((
            size,
            size.cols as u32 * cell_px.0,
            size.rows as u32 * cell_px.1,
        ));
    }

    // ----- focus -------------------------------------------------------------------------
    // Keys are only taken in a frame that began with this terminal focused: when it takes
    // focus during the frame (e.g. after Enter in the sidebar search released it), that
    // frame's keys were meant for the other widget.
    let had_focus = ui.memory(|m| m.had_focus_last_frame(id));
    if ui.memory(|m| m.has_focus(id)) {
        // Arrows and Tab belong to the program, never to egui's focus navigation (which
        // is decided before the focus filter below applies on the first focused frame).
        ui.memory_mut(|m| m.move_focus(egui::FocusDirection::None));
    }
    let response = ui.interact(rect, id, Sense::click_and_drag());
    if response.clicked_by(PointerButton::Primary)
        || response.drag_started_by(PointerButton::Primary)
        || response.clicked_by(PointerButton::Secondary)
        || response.clicked_by(PointerButton::Middle)
    {
        out.activated = true;
        if input_enabled {
            ui.memory_mut(|m| m.request_focus(id));
            // Taking focus ends any composition (macOS sends nothing for it).
            view.preedit.clear();
        }
    }
    if active && input_enabled && ui.memory(|m| m.focused().is_none()) {
        ui.memory_mut(|m| m.request_focus(id));
    }
    let focused = input_enabled && ui.memory(|m| m.has_focus(id));
    let window_focused = ui.input(|i| i.focused);
    out.focused = focused;
    if focused {
        ui.memory_mut(|m| m.set_focus_lock_filter(id, TERM_FILTER));
    } else {
        // A composition left behind when focus moved away would hide the cursor.
        view.preedit.clear();
    }

    let mut term = shared.term.lock();
    let mode = *term.mode();
    term.is_focused = focused && window_focused;

    // Focus reporting (DEC 1004).
    let has_focus_now = focused && window_focused;
    if has_focus_now != view.was_focused {
        if has_focus_now {
            view.blink_epoch = ui.input(|i| i.time);
        }
        if mode.contains(TermMode::FOCUS_IN_OUT) && alive {
            out.input
                .extend_from_slice(if has_focus_now { b"\x1b[I" } else { b"\x1b[O" });
        }
        view.was_focused = has_focus_now;
    }

    // ----- keyboard ----------------------------------------------------------------------
    if focused && had_focus {
        let events = ui.input(|i| i.filtered_events(&TERM_FILTER));
        // Option as Meta: text typed with Option held goes out as ESC + the character the
        // layout produces without Option (winit delivers it that way), for any layout.
        let meta = style.option_as_meta && ui.input(|i| i.modifiers.alt && !i.modifiers.ctrl);
        let modes = KeyModes::of(mode);
        // Taken in the first frame that reads keys after the request, whether the paste came or
        // not (an empty clipboard sends none): a later Ctrl+V is a key again.
        let menu_paste = std::mem::take(&mut view.paste_requested);
        let mut typed = Vec::new();
        // With the kitty keyboard protocol a key waits for the next event: the text it typed comes
        // right after it, and goes out with the key instead of on its own.
        let mut held: Option<input::KeyEvent> = None;
        for event in events {
            if let Some(key) = held.take() {
                let text = match &event {
                    Event::Text(t) => Some(t.as_str()),
                    _ => None,
                };
                if let Some(bytes) = input::encode_event(&key, text, modes) {
                    typed.extend(bytes);
                    if text.is_some() {
                        continue;
                    }
                }
            }
            match event {
                Event::Key {
                    key,
                    physical_key,
                    pressed,
                    repeat,
                    modifiers,
                } => {
                    // The paste shortcut with no text on the clipboard (an image) sends no paste
                    // event, only this release: Ctrl+V goes to the program on Linux as usual;
                    // elsewhere the app passes it to an agent, which reads the clipboard itself.
                    if key == Key::V && !pressed && modifiers.command && !std::mem::take(&mut view.pasted_since_v) && alive {
                        let bytes = ctrl_key(Key::V, modifiers, modes);
                        if cfg!(target_os = "linux") && !modifiers.shift {
                            typed.extend(bytes);
                        } else {
                            out.paste_without_text = Some(bytes);
                        }
                    }
                    // ⌘ combinations are the app's (macOS). On Windows and Linux Ctrl goes to the
                    // program; the app's own shortcuts were taken out before.
                    if modifiers.mac_cmd {
                        continue;
                    }
                    let mut mods = mods_of(modifiers);
                    if modifiers.shift && !mode.contains(TermMode::ALT_SCREEN) {
                        let scroll = match key {
                            Key::PageUp => Some(Scroll::PageUp),
                            Key::PageDown => Some(Scroll::PageDown),
                            Key::Home => Some(Scroll::Top),
                            Key::End => Some(Scroll::Bottom),
                            _ => None,
                        };
                        if let Some(s) = scroll {
                            if pressed {
                                term.scroll_display(s);
                            }
                            continue;
                        }
                    }
                    if !alive {
                        if pressed && key == Key::Enter {
                            out.reconnect = true;
                        }
                        continue;
                    }
                    if modes.kitty == 0 {
                        if pressed && let Some(bytes) = input::encode_key(key, mods, modes) {
                            typed.extend(bytes);
                        }
                        continue;
                    }
                    // Option that is not Meta types characters: on a text key it is no modifier.
                    if !style.option_as_meta && !mods.ctrl && input::is_text_key(key) {
                        mods.alt = false;
                    }
                    let action = match (pressed, repeat) {
                        (false, _) => input::KeyAction::Release,
                        (true, true) => input::KeyAction::Repeat,
                        (true, false) => input::KeyAction::Press,
                    };
                    let key = input::KeyEvent {
                        key,
                        physical: physical_key,
                        mods,
                        action,
                    };
                    if pressed {
                        held = Some(key);
                    } else {
                        // A release types nothing.
                        typed.extend(input::encode_event(&key, None, modes).unwrap_or_default());
                    }
                }
                Event::Text(text) => {
                    if !alive {
                        continue;
                    }
                    if meta {
                        typed.push(0x1b);
                    }
                    typed.extend_from_slice(text.as_bytes());
                }
                // On Windows and Linux egui turns Ctrl+C, Ctrl+X and Ctrl+V into these events, but
                // in a terminal plain Ctrl+C belongs to the program: only Ctrl+Shift+C
                // and Ctrl+Shift+V (and the menu) copy and paste, as in other terminals there.
                // Windows Terminal's habits also work there: Ctrl+C copies while text is
                // selected, and Ctrl+V pastes.
                Event::Paste(text) if alive => {
                    if !menu_paste {
                        view.pasted_since_v = true;
                    }
                    let shift = ui.input(|i| i.modifiers.shift);
                    if cfg!(target_os = "macos") || cfg!(windows) || shift || menu_paste {
                        typed.extend(input::encode_paste(
                            &text,
                            mode.contains(TermMode::BRACKETED_PASTE),
                        ));
                    } else {
                        typed.extend(ctrl_key(Key::V, ui.input(|i| i.modifiers), modes));
                    }
                }
                Event::Copy | Event::Cut => {
                    let shift = ui.input(|i| i.modifiers.shift);
                    let selected = term.selection_to_string().filter(|s| !s.is_empty());
                    let copies =
                        cfg!(target_os = "macos") || shift || (cfg!(windows) && selected.is_some());
                    if copies {
                        if let Some(text) = selected {
                            ui.ctx().copy_text(text);
                            if !cfg!(target_os = "macos") {
                                term.selection = None;
                            }
                        }
                    } else if alive {
                        let key = if event == Event::Copy { Key::C } else { Key::X };
                        typed.extend(ctrl_key(key, ui.input(|i| i.modifiers), modes));
                    }
                }
                Event::Ime(ImeEvent::Preedit { text, .. }) => view.preedit = text,
                Event::Ime(ImeEvent::Commit(text)) => {
                    view.preedit.clear();
                    if alive {
                        typed.extend_from_slice(text.as_bytes());
                    }
                }
                _ => {}
            }
        }
        if let Some(key) = held.take() {
            typed.extend(input::encode_event(&key, None, modes).unwrap_or_default());
        }
        if !typed.is_empty() {
            term.scroll_display(Scroll::Bottom);
            term.selection = None;
            view.blink_epoch = ui.input(|i| i.time);
            out.input.extend(typed);
        }
    }

    // ----- mouse -------------------------------------------------------------------------
    let offset = term.grid().display_offset();
    let hover = response.hover_pos();
    let mouse_mode = mode.intersects(TermMode::MOUSE_MODE) && alive;
    let shift_held = ui.input(|i| i.modifiers.shift);
    let sgr = mode.contains(TermMode::SGR_MOUSE);
    // The link under the pointer. With the link modifier held it is underlined (the pointer a
    // hand) and a click opens it, also in a program that takes the mouse: that click is not
    // reported to it.
    let pointer_link = hover.filter(|_| input_enabled).and_then(|p| {
        let (col, row, _) = at(p);
        link_at(&term, offset, col, row)
    });
    let link_mod = ui.input(|i| link_modifier(i.modifiers));
    let mut link_hover: Vec<(usize, usize, usize)> = Vec::new();
    if link_mod && let Some(link) = &pointer_link {
        link_hover = link.cells.clone();
        if response.clicked_by(PointerButton::Primary) {
            out.link = Some(link.target.clone());
        }
    }
    if response.secondary_clicked() {
        view.menu_link = pointer_link.clone().filter(|l| l.kind != LinkKind::Path);
    }
    if mouse_mode && !shift_held && input_enabled {
        let events = ui.input(|i| i.events.clone());
        for event in events {
            match event {
                Event::PointerButton {
                    pos,
                    button,
                    pressed,
                    modifiers,
                } => {
                    let b = match button {
                        PointerButton::Primary => MouseButton::Left,
                        PointerButton::Middle => MouseButton::Middle,
                        PointerButton::Secondary => MouseButton::Right,
                        _ => continue,
                    };
                    // Only presses that land on the terminal itself, not on a menu, popup or
                    // dialog drawn over it, and not a click that opens a link.
                    if pressed && (!rect.contains(pos) || !on_terminal_layer(ui, pos)) {
                        continue;
                    }
                    if pressed && b == MouseButton::Left && link_modifier(modifiers) && !link_hover.is_empty() {
                        continue;
                    }
                    if !pressed && view.reported_button != Some(b) {
                        continue;
                    }
                    let (col, row, _) = at(pos);
                    let action = if pressed {
                        MouseAction::Press
                    } else {
                        MouseAction::Release
                    };
                    view.reported_button = pressed.then_some(b);
                    view.last_report_cell = Some((col, row));
                    if let Some(bytes) =
                        input::encode_mouse(b, action, mods_of(modifiers), col, row, sgr)
                    {
                        out.input.extend(bytes);
                    }
                }
                Event::PointerMoved(pos) => {
                    let report = match view.reported_button {
                        Some(_) => mode.intersects(TermMode::MOUSE_DRAG | TermMode::MOUSE_MOTION),
                        None => {
                            mode.contains(TermMode::MOUSE_MOTION)
                                && rect.contains(pos)
                                && on_terminal_layer(ui, pos)
                        }
                    };
                    if !report {
                        continue;
                    }
                    let (col, row, _) = at(pos);
                    if view.last_report_cell == Some((col, row)) {
                        continue;
                    }
                    view.last_report_cell = Some((col, row));
                    let b = view.reported_button.unwrap_or(MouseButton::None);
                    let mods = mods_of(ui.input(|i| i.modifiers));
                    if let Some(bytes) =
                        input::encode_mouse(b, MouseAction::Motion, mods, col, row, sgr)
                    {
                        out.input.extend(bytes);
                    }
                }
                _ => {}
            }
        }
    } else if input_enabled {
        let press_origin = ui.input(|i| i.pointer.press_origin());
        if response.drag_started_by(PointerButton::Primary)
            && let Some(p) = press_origin
        {
            let (col, row, side) = at(p);
            let ty = if ui.input(|i| i.modifiers.alt) {
                SelectionType::Block
            } else {
                SelectionType::Simple
            };
            term.selection = Some(Selection::new(ty, grid_point(offset, col, row), side));
        }
        if response.dragged_by(PointerButton::Primary)
            && let Some(p) = response.interact_pointer_pos()
        {
            // Auto-scroll while dragging past the edges.
            if p.y < rect.top() {
                term.scroll_display(Scroll::Delta(1));
                ui.ctx().request_repaint_after(Duration::from_millis(30));
            } else if p.y > rect.bottom() {
                term.scroll_display(Scroll::Delta(-1));
                ui.ctx().request_repaint_after(Duration::from_millis(30));
            }
            let offset = term.grid().display_offset();
            let (col, row, side) = at(p);
            if let Some(sel) = term.selection.as_mut() {
                sel.update(grid_point(offset, col, row), side);
            }
        }
        if response.drag_stopped()
            && style.copy_on_select
            && let Some(text) = term.selection_to_string()
        {
            ui.ctx().copy_text(text);
        }
        if let Some(p) = hover {
            let (col, row, side) = at(p);
            if response.triple_clicked() {
                term.selection = Some(Selection::new(
                    SelectionType::Lines,
                    grid_point(offset, col, row),
                    side,
                ));
            } else if response.double_clicked() {
                term.selection = Some(Selection::new(
                    SelectionType::Semantic,
                    grid_point(offset, col, row),
                    side,
                ));
            } else if response.clicked_by(PointerButton::Primary) {
                term.selection = None;
            }
            if (response.double_clicked() || response.triple_clicked())
                && style.copy_on_select
                && let Some(text) = term.selection_to_string()
            {
                ui.ctx().copy_text(text);
            }
        }
    }

    // Wheel: report, send arrows (alternate screen), or scroll the history. Without `wheel` it is
    // left to what contains the terminal (a scrolling Grid); with it, a containing scroll area
    // gets none of it.
    if let (Some(hover_pos), true) = (hover, input_enabled && wheel) {
        ui.input_mut(|i| i.smooth_scroll_delta = Vec2::ZERO);
        let wheel: f32 = ui.input(|i| {
            i.events
                .iter()
                .map(|e| match e {
                    Event::MouseWheel { unit, delta, .. } => match unit {
                        MouseWheelUnit::Point => delta.y / cell.y,
                        MouseWheelUnit::Line => delta.y,
                        MouseWheelUnit::Page => delta.y * size.rows as f32,
                    },
                    _ => 0.0,
                })
                .sum()
        });
        view.scroll_accum += wheel;
        let lines = view.scroll_accum.trunc() as i32;
        if lines != 0 {
            view.scroll_accum -= lines as f32;
            if mouse_mode && !shift_held {
                let (col, row, _) = at(hover_pos);
                let b = if lines > 0 {
                    MouseButton::WheelUp
                } else {
                    MouseButton::WheelDown
                };
                for _ in 0..lines.abs().min(10) {
                    if let Some(bytes) =
                        input::encode_mouse(b, MouseAction::Press, Mods::default(), col, row, sgr)
                    {
                        out.input.extend(bytes);
                    }
                }
            } else if mode.contains(TermMode::ALT_SCREEN | TermMode::ALTERNATE_SCROLL) && alive {
                let seq: &[u8] = if lines > 0 { b"\x1bOA" } else { b"\x1bOB" };
                for _ in 0..lines.abs().min(10) {
                    out.input.extend_from_slice(seq);
                }
            } else {
                term.scroll_display(Scroll::Delta(lines));
            }
        }
    }
    // Resting on a web address: how to open it, and where it goes when the text does not say.
    if !link_mod
        && let Some(link) = pointer_link.as_ref().filter(|l| l.kind != LinkKind::Path)
    {
        let hint = if cfg!(target_os = "macos") {
            crate::tr("⌘-click to open")
        } else {
            crate::tr("Ctrl-click to open")
        };
        let target = link.hidden().then(|| shorten(&link.target, 200));
        response.clone().on_hover_ui_at_pointer(|ui| {
            ui.set_max_width(420.0);
            ui.label(hint);
            if let Some(t) = target {
                ui.label(egui::RichText::new(t).small().weak());
            }
        });
    }
    if hover.is_some() {
        ui.ctx().set_cursor_icon(if !link_hover.is_empty() {
            egui::CursorIcon::PointingHand
        } else if mouse_mode && !shift_held {
            egui::CursorIcon::Default
        } else {
            egui::CursorIcon::Text
        });
    }

    // ----- render ------------------------------------------------------------------------
    let content = term.renderable_content();
    let colors = content.colors;
    let scheme = style.scheme;
    let default_bg = palette(257, colors, scheme);
    ui.painter_at(rect).rect_filled(rect, CornerRadius::ZERO, default_bg);
    let painter = ui.painter_at(clip);

    let offset = content.display_offset;
    let selection = content.selection;
    let cursor = content.cursor;
    let snap = |v: f32| (v * ppp).round() / ppp;
    let cell_rect = |col: usize, row: usize, width: usize| {
        let x0 = snap(origin.x + col as f32 * cell.x);
        let x1 = snap(origin.x + (col + width) as f32 * cell.x);
        let y0 = snap(origin.y + row as f32 * cell.y);
        let y1 = snap(origin.y + (row + 1) as f32 * cell.y);
        Rect::from_min_max(pos2(x0, y0), pos2(x1, y1))
    };

    let mut runs: Vec<(usize, TextRun)> = Vec::new();
    let mut singles: Vec<(usize, usize, String, RunStyle, usize)> = Vec::new(); // row, col, text, style, width
    let mut bg_run: Option<(usize, usize, usize, Color32)> = None; // row, start col, end col, color
    let flush_bg = |run: &mut Option<(usize, usize, usize, Color32)>| {
        if let Some((row, start, end, color)) = run.take() {
            let r = cell_rect(start, row, end - start);
            painter.rect_filled(r, CornerRadius::ZERO, color);
        }
    };
    let mut current: Option<(usize, TextRun)> = None;
    let mut cursor_cell: Option<(usize, usize, String, Color32, Color32, bool)> = None;

    for indexed in content.display_iter {
        let Some(vp) = term::point_to_viewport(offset, indexed.point) else {
            continue;
        };
        let (row, col) = (vp.line, vp.column.0);
        if row >= size.rows || col >= size.cols {
            continue;
        }
        // From here on `row` is where it is drawn (its slot in a cut view).
        let Some(row) = slot_of(row) else {
            continue;
        };
        let flags = indexed.cell.flags;
        if flags.contains(Flags::WIDE_CHAR_SPACER) {
            continue;
        }
        let mut fg = resolve_fg(indexed.cell.fg, flags, colors, scheme);
        let mut bg = resolve_bg(indexed.cell.bg, colors, scheme);
        if flags.contains(Flags::INVERSE) {
            let new_fg = bg.unwrap_or(default_bg);
            bg = Some(fg);
            fg = new_fg;
        }
        let selected = selection.is_some_and(|s| s.contains(indexed.point));
        if selected {
            bg = Some(scheme.selection);
        }
        let width = if flags.contains(Flags::WIDE_CHAR) {
            2
        } else {
            1
        };

        // Background runs.
        match (&mut bg_run, bg) {
            (Some((r, _, end, color)), Some(c)) if *r == row && *end == col && *color == c => {
                *end = col + width
            }
            (_, Some(c)) => {
                flush_bg(&mut bg_run);
                bg_run = Some((row, col, col + width, c));
            }
            (_, None) => flush_bg(&mut bg_run),
        }

        let mut ch = indexed.cell.c;
        if ch == '\t' || flags.contains(Flags::LEADING_WIDE_CHAR_SPACER) {
            ch = ' ';
        }
        let hidden = flags.contains(Flags::HIDDEN);
        let underline = if flags.contains(Flags::DOUBLE_UNDERLINE) {
            2
        } else if flags
            .intersects(Flags::UNDERCURL | Flags::DOTTED_UNDERLINE | Flags::DASHED_UNDERLINE)
        {
            3
        } else if flags.contains(Flags::UNDERLINE) {
            1
        } else {
            0
        };
        let run_style = RunStyle {
            fg,
            bold: flags.contains(Flags::BOLD),
            italic: flags.contains(Flags::ITALIC),
            underline,
            underline_color: indexed
                .cell
                .underline_color()
                .map(|c| resolve_fg(c, flags, colors, scheme)),
            strike: flags.contains(Flags::STRIKEOUT),
        };

        let is_cursor = cursor.shape != CursorShape::Hidden
            && cursor.point == indexed.point
            && (cursor.point.line.0 + offset as i32) < size.rows as i32;
        if is_cursor {
            let mut text = ch.to_string();
            if let Some(extra) = indexed.cell.zerowidth() {
                text.extend(extra.iter());
            }
            cursor_cell = Some((row, col, text, fg, bg.unwrap_or(default_bg), width == 2));
        }

        let zerowidth = indexed.cell.zerowidth().is_some_and(|z| !z.is_empty());
        let simple = ch.is_ascii() && width == 1 && !zerowidth;
        let blank = ch == ' ' && underline == 0 && !run_style.strike;
        if hidden || blank {
            if let Some(run) = current.take() {
                runs.push(run);
            }
            continue;
        }
        if simple {
            match &mut current {
                Some((r, run))
                    if *r == row && run.col + run.text.len() == col && run.style == run_style =>
                {
                    run.text.push(ch)
                }
                _ => {
                    if let Some(run) = current.take() {
                        runs.push(run);
                    }
                    current = Some((
                        row,
                        TextRun {
                            col,
                            text: ch.to_string(),
                            style: run_style,
                        },
                    ));
                }
            }
        } else {
            if let Some(run) = current.take() {
                runs.push(run);
            }
            let mut text = ch.to_string();
            if let Some(extra) = indexed.cell.zerowidth() {
                text.extend(extra.iter());
            }
            singles.push((row, col, text, run_style, width));
        }
    }
    flush_bg(&mut bg_run);
    if let Some(run) = current.take() {
        runs.push(run);
    }

    let paint_text =
        |row: usize, col: usize, text: &str, st: &RunStyle, width: usize, color: Color32| {
            let r = cell_rect(col, row, width);
            if text.chars().count() == 1
                && draw_special(&painter, text.chars().next().unwrap(), r, color, ppp)
            {
                return;
            }
            let real_face = (!st.bold || style.real_bold) && (!st.italic || style.real_italic);
            let fam = if real_face {
                family(st.bold, st.italic)
            } else {
                family(st.bold && style.real_bold, st.italic && style.real_italic)
            };
            let font = FontId::new(style.font_size, fam);
            let mut job = egui::text::LayoutJob::default();
            job.append(
                text,
                0.0,
                egui::TextFormat {
                    font_id: font,
                    color,
                    italics: st.italic && !style.real_italic,
                    line_height: Some(cell.y),
                    valign: egui::Align::Center,
                    ..Default::default()
                },
            );
            job.wrap = egui::text::TextWrapping::no_max_width();
            let galley = painter.layout_job(job);
            let pos = pos2(r.left(), r.top());
            if st.bold && !style.real_bold {
                painter.galley(pos + vec2(0.5 / ppp.max(1.0), 0.0), galley.clone(), color);
            }
            painter.galley(pos, galley, color);
        };
    let decorate = |row: usize, col: usize, width: usize, st: &RunStyle| {
        let r = cell_rect(col, row, width);
        let thickness = (1.0 / ppp).max(0.5);
        let ucolor = st.underline_color.unwrap_or(st.fg);
        if st.underline > 0 {
            let y = snap(r.bottom() - thickness * 1.5);
            painter.hline(r.x_range(), y, Stroke::new(thickness.max(1.0), ucolor));
            if st.underline == 2 {
                painter.hline(
                    r.x_range(),
                    y - 2.0,
                    Stroke::new(thickness.max(1.0), ucolor),
                );
            }
        }
        if st.strike {
            painter.hline(
                r.x_range(),
                snap(r.center().y),
                Stroke::new(thickness.max(1.0), st.fg),
            );
        }
    };

    for (row, run) in &runs {
        paint_text(
            *row,
            run.col,
            &run.text,
            &run.style,
            run.text.len(),
            run.style.fg,
        );
        decorate(*row, run.col, run.text.len(), &run.style);
    }
    for (row, col, text, st, width) in &singles {
        paint_text(*row, *col, text, st, *width, st.fg);
        decorate(*row, *col, *width, st);
    }
    for &(row, s, e) in &link_hover {
        if let Some(slot) = slot_of(row) {
            let r = cell_rect(s, slot, e - s);
            let thickness = (1.0 / ppp).max(1.0);
            painter.hline(r.x_range(), snap(r.bottom() - thickness * 1.5), Stroke::new(thickness, scheme.cursor));
        }
    }

    // Cursor.
    // Blinking stops a while after the last key (the cursor stays solid), so an idle window
    // does not repaint twice a second forever.
    let since_input = ui.input(|i| i.time) - view.blink_epoch;
    let blink_on = if style.cursor_blink && has_focus_now && alive && since_input < BLINK_SECS {
        let t = since_input;
        let phase = (t / 0.53).floor() as i64;
        let next = (phase + 1) as f64 * 0.53 - t;
        ui.ctx()
            .request_repaint_after(Duration::from_secs_f64(next.max(0.01)));
        phase % 2 == 0
    } else {
        true
    };
    let cursor_color = colors[NamedColor::Cursor as usize]
        .map(rgb)
        .unwrap_or(scheme.cursor);
    let mut cursor_rect = None;
    if let Some((row, col, text, fg, bg, wide)) = cursor_cell {
        let r = cell_rect(col, row, if wide { 2 } else { 1 });
        cursor_rect = Some(r);
        if !view.preedit.is_empty() {
            // Drawn below.
        } else if !has_focus_now {
            painter.rect_stroke(
                r,
                CornerRadius::ZERO,
                Stroke::new(1.0, cursor_color),
                StrokeKind::Inside,
            );
        } else if blink_on {
            match cursor.shape {
                CursorShape::Beam => {
                    painter.rect_filled(
                        Rect::from_min_size(r.min, vec2(2.0f32.max(1.0 / ppp), r.height())),
                        CornerRadius::ZERO,
                        cursor_color,
                    );
                }
                CursorShape::Underline => {
                    painter.rect_filled(
                        Rect::from_min_max(pos2(r.left(), r.bottom() - 2.0), r.max),
                        CornerRadius::ZERO,
                        cursor_color,
                    );
                }
                CursorShape::HollowBlock => {
                    painter.rect_stroke(
                        r,
                        CornerRadius::ZERO,
                        Stroke::new(1.0, cursor_color),
                        StrokeKind::Inside,
                    );
                }
                _ => {
                    painter.rect_filled(r, CornerRadius::ZERO, cursor_color);
                    let text_color = if bg == cursor_color { fg } else { bg };
                    if text.trim() != "" {
                        let st = RunStyle {
                            fg: text_color,
                            bold: false,
                            italic: false,
                            underline: 0,
                            underline_color: None,
                            strike: false,
                        };
                        paint_text(row, col, &text, &st, if wide { 2 } else { 1 }, text_color);
                    }
                }
            }
        }
    }
    drop(term);
    if crop.cut_above {
        let first = origin.y + cell.y;
        fade(&ui.painter_at(rect), Rect::from_min_max(rect.min, pos2(rect.max.x, first)), default_bg, false);
    }
    if fixed.is_some_and(|s| s.cols as f32 * cell.x > rect.width() - PAD - 1.0) {
        let edge = Rect::from_min_max(pos2(rect.max.x - 16.0, rect.min.y), rect.max);
        fade(&ui.painter_at(rect), edge, default_bg, true);
    }

    // IME pre-edit text and candidate window position.
    if focused {
        let anchor = cursor_rect.unwrap_or_else(|| Rect::from_min_size(origin, cell));
        if !view.preedit.is_empty() {
            let font = FontId::new(style.font_size, family(false, false));
            let galley = painter.layout_no_wrap(view.preedit.clone(), font, scheme.foreground);
            let r = Rect::from_min_size(anchor.min, vec2(galley.size().x.max(cell.x), cell.y));
            painter.rect_filled(r, CornerRadius::ZERO, default_bg);
            painter.galley(anchor.min, galley, scheme.foreground);
            painter.hline(
                r.x_range(),
                r.bottom() - 1.0,
                Stroke::new(1.0, scheme.foreground),
            );
        }
        let to_global = ui
            .ctx()
            .layer_transform_to_global(ui.layer_id())
            .unwrap_or_default();
        ui.output_mut(|o| {
            o.ime = Some(egui::output::IMEOutput {
                purpose: egui::IMEPurpose::Terminal,
                rect: to_global * anchor,
                cursor_rect: to_global * anchor,
                should_interrupt_composition: false,
            })
        });
    }

    // Context menu.
    // In mouse-reporting mode the right button belongs to the program: Shift+right-click
    // opens the menu, which then stays open after Shift is released.
    if mouse_mode && !shift_held && !view.menu_open {
        return out;
    }
    let mut paste = false;
    let menu_link = view.menu_link.clone();
    let mut open_link = None;
    let menu = response.context_menu(|ui| {
        if let Some(link) = &menu_link {
            if ui.button(crate::tr("Open Link")).clicked() {
                open_link = Some(link.target.clone());
                ui.close();
            }
            if ui.button(crate::tr("Copy Link")).clicked() {
                ui.ctx().copy_text(link.target.clone());
                ui.close();
            }
            ui.separator();
        }
        let term = shared.term.lock();
        let has_sel = term.selection.as_ref().is_some_and(|s| !s.is_empty());
        drop(term);
        if ui
            .add_enabled(has_sel, egui::Button::new(crate::tr("Copy")))
            .clicked()
        {
            if let Some(text) = shared.term.lock().selection_to_string() {
                ui.ctx().copy_text(text);
            }
            ui.close();
        }
        if ui
            .add_enabled(alive, egui::Button::new(crate::tr("Paste")))
            .clicked()
        {
            paste = true;
            ui.memory_mut(|m| m.request_focus(id));
            ui.close();
        }
        if ui.button(crate::tr("Select All")).clicked() {
            select_all(shared);
            ui.close();
        }
        ui.separator();
        if ui.button(crate::tr("Clear Scrollback")).clicked() {
            clear_scrollback(shared);
            ui.close();
        }
    });
    view.menu_open = menu.is_some();
    if menu.is_none() {
        view.menu_link = None;
    }
    if open_link.is_some() {
        out.link = open_link;
    }
    if paste {
        view.request_paste(ui.ctx());
    }
    out
}

fn on_terminal_layer(ui: &Ui, pos: Pos2) -> bool {
    ui.ctx()
        .layer_id_at(pos)
        .is_none_or(|layer| layer == ui.layer_id())
}

/// `text` as a paste into this terminal (bracketed when the program asked for that).
pub fn paste_bytes(shared: &TermShared, text: &str) -> Vec<u8> {
    let bracketed = shared.term.lock().mode().contains(TermMode::BRACKETED_PASTE);
    input::encode_paste(text, bracketed)
}

pub fn select_all(shared: &TermShared) {
    let mut term = shared.term.lock();
    let start = Point::new(term.topmost_line(), Column(0));
    let end = Point::new(term.bottommost_line(), term.last_column());
    let mut sel = Selection::new(SelectionType::Simple, start, Side::Left);
    sel.update(end, Side::Right);
    term.selection = Some(sel);
}

/// Clears the scrollback. While a full-screen program has the alternate screen, the history
/// belongs to the normal screen: it is cleared as soon as the program leaves.
pub fn clear_scrollback(shared: &TermShared) {
    let mut term = shared.term.lock();
    if term.mode().contains(TermMode::ALT_SCREEN) {
        shared.notes.lock().clear_history_pending = true;
    } else {
        shared.prompt.lock().dropped(term.grid().history_size());
        term.grid_mut().clear_history();
    }
    term.selection = None;
}

pub fn copy_selection(ctx: &egui::Context, shared: &TermShared) -> bool {
    match shared.term.lock().selection_to_string() {
        Some(text) => {
            ctx.copy_text(text);
            true
        }
        None => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::{Feeder, new_terminal, term_config};
    use crate::scheme::SCHEMES;

    #[test]
    fn links_are_paths_files_and_web_addresses() {
        for w in ["src/app.rs:120", "./run.sh", "Cargo.toml", "https://example.com/a", "(src/lib.rs:4:2)", "~/notes.md"] {
            assert!(looks_like_link(w), "{w}");
        }
        for w in ["hello", "1.5", "...", "/", "https://", "v0.1.5:", "--force", "a."] {
            assert!(!looks_like_link(w), "{w}");
        }
    }

    fn term_with(cols: usize, rows: usize, bytes: &[u8]) -> crate::core::SharedTerm {
        let (shared, rx) = new_terminal(term_config(100, Default::default(), false), GridSize { cols, rows }, (&SCHEMES[0]).into());
        Feeder::new(shared.clone(), rx).feed(bytes);
        shared
    }

    fn link(shared: &crate::core::SharedTerm, col: usize, row: usize) -> Option<Link> {
        let term = shared.term.lock();
        link_at(&term, 0, col, row)
    }

    #[test]
    fn a_web_address_is_cut_from_the_punctuation_around_it() {
        let span = |s: &str| {
            let chars: Vec<char> = s.chars().collect();
            url_span(&chars).map(|(a, b)| chars[a..b].iter().collect::<String>())
        };
        assert_eq!(span("(https://x.com/a)."), Some("https://x.com/a".into()));
        assert_eq!(span("<https://x.com/a?b=1&c=2>,"), Some("https://x.com/a?b=1&c=2".into()));
        assert_eq!(span("[docs](https://x.com/d)"), Some("https://x.com/d".into()));
        assert_eq!(span("https://en.wikipedia.org/wiki/Rust_(language)"), Some("https://en.wikipedia.org/wiki/Rust_(language)".into()));
        assert_eq!(span("\"HTTP://Example.com/\"!"), Some("HTTP://Example.com/".into()));
        assert_eq!(span("https://"), None);
        assert_eq!(span("https://.../"), None);
        assert_eq!(span("ftp://x.com"), None);
    }

    #[test]
    fn links_under_the_pointer() {
        // Claude Code prints a markdown link as its text and the address in brackets.
        let t = term_with(40, 4, b"see (https://x.com/a). ok");
        let l = link(&t, 8, 0).unwrap();
        assert_eq!((l.target.as_str(), l.kind, l.cells.clone()), ("https://x.com/a", LinkKind::Web, vec![(0, 5, 20)]));
        assert!(!l.hidden());
        // Not on the address: the bracket, or the word after it.
        assert_eq!(link(&t, 4, 0), None);
        assert_eq!(link(&t, 24, 0), None);
        // A path stays a path for the app to look up.
        let t = term_with(40, 4, b"error at (src/app.rs:120), see");
        let l = link(&t, 12, 0).unwrap();
        assert_eq!((l.target.as_str(), l.kind), ("src/app.rs:120", LinkKind::Path));
    }

    #[test]
    fn an_address_wrapped_by_the_terminal_is_one_link() {
        let t = term_with(20, 4, b"https://example.com/abcdefgh");
        for (col, row) in [(3, 0), (2, 1)] {
            let l = link(&t, col, row).unwrap();
            assert_eq!(l.target, "https://example.com/abcdefgh");
            assert_eq!(l.cells, vec![(0, 0, 20), (1, 0, 8)]);
            assert!(l.hidden());
        }
    }

    #[test]
    fn an_address_a_program_broke_at_the_edge_is_one_link() {
        // Ink breaks a long word at its width and indents the rest (24 columns here).
        let t = term_with(24, 4, b"> https://example.com/a1\r\n  b2/c3?d=4 next\r\n");
        for (col, row) in [(5, 0), (4, 1)] {
            let l = link(&t, col, row).unwrap();
            assert_eq!(l.target, "https://example.com/a1b2/c3?d=4", "at {col},{row}");
            assert_eq!(l.cells, vec![(0, 2, 24), (1, 2, 11)]);
        }
        // An address that ends at the edge by chance, followed by a word: not joined.
        let t = term_with(24, 4, b"> https://example.com/a1\r\n  Next.\r\n");
        assert_eq!(link(&t, 5, 0).unwrap().target, "https://example.com/a1");
        assert_eq!(link(&t, 3, 1), None);
        // Not at the edge: the next row is other text.
        let t = term_with(40, 4, b"> see https://x.com/a\r\n  b2/c3\r\n");
        assert_eq!(link(&t, 8, 0).unwrap().target, "https://x.com/a");
    }

    #[test]
    fn an_osc8_hyperlink_opens_its_address() {
        let t = term_with(40, 4, b"\x1b]8;;https://h.com/pr/12\x1b\\PR #12\x1b]8;;\x1b\\ merged");
        let l = link(&t, 1, 0).unwrap();
        assert_eq!((l.target.as_str(), l.kind, l.cells.clone()), ("https://h.com/pr/12", LinkKind::Hyperlink, vec![(0, 0, 6)]));
        assert!(l.hidden());
        assert_eq!(link(&t, 9, 0), None);
    }

    #[test]
    fn dim_darkens_every_indexed_colour() {
        let (colors, scheme) = (Colors::default(), &SCHEMES[0]);
        let dim = |i: u8| resolve_fg(Color::Indexed(i), Flags::DIM, &colors, scheme);
        let plain = |i: u8| resolve_fg(Color::Indexed(i), Flags::empty(), &colors, scheme);
        assert_eq!(dim(1), palette(NamedColor::DimRed as usize, &colors, scheme));
        for i in [17, 100, 196, 245] {
            assert_eq!(dim(i), mul(plain(i), DIM_FACTOR), "colour {i}");
            assert_ne!(dim(i), plain(i), "colour {i}");
        }
    }

    /// The menu's Paste pastes into its pane, bracketed when the program asked, on every
    /// platform; on Linux a paste event it did not ask for stays a typed Ctrl+V.
    #[test]
    fn the_menus_paste_pastes_into_its_pane() {
        let ctx = egui::Context::default();
        let (shared, rx) = new_terminal(term_config(100, Default::default(), false), GridSize { cols: 40, rows: 10 }, (&SCHEMES[0]).into());
        Feeder::new(shared.clone(), rx).feed(b"\x1b[?2004h");
        let style = TermStyle {
            font_size: 13.0,
            cell: vec2(8.0, 16.0),
            scheme: &SCHEMES[0],
            real_bold: false,
            real_italic: false,
            option_as_meta: false,
            copy_on_select: false,
            cursor_blink: false,
        };
        let rect = Rect::from_min_size(Pos2::ZERO, vec2(400.0, 200.0));
        let mut view = TermView::new(pane_widget_id(1));
        let frame = |view: &mut TermView, events: Vec<Event>| {
            let raw = egui::RawInput { screen_rect: Some(rect), focused: true, events, ..Default::default() };
            let mut input = Vec::new();
            let mut out = ctx.run_ui(raw, |ui| input = show(ui, rect, view, &shared, &style, true, true, true).input);
            out.textures_delta.clear();
            input
        };
        // The pane takes focus, and reads keys from the frame after.
        frame(&mut view, vec![]);
        frame(&mut view, vec![]);
        view.request_paste(&ctx);
        assert_eq!(frame(&mut view, vec![Event::Paste("a\nb".into())]), b"\x1b[200~a\rb\x1b[201~");
        let plain = frame(&mut view, vec![Event::Paste("x".into())]);
        if cfg!(target_os = "linux") {
            assert_eq!(plain, [0x16]);
        } else {
            assert_eq!(plain, b"\x1b[200~x\x1b[201~");
        }
    }

    /// The paste shortcut with only an image on the clipboard brings no paste event, only the
    /// key's release: Ctrl+V for the app to give an agent (typed directly on Linux, where Ctrl+V
    /// is the program's). After a paste of text nothing more is sent.
    #[test]
    fn a_paste_without_text_hands_over_ctrl_v() {
        let ctx = egui::Context::default();
        let (shared, rx) = new_terminal(term_config(100, Default::default(), false), GridSize { cols: 40, rows: 10 }, (&SCHEMES[0]).into());
        drop(rx);
        let style = TermStyle {
            font_size: 13.0,
            cell: vec2(8.0, 16.0),
            scheme: &SCHEMES[0],
            real_bold: false,
            real_italic: false,
            option_as_meta: false,
            copy_on_select: false,
            cursor_blink: false,
        };
        let rect = Rect::from_min_size(Pos2::ZERO, vec2(400.0, 200.0));
        let mut view = TermView::new(pane_widget_id(1));
        let mut frame = |events: Vec<Event>| {
            let raw = egui::RawInput { screen_rect: Some(rect), focused: true, events, ..Default::default() };
            let mut resp = TermResponse::default();
            let mut out = ctx.run_ui(raw, |ui| resp = show(ui, rect, &mut view, &shared, &style, true, true, true));
            out.textures_delta.clear();
            resp
        };
        let cmd = if cfg!(target_os = "macos") { egui::Modifiers::MAC_CMD | egui::Modifiers::COMMAND } else { egui::Modifiers::CTRL | egui::Modifiers::COMMAND };
        let release = Event::Key { key: Key::V, physical_key: Some(Key::V), pressed: false, repeat: false, modifiers: cmd };
        frame(vec![]);
        frame(vec![]);
        let r = frame(vec![release.clone()]);
        if cfg!(target_os = "linux") {
            assert_eq!((r.input, r.paste_without_text), (vec![0x16], None));
        } else {
            assert_eq!((r.input, r.paste_without_text), (vec![], Some(vec![0x16])));
        }
        let r = frame(vec![Event::Paste("x".into()), release]);
        assert_eq!(r.paste_without_text, None);
        assert!(!r.input.contains(&0x16) || cfg!(target_os = "linux"));
    }

    /// Keys go out as the program asked: in the kitty protocol a key carries the text it typed
    /// (which is not typed again), plain text stays text, and releases are sent when asked for.
    #[test]
    fn keys_follow_the_keyboard_protocol() {
        let ctx = egui::Context::default();
        let (shared, rx) = new_terminal(term_config(100, Default::default(), false), GridSize { cols: 40, rows: 10 }, (&SCHEMES[0]).into());
        let mut feeder = Feeder::new(shared.clone(), rx);
        let style = TermStyle {
            font_size: 13.0,
            cell: vec2(8.0, 16.0),
            scheme: &SCHEMES[0],
            real_bold: false,
            real_italic: false,
            option_as_meta: true,
            copy_on_select: false,
            cursor_blink: false,
        };
        let rect = Rect::from_min_size(Pos2::ZERO, vec2(400.0, 200.0));
        let mut view = TermView::new(pane_widget_id(1));
        let mut frame = |mut events: Vec<Event>| {
            let modifiers = match events.first() {
                Some(Event::Key { modifiers, .. }) => *modifiers,
                _ => egui::Modifiers::NONE,
            };
            events.insert(0, Event::ModifiersChanged(modifiers));
            let raw = egui::RawInput { screen_rect: Some(rect), focused: true, events, ..Default::default() };
            let mut input = Vec::new();
            let mut out = ctx.run_ui(raw, |ui| input = show(ui, rect, &mut view, &shared, &style, true, true, true).input);
            out.textures_delta.clear();
            input
        };
        let key = |key: Key, modifiers: egui::Modifiers, pressed: bool| Event::Key {
            key,
            physical_key: Some(key),
            pressed,
            repeat: false,
            modifiers,
        };
        let (none, shift, alt) = (egui::Modifiers::NONE, egui::Modifiers::SHIFT, egui::Modifiers::ALT);
        // A key and its release, with the text it typed.
        let typed = |k: Key, m: egui::Modifiers, text: Option<&str>| {
            let mut events = vec![key(k, m, true)];
            events.extend(text.map(|t| Event::Text(t.into())));
            events.push(key(k, m, false));
            events
        };
        frame(vec![]);
        frame(vec![]);
        // Legacy: Shift+Enter is Enter.
        assert_eq!(frame(typed(Key::Enter, shift, None)), b"\r");
        assert_eq!(frame(typed(Key::A, alt, Some("a"))), b"\x1ba");
        feeder.feed(b"\x1b[>1u");
        assert_eq!(frame(typed(Key::Enter, shift, None)), b"\x1b[13;2u");
        assert_eq!(frame(typed(Key::A, alt, Some("a"))), b"\x1b[97;3u");
        assert_eq!(frame(typed(Key::A, shift, Some("A"))), b"A");
        // Every key as an escape code, releases too.
        feeder.feed(b"\x1b[=10u");
        assert_eq!(frame(typed(Key::A, none, Some("a"))), b"\x1b[97u\x1b[97;1:3u");
        // Text without a key event (an input method) is typed as it is.
        assert_eq!(frame(vec![Event::Text("é".into())]), "é".as_bytes());
    }

    /// A screen of `rows` rows with text on the listed ones.
    fn screen(rows: usize, text: &[usize]) -> Vec<bool> {
        (0..rows).map(|r| !text.contains(&r)).collect()
    }

    #[test]
    fn a_cut_view_narrows_empty_space_before_cutting_text() {
        // Claude Code 2.1: header on rows 0–3, empty space, its input pinned to the bottom
        // (effort line 34, box 35–37 with the cursor on 36, status line 38).
        let claude = screen(40, &[0, 1, 2, 3, 34, 35, 36, 37, 38]);
        let (rows, cut) = rows_shown(&claude, 20, 36, 38);
        assert!(!cut);
        assert_eq!(rows.len(), 20);
        // Header and input both in view, the gap between them narrowed to 11 rows.
        assert_eq!(&rows[..4], &[0, 1, 2, 3]);
        assert_eq!(&rows[15..], &[34, 35, 36, 37, 38]);
        // Very little room: the gap keeps one row.
        let (rows, cut) = rows_shown(&claude, 10, 36, 38);
        assert_eq!((rows, cut), (vec![0, 1, 2, 3, 4, 34, 35, 36, 37, 38], false));
        // Less still: cut above, the cursor and the status line kept; the row above the first
        // one shown is listed for the faded top edge.
        let (rows, cut) = rows_shown(&claude, 5, 36, 38);
        assert!(cut);
        assert_eq!(rows, vec![4, 34, 35, 36, 37, 38]);
        // Everything fits: every row, as on the screen.
        assert_eq!(rows_shown(&claude, 40, 36, 38), ((0..40).collect(), false));
    }

    #[test]
    fn a_cut_view_of_a_full_shell_keeps_its_latest_lines() {
        // Output on every row down to the prompt (row 30, cursor), nothing below.
        let text: Vec<usize> = (0..=30).collect();
        let shell = screen(50, &text);
        let (rows, cut) = rows_shown(&shell, 20, 30, 30);
        assert!(cut);
        assert_eq!(rows.first(), Some(&10));
        assert_eq!(rows.last(), Some(&30));
        assert_eq!(rows.len(), 21);
        // A fresh shell: only its prompt, at the top.
        let fresh = screen(50, &[0]);
        assert_eq!(rows_shown(&fresh, 20, 0, 0), (vec![0], false));
    }

    #[test]
    fn a_cut_view_keeps_the_cursor_row() {
        // Everything fits: nothing hidden.
        assert_eq!(rows_to_skip(24, 30, 23, 23), 0);
        // A fresh shell near the top: the top rows.
        assert_eq!(rows_to_skip(50, 20, 3, 3), 0);
        // Output reached row 40: rows 21..=40 shown.
        assert_eq!(rows_to_skip(50, 20, 40, 40), 21);
        // Cursor on the last row (full-screen program's input line).
        assert_eq!(rows_to_skip(50, 20, 49, 49), 30);
    }

    #[test]
    fn a_cut_view_keeps_what_is_drawn_below_the_cursor() {
        // An agent's input box (cursor on row 40) with its border and status line down to 42.
        assert_eq!(rows_to_skip(50, 20, 40, 42), 23);
        // Short content below the cursor near the top: still nothing hidden.
        assert_eq!(rows_to_skip(50, 20, 3, 5), 0);
        // A full-screen editor with its cursor near the top and a status line on the last row:
        // the cursor wins, the rest fills the view.
        assert_eq!(rows_to_skip(50, 20, 2, 49), 2);
        // Text below the cursor reaching beyond one view: the cursor stays on the first row.
        assert_eq!(rows_to_skip(50, 20, 10, 45), 10);
    }
}
