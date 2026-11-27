//! Windows: the title bar the app draws for a window without the system caption
//! (`platform::custom_caption`): the app's mark and name, its menus (those of the macOS menu
//! bar), the window buttons as Windows 11 draws them, and the resize edges.

use eframe::egui::{self, Color32, CursorIcon, Pos2, Rect, ResizeDirection, Sense, Stroke, Ui, ViewportCommand, pos2, vec2};

use super::{kit, style};
use crate::actions::Action;
use crate::app::App;
use crate::i18n::t;
use tc_core::model::View;

/// The title bar's height.
pub const TITLE_H: f32 = 34.0;
/// One window button: as wide as Windows makes them.
const BUTTON_W: f32 = 46.0;
/// The three buttons, at the right end of the title bar.
const CAPTION_W: f32 = 3.0 * BUTTON_W;
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

impl App {
    /// The title bar: mark and name at the left, then the menus; empty space moves the window;
    /// the window buttons at the right.
    pub fn title_bar(&mut self, ui: &mut Ui) {
        let p = style::pal(ui);
        let bar = ui.max_rect();
        ui.painter().hline(bar.x_range(), bar.bottom() - 0.5, Stroke::new(1.0, p.border));
        super::view::title_drag_area(ui, bar, "title-bar");
        let caption = Rect::from_min_max(pos2(bar.right() - CAPTION_W, bar.top()), bar.max);
        caption_buttons(ui, caption, self.window_focused);

        let text = if self.window_focused { p.text_muted } else { p.text_faint };
        let mark = Rect::from_center_size(pos2(bar.left() + 12.0 + 8.0, bar.center().y), vec2(16.0, 16.0));
        super::settings::paint_logo(ui.painter(), mark, &p);
        let name = ui.painter().layout_no_wrap("TernaryCode".to_string(), style::medium(style::SMALL), text);
        let name_at = pos2(mark.right() + 8.0, bar.center().y - name.size().y / 2.0);
        let name_w = name.size().x;
        ui.painter().galley(name_at, name, text);

        let menus_min_x = name_at.x + name_w + 14.0;
        let menus = Rect::from_min_max(pos2(menus_min_x, bar.top()), pos2((caption.left() - 8.0).max(menus_min_x), bar.bottom()));
        #[cfg(debug_assertions)]
        crate::devtest::record("title-menus", menus);
        let chosen = ui
            .scope_builder(egui::UiBuilder::new().max_rect(menus).layout(egui::Layout::left_to_right(egui::Align::Center)), |ui| {
                ui.set_clip_rect(menus.intersect(ui.clip_rect()));
                // The menus' names: quiet, 26 high, rounded 5; the open one pressed.
                let st = ui.style_mut();
                st.spacing.button_padding = vec2(9.0, 5.0);
                st.spacing.item_spacing.x = 2.0;
                for (w, fill) in [
                    (&mut st.visuals.widgets.inactive, Color32::TRANSPARENT),
                    (&mut st.visuals.widgets.hovered, p.hover),
                    (&mut st.visuals.widgets.active, p.subtle),
                    (&mut st.visuals.widgets.open, p.subtle),
                ] {
                    w.weak_bg_fill = fill;
                    w.bg_fill = fill;
                    w.bg_stroke = Stroke::NONE;
                    w.corner_radius = egui::CornerRadius::same(5);
                    w.fg_stroke = Stroke::new(1.0, if fill == Color32::TRANSPARENT { p.text_muted } else { p.text });
                    w.expansion = 0.0;
                }
                egui::MenuBar::new().ui(ui, |ui| self.title_menus(ui)).inner
            })
            .inner;
        if let Some(action) = chosen {
            self.actions.push(action);
        }
    }

    /// The menus, as the macOS menu bar has them (`menu.rs`), the Windows way: Settings and
    /// Exit under File, updates and About under Help. The action of the row clicked.
    fn title_menus(&mut self, ui: &mut Ui) -> Option<Action> {
        use Action as A;
        let maximized = ui.ctx().input(|i| i.viewport().maximized.unwrap_or(false));
        let mut chosen = None;
        ui.menu_button(t("File"), |ui| {
            chosen = menu_rows(ui, &[
                Some((t("Add Project…"), A::AddProject)),
                Some((t("Open File…"), A::QuickOpen)),
                Some((t("Save"), A::SaveFile)),
                None,
                Some((t("New Tab"), A::NewTab)),
                Some((t("New Browser Page"), A::NewBrowserTab)),
                Some((t("Reopen Closed Page"), A::ReopenClosedPage)),
                Some((t("Close Pane"), A::ClosePane)),
                None,
                Some((t("Settings…"), A::Settings)),
                Some((t("Agent Integrations…"), A::Integrations)),
                None,
                Some((t("Exit"), A::Quit)),
            ]);
        });
        ui.menu_button(t("Edit"), |ui| {
            chosen = menu_rows(ui, &[
                Some((t("Copy"), A::Copy)),
                Some((t("Paste"), A::Paste)),
                Some((t("Select All"), A::SelectAll)),
                None,
                Some((t("Find in Pane…"), A::Find)),
                Some((t("Find in All Panes…"), A::FindAll)),
                None,
                Some((t("Clear Scrollback"), A::ClearScrollback)),
            ]);
        });
        ui.menu_button(t("View"), |ui| {
            chosen = menu_rows(ui, &[
                Some((t("Command Palette…"), A::Palette)),
                Some((t("Switch Project…"), A::SwitchProject)),
                Some((t("Move Project Up"), A::MoveActiveProject(true))),
                Some((t("Move Project Down"), A::MoveActiveProject(false))),
                Some((t("Show/Hide Sidebar"), A::ToggleSidebar)),
                Some((t("Narrow Sidebar"), A::ToggleCompactSidebar)),
                Some((t("Zen Mode"), A::ToggleZenMode)),
                Some((t("Files"), A::ToggleEditor)),
                Some((t("Show/Hide Panel"), A::ToggleDrawer)),
                Some((t("Expand Panel"), A::TogglePanelExpanded)),
                Some((t("Show/Hide File Tree"), A::ToggleFiles)),
                None,
                Some((t("Browser"), A::ShowBrowser)),
                Some((t("Open Address"), A::BrowserAddress)),
                Some((t("Reload Page"), A::BrowserReload)),
                None,
                Some((t("Focus View"), A::SetView(View::Focus))),
                Some((t("Overview"), A::ToggleGrid)),
                Some((t("Accounts & Usage…"), A::OpenProfiles)),
                Some((t("Phone…"), A::OpenPhonePairing)),
                None,
                Some((t("Split Right"), A::SplitRight)),
                Some((t("Split Down"), A::SplitDown)),
                Some((t("Next Pane"), A::NextPane)),
                Some((t("Previous Pane"), A::PrevPane)),
                None,
                Some((t("Bigger Text"), A::FontBigger)),
                Some((t("Smaller Text"), A::FontSmaller)),
                Some((t("Actual Size"), A::FontReset)),
            ]);
        });
        ui.menu_button(t("Agent"), |ui| {
            chosen = menu_rows(ui, &[
                Some((t("Run Agent…"), A::PickPreset)),
                Some((t("Review Changes…"), A::Review)),
                Some((t("Commit History"), A::History)),
                Some((t("Commit…"), A::Commit)),
                None,
                Some((t("Go to Next Waiting Agent"), A::NextAttention)),
                Some((t("Back to Where I Was"), A::BackAttention)),
                Some((t("Inbox"), A::ToggleAttention)),
                Some((t("Automations"), A::ToggleAutomations)),
            ]);
        });
        ui.menu_button(t("Window"), |ui| {
            menu_body(ui);
            if kit::menu_item(ui, None, t("Minimize"), None, false).clicked() {
                ui.ctx().send_viewport_cmd(ViewportCommand::Minimized(true));
                ui.close();
            }
            let zoom = if maximized { t("Restore Down") } else { t("Maximize") };
            if kit::menu_item(ui, None, zoom, None, false).clicked() {
                ui.ctx().send_viewport_cmd(ViewportCommand::Maximized(!maximized));
                ui.close();
            }
            kit::menu_sep(ui);
            chosen = menu_rows(ui, &[Some((t("Show Next Tab"), A::NextTab)), Some((t("Show Previous Tab"), A::PrevTab))]);
        });
        ui.menu_button(t("Help"), |ui| {
            chosen = menu_rows(ui, &[Some((t("Check for Updates…"), A::CheckForUpdates)), None, Some((t("About TernaryCode"), A::About))]);
        });
        chosen
    }
}

/// Rows of a title bar menu (`None`: a line between groups) with their shortcuts; the action
/// of the one clicked.
fn menu_rows(ui: &mut Ui, rows: &[Option<(&str, Action)>]) -> Option<Action> {
    menu_body(ui);
    let mut chosen = None;
    for row in rows {
        match row {
            Some((label, action)) => {
                if kit::menu_item(ui, None, label, kit::keys(action), false).clicked() {
                    chosen = Some(action.clone());
                    ui.close();
                }
            }
            None => kit::menu_sep(ui),
        }
    }
    chosen
}

/// A title bar menu's width: rows take all there is, and a menu bar's popup has a lot.
fn menu_body(ui: &mut Ui) {
    kit::menu_body(ui);
    ui.set_width(260.0);
}

/// Minimise, maximise (restore when maximised) and close, from the right end of `bar`. Glyphs
/// are fainter while the window is not the active one.
fn caption_buttons(ui: &mut Ui, bar: Rect, focused: bool) {
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
