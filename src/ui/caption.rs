//! Windows: the window buttons at the right of the top bar and the resize edges, for a window
//! without the system caption (`platform::custom_caption`). They look and act as Windows 11's.

use eframe::egui::{self, Color32, CursorIcon, Pos2, Rect, ResizeDirection, Sense, Stroke, Ui, ViewportCommand, pos2, vec2};

use super::style;
use crate::i18n::t;

/// One window button: as wide as Windows makes them.
const BUTTON_W: f32 = 46.0;
/// The three buttons, at the right end of the top bar.
pub const CAPTION_W: f32 = 3.0 * BUTTON_W;
/// How far in from the window's border a press resizes it.
const EDGE: f32 = 4.0;
/// How far along an edge from a corner the press resizes both ways.
const CORNER: f32 = 12.0;
/// Close's red (Windows 11), hovered.
const CLOSE_RED: Color32 = Color32::from_rgb(0xC4, 0x2B, 0x1C);

#[derive(Clone, Copy, PartialEq)]
enum Button {
    Minimize,
    Maximize,
    Close,
}

/// Minimise, maximise (restore when maximised) and close, from the right end of `bar`. Glyphs
/// are fainter while the window is not the active one.
pub fn caption_buttons(ui: &mut Ui, bar: Rect, focused: bool) {
    let p = style::pal(ui);
    let dark = ui.visuals().dark_mode;
    let maximized = ui.ctx().input(|i| i.viewport().maximized.unwrap_or(false));
    let glyph = if focused { p.text } else { p.text_faint };
    // White or black over the bar: 6 % / 3.7 % hovered, 4 % / 2.4 % pressed (Windows 11).
    let wash = |hover: bool| match (dark, hover) {
        (true, true) => Color32::from_white_alpha(15),
        (true, false) => Color32::from_white_alpha(10),
        (false, true) => Color32::from_black_alpha(9),
        (false, false) => Color32::from_black_alpha(6),
    };
    // The bar's bottom line stays drawn under them.
    let h = bar.height() - 1.0;
    for (i, button) in [Button::Close, Button::Maximize, Button::Minimize].into_iter().enumerate() {
        let rect = Rect::from_min_size(pos2(bar.right() - BUTTON_W * (i as f32 + 1.0), bar.top()), vec2(BUTTON_W, h));
        let tip = match button {
            Button::Minimize => t("Minimize"),
            Button::Maximize if maximized => t("Restore Down"),
            Button::Maximize => t("Maximize"),
            Button::Close => t("Close"),
        };
        let r = ui.interact(rect, egui::Id::new(("window-button", i)), Sense::click()).on_hover_text(tip);
        #[cfg(debug_assertions)]
        crate::devtest::record(["window-close", "window-maximize", "window-minimize"][i], rect);
        let down = r.is_pointer_button_down_on();
        let (fill, color) = match button {
            Button::Close if down => (CLOSE_RED.gamma_multiply(0.9), Color32::WHITE),
            Button::Close if r.hovered() => (CLOSE_RED, Color32::WHITE),
            _ if down => (wash(false), glyph),
            _ if r.hovered() => (wash(true), glyph),
            _ => (Color32::TRANSPARENT, glyph),
        };
        let painter = ui.painter();
        if fill != Color32::TRANSPARENT {
            painter.rect_filled(rect, 0.0, fill);
        }
        draw_glyph(ui, button, maximized, rect.center(), color);
        if r.clicked() {
            let command = match button {
                Button::Minimize => ViewportCommand::Minimized(true),
                Button::Maximize => ViewportCommand::Maximized(!maximized),
                // As the system's ×: the close request is handled like any other.
                Button::Close => ViewportCommand::Close,
            };
            ui.ctx().send_viewport_cmd(command);
        }
    }
}

/// A button's glyph, 10 points wide, in lines one whole pixel thick on pixel centres.
fn draw_glyph(ui: &Ui, button: Button, maximized: bool, centre: Pos2, color: Color32) {
    let painter = ui.painter();
    let ppp = ui.ctx().pixels_per_point();
    let stroke = Stroke::new(ppp.round().max(1.0) / ppp, color);
    let snap = |p: Pos2| pos2(painter.round_to_pixel_center(p.x), painter.round_to_pixel_center(p.y));
    let c = snap(centre);
    let line = |a: Pos2, b: Pos2| {
        painter.line_segment([snap(a), snap(b)], stroke);
    };
    let square = |min: Pos2, size: f32| {
        let (a, b) = (snap(min), snap(min + vec2(size, size)));
        painter.line_segment([a, pos2(b.x, a.y)], stroke);
        painter.line_segment([pos2(b.x, a.y), b], stroke);
        painter.line_segment([b, pos2(a.x, b.y)], stroke);
        painter.line_segment([pos2(a.x, b.y), a], stroke);
    };
    match button {
        Button::Minimize => line(c + vec2(-5.0, 0.0), c + vec2(5.0, 0.0)),
        Button::Maximize if maximized => {
            // Restore: a square, and the top and right edges of one behind it.
            square(c + vec2(-5.0, -3.0), 8.0);
            line(c + vec2(-3.0, -5.0), c + vec2(5.0, -5.0));
            line(c + vec2(5.0, -5.0), c + vec2(5.0, 3.0));
        }
        Button::Maximize => square(c + vec2(-5.0, -5.0), 10.0),
        Button::Close => {
            line(c + vec2(-5.0, -5.0), c + vec2(5.0, 5.0));
            line(c + vec2(-5.0, 5.0), c + vec2(5.0, -5.0));
        }
    }
}

/// The window's edges and corners resize it (with the system caption gone, Windows gives the
/// window none): thin areas above everything, none while it is maximised or full screen.
pub fn resize_edges(ctx: &egui::Context) {
    let whole = ctx.input(|i| i.viewport().maximized.unwrap_or(false) || i.viewport().fullscreen.unwrap_or(false));
    if whole {
        return;
    }
    let w = ctx.content_rect();
    let (l, r, top, b) = (w.left(), w.right(), w.top(), w.bottom());
    use ResizeDirection as D;
    let zones = [
        (D::North, Rect::from_min_max(pos2(l + CORNER, top), pos2(r - CORNER, top + EDGE))),
        (D::South, Rect::from_min_max(pos2(l + CORNER, b - EDGE), pos2(r - CORNER, b))),
        (D::West, Rect::from_min_max(pos2(l, top + CORNER), pos2(l + EDGE, b - CORNER))),
        (D::East, Rect::from_min_max(pos2(r - EDGE, top + CORNER), pos2(r, b - CORNER))),
        // Each corner: the ends of the two edges that meet there.
        (D::NorthWest, Rect::from_min_max(pos2(l, top), pos2(l + CORNER, top + EDGE))),
        (D::NorthWest, Rect::from_min_max(pos2(l, top), pos2(l + EDGE, top + CORNER))),
        (D::NorthEast, Rect::from_min_max(pos2(r - CORNER, top), pos2(r, top + EDGE))),
        (D::NorthEast, Rect::from_min_max(pos2(r - EDGE, top), pos2(r, top + CORNER))),
        (D::SouthWest, Rect::from_min_max(pos2(l, b - EDGE), pos2(l + CORNER, b))),
        (D::SouthWest, Rect::from_min_max(pos2(l, b - CORNER), pos2(l + EDGE, b))),
        (D::SouthEast, Rect::from_min_max(pos2(r - CORNER, b - EDGE), pos2(r, b))),
        (D::SouthEast, Rect::from_min_max(pos2(r - EDGE, b - CORNER), pos2(r, b))),
    ];
    for (i, (dir, rect)) in zones.into_iter().enumerate() {
        egui::Area::new(egui::Id::new(("resize-edge", i)))
            .order(egui::Order::Foreground)
            .fixed_pos(rect.min)
            .show(ctx, |ui| {
                let (_, resp) = ui.allocate_exact_size(rect.size(), Sense::drag());
                if resp.hovered() || resp.dragged() {
                    ctx.set_cursor_icon(cursor(dir));
                }
                if resp.drag_started() {
                    ctx.send_viewport_cmd(ViewportCommand::BeginResize(dir));
                }
            });
    }
}

fn cursor(dir: ResizeDirection) -> CursorIcon {
    match dir {
        ResizeDirection::North => CursorIcon::ResizeNorth,
        ResizeDirection::South => CursorIcon::ResizeSouth,
        ResizeDirection::East => CursorIcon::ResizeEast,
        ResizeDirection::West => CursorIcon::ResizeWest,
        ResizeDirection::NorthEast => CursorIcon::ResizeNorthEast,
        ResizeDirection::SouthEast => CursorIcon::ResizeSouthEast,
        ResizeDirection::NorthWest => CursorIcon::ResizeNorthWest,
        ResizeDirection::SouthWest => CursorIcon::ResizeSouthWest,
    }
}
