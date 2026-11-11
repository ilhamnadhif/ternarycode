//! Shared components drawn with the design tokens of `style.rs`: buttons, icon buttons, button
//! groups, tab strips, keyboard hints, empty states and cards. Screens use these instead of
//! styling egui widgets one by one, so every view looks and behaves the same.

use eframe::egui::{
    self, Align2, Color32, CornerRadius, FontId, Rect, Response, RichText, Sense, Stroke,
    StrokeKind, Ui, Vec2, WidgetInfo, WidgetType, vec2,
};

use super::style::{self, RADIUS};

/// The keyboard shortcut of an action, as the menu (macOS) or the shortcut table (elsewhere)
/// has it. One table, so tooltips and hints never disagree with what the keys do.
pub fn keys(action: &crate::actions::Action) -> Option<&'static str> {
    let (mac, other) = key_labels(action)?;
    Some(if cfg!(target_os = "macos") { mac } else { other }).filter(|k| !k.is_empty())
}

/// An action's shortcut on macOS and on Windows and Linux ("" where it has none there). The
/// macOS menu takes its key equivalents from here (`menu.rs`); `App::handle_shortcuts` binds
/// the others, and a test checks they agree.
pub fn key_labels(action: &crate::actions::Action) -> Option<(&'static str, &'static str)> {
    use crate::actions::Action as A;
    use crate::lsp::{Goto, IdeCmd};
    const MAC_DIGITS: [&str; 9] = ["⌘1", "⌘2", "⌘3", "⌘4", "⌘5", "⌘6", "⌘7", "⌘8", "⌘9"];
    const DIGITS: [&str; 9] = [
        "Ctrl+Shift+1", "Ctrl+Shift+2", "Ctrl+Shift+3", "Ctrl+Shift+4", "Ctrl+Shift+5",
        "Ctrl+Shift+6", "Ctrl+Shift+7", "Ctrl+Shift+8", "Ctrl+Shift+9",
    ];
    Some(match action {
        A::ToggleSidebar => ("⌘B", "Ctrl+Shift+B"),
        A::ToggleEditor => ("⌘E", "Ctrl+Shift+Y"),
        A::QuickOpen => ("⌘O", "Ctrl+Shift+O"),
        A::AddProject => ("⇧⌘O", "Ctrl+Shift+N"),
        A::PickPreset => ("⇧⌘A", "Ctrl+Shift+A"),
        A::SplitRight => ("⌘D", "Ctrl+Shift+D"),
        A::SplitDown => ("⇧⌘D", "Ctrl+Shift+E"),
        A::NewTab => ("⌘T", "Ctrl+Shift+T"),
        A::ClosePane => ("⌘W", "Ctrl+Shift+W"),
        A::Review => ("⇧⌘R", "Ctrl+Shift+R"),
        A::Palette => ("⌘K", "Ctrl+Shift+K"),
        A::SwitchProject => ("⌘P", "Ctrl+Shift+P"),
        A::NextAttention => ("⌘J", "Ctrl+Shift+J"),
        A::BackAttention => ("⇧⌘J", "Ctrl+Alt+J"),
        A::ToggleAttention => ("⌥⌘J", "Ctrl+Shift+Alt+J"),
        A::ToggleGrid => ("⌘0", "Ctrl+Shift+0"),
        A::OpenGridTile => ("⌘↵", "Ctrl+Shift+Enter"),
        A::SaveFile => ("⌘S", "Ctrl+Shift+S"),
        A::Find => ("⌘F", "Ctrl+Shift+F"),
        A::FindAll => ("⇧⌘F", "Ctrl+Shift+G"),
        A::Settings => ("⌘,", "Ctrl+Shift+,"),
        A::Quit => ("⌘Q", "Ctrl+Shift+Q"),
        A::Copy => ("⌘C", "Ctrl+Shift+C"),
        A::Paste => ("⌘V", "Ctrl+Shift+V"),
        A::SelectAll => ("⌘A", ""),
        A::ClearScrollback => ("⌥⌘K", "Ctrl+Shift+Alt+K"),
        A::NextTab => ("⇧⌘]", "Ctrl+Shift+PageDown"),
        A::PrevTab => ("⇧⌘[", "Ctrl+Shift+PageUp"),
        A::NextPane => ("⌘]", "Ctrl+Shift+]"),
        A::PrevPane => ("⌘[", "Ctrl+Shift+["),
        A::FontBigger => ("⌘=", "Ctrl+Shift+="),
        A::FontSmaller => ("⌘-", "Ctrl+Shift+-"),
        // ⌘0 is the Grid.
        A::FontReset => ("", "Ctrl+Shift+Backspace"),
        A::SelectProjectIndex(i) if *i < 9 => (MAC_DIGITS[*i], DIGITS[*i]),
        // The editor's own, while its text has the keyboard (`App::ide_keys`).
        A::Ide(cmd) => match cmd {
            IdeCmd::Goto(Goto::Definition) => ("F12", "F12"),
            IdeCmd::Goto(Goto::Implementation) => ("⌘F12", "Ctrl+F12"),
            IdeCmd::References => ("⇧F12", "Shift+F12"),
            IdeCmd::Rename => ("F2", "F2"),
            IdeCmd::CodeActions => ("⌘.", "Ctrl+."),
            IdeCmd::Format => ("⇧⌥F", "Shift+Alt+F"),
            IdeCmd::Symbols => ("⌥⌘O", "Ctrl+Alt+O"),
            IdeCmd::WorkspaceSymbols => ("⌥⌘T", "Ctrl+Alt+T"),
            IdeCmd::Problems => ("⇧⌘M", "Ctrl+Shift+M"),
            IdeCmd::Back => ("⌃-", "Ctrl+-"),
            IdeCmd::Forward => ("⌃⇧-", "Ctrl+Shift+-"),
            _ => return None,
        },
        _ => return None,
    })
}

/// A tooltip followed by the action's shortcut.
pub fn tip(text: &str, action: crate::actions::Action) -> String {
    match keys(&action) {
        Some(k) => format!("{text}   {k}"),
        None => text.to_string(),
    }
}

/// Selects characters `from..to` of the text field `id` (the caret when they are equal).
pub fn select_text(ctx: &egui::Context, id: egui::Id, from: usize, to: usize) {
    use egui::text::{CCursor, CCursorRange};
    let mut state = egui::text_edit::TextEditState::load(ctx, id).unwrap_or_default();
    state
        .cursor
        .set_char_range(Some(CCursorRange::two(CCursor::new(from), CCursor::new(to))));
    state.store(ctx, id);
}

// ---------------------------------------------------------------------------------------------
// Buttons
// ---------------------------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// Filled accent: the one main action of a view.
    Primary,
    /// Outlined surface: ordinary actions.
    Secondary,
    /// No frame until hovered: toolbars and quiet actions.
    Ghost,
    /// Outlined, red text: destructive actions.
    Danger,
    /// Filled red: the button that confirms a destructive action in a dialog.
    DangerFilled,
}

/// A button with an optional leading icon. `small` makes it 24 px high instead of 28 (the
/// height of form controls, so it lines up with egui's own buttons in a dialog).
pub fn button_ex(ui: &mut Ui, kind: Kind, icon: Option<&str>, text: &str, small: bool) -> Response {
    button_full(ui, kind, icon, text, small, false)
}

/// [`button_ex`] that can be shown "on" (an accent-tinted toggle, e.g. Root or SFTP).
pub fn button_full(
    ui: &mut Ui,
    kind: Kind,
    icon: Option<&str>,
    text: &str,
    small: bool,
    on: bool,
) -> Response {
    button_impl(ui, kind, icon, text, None, small, on)
}

/// A button followed by its key hint, quieter (`Run Agent ⇧⌘A`, `Open in Editor ↵`).
pub fn button_keys(ui: &mut Ui, kind: Kind, icon: Option<&str>, text: &str, keys: Option<&str>, small: bool) -> Response {
    button_impl(ui, kind, icon, text, keys.filter(|k| !k.is_empty()), small, false)
}

/// Width a button takes (for placing it before drawing).
pub fn button_width(ui: &Ui, icon: Option<&str>, text: &str, keys: Option<&str>, small: bool) -> f32 {
    let size = if small { style::SMALL + 1.0 } else { style::BODY };
    let pad = if small { 8.0 } else { 12.0 };
    let height = if small { 24.0 } else { style::CONTROL_H };
    let w = |t: &str, f: FontId| ui.painter().layout_no_wrap(t.to_string(), f, Color32::PLACEHOLDER).size().x;
    let text_w = if text.is_empty() { 0.0 } else { w(text, style::medium(size)) };
    let icon_w = icon.map_or(0.0, |i| w(i, style::icon_font(size + 1.5)));
    let keys_w = keys.filter(|k| !k.is_empty()).map_or(0.0, |k| w(k, style::medium(11.0)) + 8.0);
    let gap = if text_w > 0.0 && icon_w > 0.0 { if small { 5.0 } else { 6.0 } } else { 0.0 };
    if text.is_empty() && keys_w == 0.0 {
        return height;
    }
    (text_w + icon_w + gap + keys_w + pad * 2.0).max(height)
}

fn button_impl(
    ui: &mut Ui,
    kind: Kind,
    icon: Option<&str>,
    text: &str,
    keys: Option<&str>,
    small: bool,
    on: bool,
) -> Response {
    let p = style::pal(ui);
    let size = if small {
        style::SMALL + 1.0
    } else {
        style::BODY
    };
    let height = if small { 24.0 } else { style::CONTROL_H };
    let pad = if small { 8.0 } else { 12.0 };
    let enabled = ui.is_enabled();
    let text_color = match kind {
        Kind::Primary | Kind::DangerFilled => p.on_accent,
        Kind::Danger => p.err,
        _ if on => p.accent_text,
        Kind::Secondary | Kind::Ghost => p.text,
    };
    let galley = (!text.is_empty()).then(|| {
        ui.painter()
            .layout_no_wrap(text.to_string(), style::medium(size), Color32::PLACEHOLDER)
    });
    let icon_color = match kind {
        Kind::Primary | Kind::DangerFilled => p.on_accent.gamma_multiply(0.85),
        Kind::Danger => p.err,
        _ if galley.is_none() || on => text_color,
        _ => p.text_muted,
    };
    let icon_galley = icon.map(|i| {
        ui.painter().layout_no_wrap(
            i.to_string(),
            style::icon_font(size + 1.5),
            Color32::PLACEHOLDER,
        )
    });
    let keys_galley = keys.map(|k| ui.painter().layout_no_wrap(k.to_string(), style::medium(11.0), Color32::PLACEHOLDER));
    let keys_color = match kind {
        Kind::Primary | Kind::DangerFilled => p.on_accent.gamma_multiply(0.85),
        _ => p.text_faint,
    };
    let gap = if galley.is_some() && icon_galley.is_some() {
        if small { 5.0 } else { 6.0 }
    } else {
        0.0
    };
    let keys_w = keys_galley.as_ref().map_or(0.0, |g| g.size().x + 8.0);
    let content_w = galley.as_ref().map_or(0.0, |g| g.size().x)
        + icon_galley.as_ref().map_or(0.0, |g| g.size().x)
        + gap
        + keys_w;
    let width = if galley.is_none() && keys_galley.is_none() {
        height
    } else {
        content_w + pad * 2.0
    };
    let (rect, resp) = ui.allocate_exact_size(vec2(width.max(height), height), Sense::click());
    resp.widget_info(|| WidgetInfo::selected(WidgetType::Button, enabled, on, text));
    if ui.is_rect_visible(rect) {
        let hovered = enabled && resp.hovered();
        let pressed = enabled && resp.is_pointer_button_down_on();
        let (fill, stroke) = match kind {
            Kind::Primary => (
                if !enabled {
                    p.accent.gamma_multiply(0.5)
                } else if hovered {
                    p.accent_hover
                } else {
                    p.accent
                },
                Stroke::NONE,
            ),
            Kind::DangerFilled => (
                if !enabled {
                    p.err.gamma_multiply(0.5)
                } else if hovered {
                    p.err.gamma_multiply(1.12)
                } else {
                    p.err
                },
                Stroke::NONE,
            ),
            Kind::Secondary | Kind::Danger => (
                if on {
                    p.accent_soft
                } else if pressed {
                    p.subtle
                } else if hovered {
                    p.hover
                } else {
                    p.surface
                },
                Stroke::new(1.0, if on { style::soft(p.accent, 0.45) } else { p.border_strong }),
            ),
            Kind::Ghost => (
                if on {
                    p.accent_soft
                } else if pressed {
                    p.subtle
                } else if hovered {
                    p.hover
                } else {
                    Color32::TRANSPARENT
                },
                Stroke::NONE,
            ),
        };
        let painter = ui.painter();
        let radius = CornerRadius::same(RADIUS);
        painter.rect(rect, radius, fill, stroke, StrokeKind::Inside);
        if resp.has_focus() {
            painter.rect_stroke(
                rect.expand(2.0),
                CornerRadius::same(RADIUS + 2),
                Stroke::new(1.5, p.accent),
                StrokeKind::Outside,
            );
        }
        let mut x = rect.center().x - content_w / 2.0;
        let fade = |c: Color32| {
            if enabled { c } else { c.gamma_multiply(0.45) }
        };
        if let Some(g) = icon_galley {
            let pos = egui::pos2(x, rect.center().y - g.size().y / 2.0);
            x += g.size().x + gap;
            painter.galley(pos, g, fade(icon_color));
        }
        if let Some(g) = galley {
            let pos = egui::pos2(x, rect.center().y - g.size().y / 2.0);
            x += g.size().x;
            painter.galley(pos, g, fade(text_color));
        }
        if let Some(g) = keys_galley {
            x += 8.0;
            let pos = egui::pos2(x, rect.center().y - g.size().y / 2.0);
            painter.galley(pos, g, fade(keys_color));
        }
    }
    resp
}

pub fn primary(ui: &mut Ui, text: &str) -> Response {
    button_ex(ui, Kind::Primary, None, text, false)
}

pub fn primary_icon(ui: &mut Ui, icon: &str, text: &str) -> Response {
    button_ex(ui, Kind::Primary, Some(icon), text, false)
}

pub fn secondary(ui: &mut Ui, icon: Option<&str>, text: &str) -> Response {
    button_ex(ui, Kind::Secondary, icon, text, false)
}

pub fn ghost(ui: &mut Ui, icon: Option<&str>, text: &str) -> Response {
    button_ex(ui, Kind::Ghost, icon, text, true)
}

/// A square, frameless icon button with a tooltip (also its accessible name).
pub fn icon_button(ui: &mut Ui, icon: &str, tooltip: &str) -> Response {
    icon_button_ex(ui, icon, tooltip, false, 28.0)
}

/// An icon button showing an on/off state (sidebar toggle, split, ...).
pub fn icon_toggle(ui: &mut Ui, icon: &str, tooltip: &str, on: bool) -> Response {
    icon_button_ex(ui, icon, tooltip, on, 28.0)
}

pub fn icon_button_ex(ui: &mut Ui, icon: &str, tooltip: &str, on: bool, side: f32) -> Response {
    let p = style::pal(ui);
    let (rect, resp) = ui.allocate_exact_size(Vec2::splat(side), Sense::click());
    let resp = if tooltip.is_empty() {
        resp
    } else {
        resp.on_hover_text(tooltip)
    };
    let enabled = ui.is_enabled();
    resp.widget_info(|| WidgetInfo::selected(WidgetType::Button, enabled, on, tooltip));
    if ui.is_rect_visible(rect) {
        let hovered = enabled && resp.hovered();
        let painter = ui.painter();
        let radius = CornerRadius::same(RADIUS);
        if on {
            painter.rect_filled(rect, radius, p.accent_soft);
        } else if enabled && resp.is_pointer_button_down_on() {
            painter.rect_filled(rect, radius, p.subtle);
        } else if hovered {
            painter.rect_filled(rect, radius, p.hover);
        }
        if resp.has_focus() {
            painter.rect_stroke(rect, radius, Stroke::new(1.5, p.accent), StrokeKind::Inside);
        }
        let color = if on {
            p.accent_text
        } else if hovered {
            p.text
        } else {
            p.text_muted
        };
        let color = if enabled {
            color
        } else {
            color.gamma_multiply(0.45)
        };
        painter.text(
            rect.center(),
            Align2::CENTER_CENTER,
            icon,
            style::icon_font((side * 0.58).round()),
            color,
        );
    }
    resp
}

/// Height of a segmented control.
pub const SEGMENTED_H: f32 = 26.0;
/// Room each side of a segment's content, and between segments.
const SEGMENT_PAD: f32 = 10.0;
const SEGMENT_GAP: f32 = 2.0;

/// One option of a segmented control: an optional icon, its word, an optional count after it
/// (in its own colour) and a tooltip.
#[derive(Clone)]
pub struct Seg<'a> {
    pub icon: Option<&'a str>,
    pub text: &'a str,
    pub count: Option<(String, Color32)>,
    pub tip: &'a str,
}

impl<'a> Seg<'a> {
    pub fn new(text: &'a str, tip: &'a str) -> Self {
        Self { icon: None, text, count: None, tip }
    }
    pub fn icon(mut self, icon: &'a str) -> Self {
        self.icon = Some(icon);
        self
    }
    pub fn count(mut self, n: impl ToString, color: Color32) -> Self {
        self.count = Some((n.to_string(), color));
        self
    }
}

fn segment_font() -> egui::FontId {
    style::medium(style::SMALL)
}

/// Width of a segment's content (icon, word, count).
fn segment_content(ui: &Ui, s: &Seg) -> f32 {
    let w = |t: &str, f: FontId| ui.painter().layout_no_wrap(t.to_string(), f, Color32::PLACEHOLDER).size().x;
    let mut x = w(s.text, segment_font());
    if let Some(i) = s.icon {
        x += w(i, style::icon_font(13.0)) + 5.0;
    }
    if let Some((n, _)) = &s.count {
        x += w(n, egui::FontId::proportional(10.5)) + 5.0;
    }
    x
}

/// A segmented control, `on` selected, each segment as wide as its content. Returns the option
/// clicked when it is not the selected one.
pub fn segmented(ui: &mut Ui, options: &[Seg], on: usize) -> Option<usize> {
    let widths: Vec<f32> = options.iter().map(|s| segment_content(ui, s) + 2.0 * SEGMENT_PAD).collect();
    let total = widths.iter().sum::<f32>() + SEGMENT_GAP * (options.len().saturating_sub(1)) as f32 + 4.0;
    let (rect, base) = ui.allocate_exact_size(vec2(total, SEGMENTED_H), Sense::hover());
    let mut x = rect.left() + 2.0;
    let segments: Vec<Rect> = widths
        .iter()
        .map(|w| {
            let r = Rect::from_min_size(egui::pos2(x, rect.top() + 2.0), vec2(*w, rect.height() - 4.0));
            x += w + SEGMENT_GAP;
            r
        })
        .collect();
    paint_segmented(ui, rect, base.id, &segments, options, on)
}

/// The same control filling `rect`, its segments of equal width (`segment_rects`). `id` tells
/// it apart from other controls.
pub fn segmented_in(ui: &mut Ui, rect: Rect, id: egui::Id, options: &[Seg], on: usize) -> Option<usize> {
    let segments = segment_rects(rect, options.len());
    paint_segmented(ui, rect, id, &segments, options, on)
}

/// How wide `segmented_in` needs to be for every segment's content to fit in segments of
/// equal width.
pub fn segmented_width(ui: &Ui, options: &[Seg]) -> f32 {
    let widest = options.iter().map(|s| segment_content(ui, s)).fold(0.0, f32::max);
    let n = options.len() as f32;
    n * (widest + 2.0 * SEGMENT_PAD) + (n - 1.0).max(0.0) * SEGMENT_GAP + 4.0
}

/// The segments of a `segmented_in` control over `rect`.
pub fn segment_rects(rect: Rect, n: usize) -> Vec<Rect> {
    let inner = rect.shrink(2.0);
    let n = n.max(1);
    let w = (inner.width() - SEGMENT_GAP * (n - 1) as f32) / n as f32;
    (0..n)
        .map(|i| Rect::from_min_size(egui::pos2(inner.left() + i as f32 * (w + SEGMENT_GAP), inner.top()), vec2(w, inner.height())))
        .collect()
}

fn paint_segmented(
    ui: &mut Ui,
    rect: Rect,
    id: egui::Id,
    segments: &[Rect],
    options: &[Seg],
    on: usize,
) -> Option<usize> {
    let p = style::pal(ui);
    let font = segment_font();
    ui.painter().rect_filled(rect, CornerRadius::same(RADIUS), p.subtle);
    let mut out = None;
    for (i, (r, s)) in segments.iter().zip(options).enumerate() {
        let resp = ui.interact(*r, id.with(("option", i)), Sense::click());
        let resp = if s.tip.is_empty() { resp } else { resp.on_hover_text(s.tip) };
        let selected = i == on;
        resp.widget_info(|| WidgetInfo::selected(WidgetType::Button, true, selected, s.text));
        if selected {
            ui.painter().rect(*r, CornerRadius::same(RADIUS - 2), p.surface, Stroke::new(1.0, p.border_strong), StrokeKind::Inside);
        } else if resp.hovered() {
            ui.painter().rect_filled(*r, CornerRadius::same(RADIUS - 2), p.hover);
        }
        let color = if selected || resp.hovered() { p.text } else { p.text_muted };
        let content = segment_content(ui, s);
        let mut x = r.center().x - content / 2.0;
        let cy = r.center().y;
        if let Some(icon) = s.icon {
            let g = ui.painter().layout_no_wrap(icon.to_string(), style::icon_font(13.0), color);
            ui.painter().galley(egui::pos2(x, cy - g.size().y / 2.0), g.clone(), color);
            x += g.size().x + 5.0;
        }
        let g = ui.painter().layout_no_wrap(s.text.to_string(), font.clone(), color);
        let w = g.size().x;
        ui.painter().galley(egui::pos2(x, cy - g.size().y / 2.0), g, color);
        x += w + 5.0;
        if let Some((n, c)) = &s.count {
            let g = ui.painter().layout_no_wrap(n.clone(), egui::FontId::proportional(10.5), *c);
            ui.painter().galley(egui::pos2(x, cy - g.size().y / 2.0 + 0.5), g, *c);
        }
        if resp.clicked() && !selected {
            out = Some(i);
        }
    }
    out
}

// ---------------------------------------------------------------------------------------------
// Tabs
// ---------------------------------------------------------------------------------------------

/// A row of tabs with an underline under the selected one and a hairline under the whole
/// row. Returns each tab's response.
pub fn tab_strip<T: PartialEq + Copy>(
    ui: &mut Ui,
    current: &mut T,
    items: &[(T, &str)],
) -> Vec<Response> {
    let p = style::pal(ui);
    let start_x = ui.cursor().min.x;
    let right = ui.max_rect().right();
    let pad = 10.0;
    let mut out = Vec::with_capacity(items.len());
    let bottom = ui
        .horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            for (value, label) in items {
                let selected = *current == *value;
                let galley = ui.painter().layout_no_wrap(
                    label.to_string(),
                    style::medium(style::BODY),
                    Color32::PLACEHOLDER,
                );
                let (rect, resp) =
                    ui.allocate_exact_size(vec2(galley.size().x + 2.0 * pad, 34.0), Sense::click());
                resp.widget_info(|| {
                    WidgetInfo::selected(WidgetType::SelectableLabel, true, selected, *label)
                });
                if resp.clicked() {
                    *current = *value;
                }
                let painter = ui.painter();
                let color = if selected || resp.hovered() {
                    p.text
                } else {
                    p.text_muted
                };
                painter.galley(
                    egui::pos2(rect.left() + pad, rect.center().y - galley.size().y / 2.0),
                    galley,
                    color,
                );
                if selected {
                    let line = Rect::from_min_max(
                        egui::pos2(rect.left() + pad - 4.0, rect.bottom() - 2.0),
                        egui::pos2(rect.right() - pad + 4.0, rect.bottom()),
                    );
                    painter.rect_filled(line, CornerRadius::same(1), p.accent);
                }
                if resp.has_focus() {
                    painter.rect_stroke(
                        rect.shrink(2.0),
                        CornerRadius::same(RADIUS),
                        Stroke::new(1.5, p.accent),
                        StrokeKind::Inside,
                    );
                }
                out.push(resp);
            }
        })
        .response
        .rect
        .bottom();
    ui.painter()
        .hline(start_x..=right, bottom - 0.5, Stroke::new(1.0, p.border));
    out
}

/// Covers `rect` with `color`, from clear to opaque towards the right (`to_right`) or the
/// left: the edge of a strip that hides more.
pub fn fade(painter: &egui::Painter, rect: Rect, color: Color32, to_right: bool) {
    let (left, right) = if to_right {
        (Color32::TRANSPARENT, color)
    } else {
        (color, Color32::TRANSPARENT)
    };
    let mut mesh = egui::Mesh::default();
    mesh.colored_vertex(rect.left_top(), left);
    mesh.colored_vertex(rect.right_top(), right);
    mesh.colored_vertex(rect.right_bottom(), right);
    mesh.colored_vertex(rect.left_bottom(), left);
    mesh.add_triangle(0, 1, 2);
    mesh.add_triangle(0, 2, 3);
    painter.add(mesh);
}

// ---------------------------------------------------------------------------------------------
// Small pieces
// ---------------------------------------------------------------------------------------------

/// A keyboard shortcut hint (`⌘K`) drawn as a key cap.
pub fn kbd(ui: &mut Ui, keys: &str) -> Response {
    let p = style::pal(ui);
    let galley = ui.painter().layout_no_wrap(keys.to_string(), style::medium(10.5), p.text_muted);
    let size = vec2(galley.size().x + 12.0, KBD_H);
    let (rect, resp) = ui.allocate_exact_size(size, Sense::hover());
    if ui.is_rect_visible(rect) {
        paint_kbd(ui.painter(), rect, &p);
        ui.painter()
            .galley(rect.center() - galley.size() / 2.0 - vec2(0.0, 0.5), galley, p.text_muted);
    }
    resp
}

/// Height of a key cap.
pub const KBD_H: f32 = 17.0;

/// Paints a key cap with its text at `left_center`; returns its width.
/// How wide `paint_kbd_text` draws `keys`.
pub fn kbd_width(painter: &egui::Painter, keys: &str) -> f32 {
    painter.layout_no_wrap(keys.to_string(), style::medium(10.5), Color32::PLACEHOLDER).size().x + 12.0
}

pub fn paint_kbd_text(painter: &egui::Painter, left_center: egui::Pos2, keys: &str, p: &style::Palette) -> f32 {
    let galley = painter.layout_no_wrap(keys.to_string(), style::medium(10.5), p.text_muted);
    let rect = Rect::from_min_size(egui::pos2(left_center.x, left_center.y - KBD_H / 2.0), vec2(galley.size().x + 12.0, KBD_H));
    paint_kbd(painter, rect, p);
    painter.galley(rect.center() - galley.size() / 2.0 - vec2(0.0, 0.5), galley, p.text_muted);
    rect.width()
}

/// The key cap behind a shortcut hint: a subtle fill, a border thicker at the bottom.
pub fn paint_kbd(painter: &egui::Painter, rect: Rect, p: &style::Palette) {
    painter.rect(
        rect,
        CornerRadius::same(4),
        p.subtle,
        Stroke::new(1.0, p.border),
        StrokeKind::Inside,
    );
    painter.hline(rect.x_range().shrink(3.0), rect.bottom() - 1.0, Stroke::new(1.0, p.border));
}

/// A filled dot (connection state).
pub fn dot(ui: &mut Ui, color: Color32, radius: f32) -> Response {
    let (rect, resp) = ui.allocate_exact_size(Vec2::splat(radius * 2.0 + 2.0), Sense::hover());
    ui.painter().circle_filled(rect.center(), radius, color);
    resp
}

/// A thin vertical divider between toolbar groups.
pub fn vdivider(ui: &mut Ui, height: f32) {
    let p = style::pal(ui);
    let (rect, _) = ui.allocate_exact_size(vec2(9.0, height), Sense::hover());
    ui.painter()
        .vline(rect.center().x, rect.y_range(), Stroke::new(1.0, p.border));
}

/// An icon on its own, as a label.
pub fn icon(ui: &mut Ui, icon: &str, size: f32, color: Color32) -> Response {
    ui.add(
        egui::Label::new(
            RichText::new(icon)
                .font(style::icon_font(size))
                .color(color),
        )
        .selectable(false),
    )
}

/// A bordered card.
pub fn card(ui: &Ui) -> egui::Frame {
    let p = style::pal(ui);
    egui::Frame::new()
        .fill(p.surface)
        .stroke(Stroke::new(1.0, p.border))
        .corner_radius(CornerRadius::same(8))
        .inner_margin(egui::Margin::same(14))
}

/// An icon in a soft circle of `color` (dialog headers, empty states).
pub fn icon_badge(ui: &mut Ui, icon: &str, color: Color32, diameter: f32) -> Response {
    let (rect, resp) = ui.allocate_exact_size(Vec2::splat(diameter), Sense::hover());
    let dark = ui.visuals().dark_mode;
    ui.painter().circle_filled(
        rect.center(),
        diameter / 2.0,
        style::soft(color, if dark { 0.18 } else { 0.12 }),
    );
    ui.painter().text(
        rect.center(),
        Align2::CENTER_CENTER,
        icon,
        style::icon_font((diameter * 0.48).round()),
        color,
    );
    resp
}

/// A centered empty state: an icon in a soft circle, a title, a line of explanation, then
/// whatever `actions` adds (buttons, shortcut hints).
pub fn empty_state(
    ui: &mut Ui,
    icon: &str,
    title: &str,
    body: &str,
    actions: impl FnOnce(&mut Ui),
) {
    let p = style::pal(ui);
    ui.vertical_centered(|ui| {
        ui.add_space((ui.available_height() * 0.22).clamp(12.0, 140.0));
        let (rect, _) = ui.allocate_exact_size(Vec2::splat(56.0), Sense::hover());
        ui.painter().circle_filled(rect.center(), 28.0, p.subtle);
        ui.painter().text(
            rect.center(),
            Align2::CENTER_CENTER,
            icon,
            style::icon_font(26.0),
            p.text_muted,
        );
        ui.add_space(10.0);
        ui.label(
            RichText::new(title)
                .font(style::semibold(15.0))
                .color(p.text),
        );
        ui.add_space(2.0);
        ui.label(RichText::new(body).color(p.text_muted));
        ui.add_space(12.0);
        actions(ui);
    });
}

/// `ScrollArea::show_rows` for rows of exactly `row_h` laid out edge to edge. egui places the
/// rows `row_h + item_spacing.y` apart, so the spacing is zeroed around the area too:
/// otherwise the rows slip by that much each time one scrolls out of view and the bottom of
/// the list is left empty.
pub fn show_rows<R>(
    area: egui::ScrollArea,
    ui: &mut Ui,
    row_h: f32,
    total: usize,
    add: impl FnOnce(&mut Ui, std::ops::Range<usize>) -> R,
) -> egui::scroll_area::ScrollAreaOutput<R> {
    ui.scope(|ui| {
        ui.spacing_mut().item_spacing.y = 0.0;
        area.show_rows(ui, row_h, total, add)
    })
    .inner
}

// ---------------------------------------------------------------------------------------------
// Pills, badges, labels
// ---------------------------------------------------------------------------------------------

/// Height of a pill.
pub const PILL_H: f32 = 18.0;

/// How wide a pill with `icon` and `text` is.
pub fn pill_width(painter: &egui::Painter, icon: Option<&str>, text: &str) -> f32 {
    let w = painter.layout_no_wrap(text.to_string(), style::medium(11.0), Color32::PLACEHOLDER).size().x;
    let i = icon.map_or(0.0, |i| painter.layout_no_wrap(i.to_string(), style::icon_font(11.5), Color32::PLACEHOLDER).size().x + 4.0);
    w + i + 14.0
}

/// Paints a pill (soft fill, coloured icon and text) with its left edge at `left_center`;
/// returns its rectangle.
pub fn paint_pill(painter: &egui::Painter, left_center: egui::Pos2, icon: Option<&str>, text: &str, fg: Color32, bg: Color32) -> Rect {
    let w = pill_width(painter, icon, text);
    let rect = Rect::from_min_size(egui::pos2(left_center.x, left_center.y - PILL_H / 2.0), vec2(w, PILL_H));
    painter.rect_filled(rect, CornerRadius::same(9), bg);
    let mut x = rect.left() + 7.0;
    if let Some(i) = icon {
        let g = painter.layout_no_wrap(i.to_string(), style::icon_font(11.5), fg);
        painter.galley(egui::pos2(x, rect.center().y - g.size().y / 2.0), g.clone(), fg);
        x += g.size().x + 4.0;
    }
    let g = painter.layout_no_wrap(text.to_string(), style::medium(11.0), fg);
    painter.galley(egui::pos2(x, rect.center().y - g.size().y / 2.0), g, fg);
    rect
}

/// A pill as a widget.
pub fn pill(ui: &mut Ui, icon: Option<&str>, text: &str, fg: Color32, bg: Color32) -> Response {
    let w = pill_width(ui.painter(), icon, text);
    let (rect, resp) = ui.allocate_exact_size(vec2(w, PILL_H), Sense::hover());
    if ui.is_rect_visible(rect) {
        paint_pill(ui.painter(), rect.left_center(), icon, text, fg, bg);
    }
    resp
}

/// A status pill: the status colour on its soft tint.
pub fn status_pill(ui: &mut Ui, icon: &str, text: &str, color: Color32) -> Response {
    let bg = style::tint(color, ui.visuals().dark_mode);
    pill(ui, Some(icon), text, color, bg)
}

/// A count in a filled capsule (16 high), its right edge at `right_center`; returns its
/// rectangle.
pub fn paint_badge(painter: &egui::Painter, right_center: egui::Pos2, text: &str, fill: Color32, fg: Color32, height: f32) -> Rect {
    let g = painter.layout_no_wrap(text.to_string(), style::semibold(if height < 16.0 { 10.0 } else { 10.5 }), fg);
    let w = (g.size().x + 10.0).max(height + 1.0);
    let rect = Rect::from_min_size(egui::pos2(right_center.x - w, right_center.y - height / 2.0), vec2(w, height));
    painter.rect_filled(rect, CornerRadius::same((height / 2.0) as u8), fill);
    painter.galley(rect.center() - g.size() / 2.0, g, fg);
    rect
}

/// A small upper-case section label ("PROJECTS"), letter-spaced; returns the galley to place.
pub fn section_galley(painter: &egui::Painter, text: &str, color: Color32) -> std::sync::Arc<egui::Galley> {
    let mut job = egui::text::LayoutJob::default();
    let mut f = egui::TextFormat::simple(style::semibold(10.5), color);
    f.extra_letter_spacing = 0.63;
    job.append(&text.to_uppercase(), 0.0, f);
    painter.layout_job(job)
}

/// A key hint as plain quiet text (`⌘J`), as menus and rows show them.
pub fn keys_galley(painter: &egui::Painter, keys: &str, color: Color32) -> std::sync::Arc<egui::Galley> {
    painter.layout_no_wrap(keys.to_string(), egui::FontId::proportional(11.0), color)
}

// ---------------------------------------------------------------------------------------------
// Form controls
// ---------------------------------------------------------------------------------------------

/// An on/off switch (30 × 18). Clicking flips `on`; the response says it changed.
pub fn switch(ui: &mut Ui, on: &mut bool) -> Response {
    let p = style::pal(ui);
    let (rect, mut resp) = ui.allocate_exact_size(vec2(30.0, 18.0), Sense::click());
    if resp.clicked() {
        *on = !*on;
        resp.mark_changed();
    }
    let enabled = ui.is_enabled();
    resp.widget_info(|| WidgetInfo::selected(WidgetType::Checkbox, enabled, *on, ""));
    if ui.is_rect_visible(rect) {
        let t = ui.ctx().animate_bool_with_time(resp.id, *on, 0.12);
        let track = if *on { p.accent } else { p.border_strong };
        let fade = |c: Color32| if enabled { c } else { c.gamma_multiply(0.45) };
        let painter = ui.painter();
        painter.rect_filled(rect, CornerRadius::same(9), fade(track));
        let x = egui::lerp(rect.left() + 9.0..=rect.right() - 9.0, t);
        let c = egui::pos2(x, rect.center().y);
        painter.circle_filled(c + vec2(0.0, 0.5), 7.0, Color32::from_black_alpha(60));
        painter.circle_filled(c, 7.0, fade(Color32::WHITE));
        if resp.has_focus() {
            painter.rect_stroke(rect.expand(2.0), CornerRadius::same(11), Stroke::new(1.5, p.accent), StrokeKind::Outside);
        }
    }
    resp
}

/// A number with − and + on either side (28 high). Returns −1 or +1 when one is clicked.
pub fn stepper(ui: &mut Ui, text: &str) -> i32 {
    let p = style::pal(ui);
    let g = ui.painter().layout_no_wrap(text.to_string(), egui::FontId::proportional(13.0), p.text);
    let w = 26.0 * 2.0 + g.size().x + 20.0 + 2.0;
    let (rect, resp) = ui.allocate_exact_size(vec2(w, style::CONTROL_H), Sense::hover());
    let mut out = 0;
    if ui.is_rect_visible(rect) || resp.hovered() {
        let painter = ui.painter().clone();
        painter.rect(rect, CornerRadius::same(RADIUS), p.surface, Stroke::new(1.0, p.border_strong), StrokeKind::Inside);
        let minus = Rect::from_min_size(rect.min + vec2(1.0, 1.0), vec2(26.0, rect.height() - 2.0));
        let plus = Rect::from_min_size(egui::pos2(rect.right() - 27.0, rect.top() + 1.0), vec2(26.0, rect.height() - 2.0));
        for (r, glyph, step, salt) in [(minus, "−", -1, "minus"), (plus, "+", 1, "plus")] {
            let b = ui.interact(r, resp.id.with(salt), Sense::click());
            if b.hovered() {
                painter.rect_filled(r, CornerRadius::same(RADIUS - 1), p.hover);
            }
            painter.text(r.center(), Align2::CENTER_CENTER, glyph, egui::FontId::proportional(13.0), if b.hovered() { p.text } else { p.text_muted });
            if b.clicked() {
                out = step;
            }
        }
        painter.vline(minus.right(), rect.y_range().shrink(1.0), Stroke::new(1.0, p.border));
        painter.vline(plus.left(), rect.y_range().shrink(1.0), Stroke::new(1.0, p.border));
        painter.galley(rect.center() - g.size() / 2.0, g, p.text);
    }
    out
}

/// A drop-down: the current choice in a box with a caret; a click lists `options` under it.
/// Returns the option picked.
pub fn select(ui: &mut Ui, id_salt: impl std::hash::Hash + std::fmt::Debug, current: &str, options: &[&str], min_width: f32) -> Option<usize> {
    let p = style::pal(ui);
    let g = ui.painter().layout_no_wrap(current.to_string(), egui::FontId::proportional(13.0), p.text);
    let w = (g.size().x + 10.0 + 8.0 + 12.0 + 8.0).max(min_width);
    let (rect, resp) = ui.allocate_exact_size(vec2(w, style::CONTROL_H), Sense::click());
    let resp = resp.on_hover_cursor(egui::CursorIcon::PointingHand);
    let id = ui.id().with(id_salt);
    if ui.is_rect_visible(rect) {
        let fill = if resp.hovered() { p.hover } else { p.surface };
        ui.painter().rect(rect, CornerRadius::same(RADIUS), fill, Stroke::new(1.0, p.border_strong), StrokeKind::Inside);
        ui.painter().galley(egui::pos2(rect.left() + 10.0, rect.center().y - g.size().y / 2.0), g, p.text);
        ui.painter().text(egui::pos2(rect.right() - 8.0, rect.center().y), Align2::RIGHT_CENTER, super::icons::CARET_DOWN, style::icon_font(12.0), p.text_faint);
    }
    let mut picked = None;
    egui::Popup::menu(&resp).id(id).width(w).show(|ui| {
        menu_body(ui);
        for (i, o) in options.iter().enumerate() {
            let on = *o == current;
            if menu_item(ui, on.then_some(super::icons::CHECK), o, None, false).clicked() {
                picked = Some(i);
                ui.close();
            }
        }
    });
    picked
}

/// A text field with an optional leading icon and a hint (30 high, 26 when `small`), on the
/// canvas colour, ringed in the accent while it has the keyboard.
pub fn field(ui: &mut Ui, id: egui::Id, text: &mut String, icon: Option<&str>, hint: &str, width: f32, small: bool) -> Response {
    let p = style::pal(ui);
    let h = if small { 26.0 } else { 30.0 };
    let pad = if small { 8.0 } else { 10.0 };
    let font = egui::FontId::proportional(if small { 12.5 } else { style::BODY });
    let (rect, _) = ui.allocate_exact_size(vec2(width, h), Sense::hover());
    let focused = ui.ctx().memory(|m| m.has_focus(id));
    let painter = ui.painter();
    if focused {
        painter.rect_stroke(rect.expand(3.0), CornerRadius::same(RADIUS + 3), Stroke::new(3.0, style::soft(p.accent, 0.22)), StrokeKind::Inside);
    }
    painter.rect(rect, CornerRadius::same(RADIUS), p.canvas, Stroke::new(1.0, if focused { p.accent } else { p.border_strong }), StrokeKind::Inside);
    let mut x = rect.left() + pad;
    if let Some(i) = icon {
        let g = painter.layout_no_wrap(i.to_string(), style::icon_font(14.0), p.text_faint);
        painter.galley(egui::pos2(x, rect.center().y - g.size().y / 2.0), g.clone(), p.text_faint);
        x += g.size().x + 8.0;
    }
    let inner = Rect::from_min_max(egui::pos2(x, rect.top() + 1.0), egui::pos2(rect.right() - pad, rect.bottom() - 1.0));
    // A child that leaves the parent's layout alone (the field's place is taken above).
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(inner).layout(egui::Layout::left_to_right(egui::Align::Center)));
    child.add_sized(
        inner.size(),
        egui::TextEdit::singleline(text)
            .id(id)
            .frame(egui::Frame::NONE)
            .margin(egui::Margin::ZERO)
            .font(font)
            .hint_text(RichText::new(hint).color(p.text_faint))
            .vertical_align(egui::Align::Center),
    )
}

// ---------------------------------------------------------------------------------------------
// Menus
// ---------------------------------------------------------------------------------------------

/// Call first in a menu's contents: items sit edge to edge.
pub fn menu_body(ui: &mut Ui) {
    ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
    ui.set_min_width(220.0);
}

/// A menu row: an icon (or room for one), a label, a key hint at the right. Hovered, it fills
/// with the accent. `danger` draws it in the error colour.
pub fn menu_item(ui: &mut Ui, icon: Option<&str>, label: &str, keys: Option<&str>, danger: bool) -> Response {
    let p = style::pal(ui);
    let enabled = ui.is_enabled();
    let label_g = ui.painter().layout_no_wrap(label.to_string(), egui::FontId::proportional(13.0), Color32::PLACEHOLDER);
    let keys_g = keys.filter(|k| !k.is_empty()).map(|k| keys_galley(ui.painter(), k, Color32::PLACEHOLDER));
    let want = 8.0 + 15.0 + 9.0 + label_g.size().x + keys_g.as_ref().map_or(0.0, |g| g.size().x + 18.0) + 8.0;
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width().max(want), 26.0), Sense::click());
    resp.widget_info(|| WidgetInfo::labeled(WidgetType::Button, enabled, label));
    if ui.is_rect_visible(rect) {
        let hovered = enabled && resp.hovered();
        let painter = ui.painter();
        if hovered {
            painter.rect_filled(rect, CornerRadius::same(5), p.accent);
        }
        let fade = |c: Color32| if enabled { c } else { c.gamma_multiply(0.45) };
        let (text_c, icon_c, keys_c) = if hovered {
            (p.on_accent, p.on_accent.gamma_multiply(0.85), p.on_accent.gamma_multiply(0.85))
        } else if danger {
            (p.err, p.err, p.text_faint)
        } else {
            (p.text, p.text_muted, p.text_faint)
        };
        if let Some(i) = icon {
            painter.text(egui::pos2(rect.left() + 8.0 + 7.5, rect.center().y), Align2::CENTER_CENTER, i, style::icon_font(14.0), fade(icon_c));
        }
        painter.galley(egui::pos2(rect.left() + 32.0, rect.center().y - label_g.size().y / 2.0), label_g, fade(text_c));
        if let Some(g) = keys_g {
            painter.galley(egui::pos2(rect.right() - 8.0 - g.size().x, rect.center().y - g.size().y / 2.0), g, fade(keys_c));
        }
    }
    resp
}

/// A line between groups of a menu.
pub fn menu_sep(ui: &mut Ui) {
    let p = style::pal(ui);
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 11.0), Sense::hover());
    ui.painter().hline(rect.x_range().shrink(4.0), rect.center().y, Stroke::new(1.0, p.border));
}

/// A small title over a group of menu rows.
pub fn menu_header(ui: &mut Ui, text: &str) {
    let p = style::pal(ui);
    let g = section_galley(ui.painter(), text, p.text_faint);
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), g.size().y + 10.0), Sense::hover());
    ui.painter().galley(egui::pos2(rect.left() + 8.0, rect.top() + 6.0), g, p.text_faint);
}

/// A menu row that opens a submenu (a caret at its right); show the submenu with
/// `egui::containers::menu::SubMenu::new().show(ui, &response, …)`.
pub fn submenu_item(ui: &mut Ui, icon: Option<&str>, label: &str) -> Response {
    let r = menu_item(ui, icon, label, None, false);
    let p = style::pal(ui);
    let c = if r.hovered() { p.on_accent } else { p.text_faint };
    ui.painter().text(egui::pos2(r.rect.right() - 10.0, r.rect.center().y), Align2::CENTER_CENTER, super::icons::CARET_RIGHT, style::icon_font(11.0), c);
    r
}
