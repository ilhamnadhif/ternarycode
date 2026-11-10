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
    let icon_color = if matches!(kind, Kind::Primary | Kind::Danger | Kind::DangerFilled) || galley.is_none() || on {
        text_color
    } else {
        p.text_muted
    };
    let icon_galley = icon.map(|i| {
        ui.painter().layout_no_wrap(
            i.to_string(),
            style::icon_font(size + 2.0),
            Color32::PLACEHOLDER,
        )
    });
    let gap = if galley.is_some() && icon_galley.is_some() {
        6.0
    } else {
        0.0
    };
    let content_w = galley.as_ref().map_or(0.0, |g| g.size().x)
        + icon_galley.as_ref().map_or(0.0, |g| g.size().x)
        + gap;
    let width = if galley.is_none() {
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
                Stroke::new(
                    1.0,
                    if on {
                        style::soft(p.accent, 0.45)
                    } else if hovered {
                        p.border_strong
                    } else {
                        p.border
                    },
                ),
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
            painter.galley(pos, g, fade(text_color));
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

/// A segmented control of words (`(label, tooltip)`), `on` selected. Returns the option clicked
/// when it is not the selected one.
pub fn segmented(ui: &mut Ui, options: &[(&str, &str)], on: usize) -> Option<usize> {
    let p = style::pal(ui);
    let h = 24.0;
    let font = style::medium(style::SMALL);
    let galleys: Vec<_> = options
        .iter()
        .map(|(text, _)| ui.painter().layout_no_wrap(text.to_string(), font.clone(), Color32::PLACEHOLDER))
        .collect();
    let widths: Vec<f32> = galleys.iter().map(|g| g.size().x + 18.0).collect();
    let (rect, base) = ui.allocate_exact_size(vec2(widths.iter().sum::<f32>() + 4.0, h), Sense::hover());
    ui.painter().rect_filled(rect, CornerRadius::same(RADIUS), p.subtle);
    let mut x = rect.left() + 2.0;
    let mut out = None;
    for (i, ((galley, w), (text, tip))) in galleys.into_iter().zip(widths).zip(options).enumerate() {
        let r = Rect::from_min_size(egui::pos2(x, rect.top() + 2.0), vec2(w, h - 4.0));
        x += w;
        let resp = ui.interact(r, base.id.with(("option", i)), Sense::click()).on_hover_text(*tip);
        let selected = i == on;
        resp.widget_info(|| WidgetInfo::selected(WidgetType::Button, true, selected, *text));
        if selected {
            ui.painter().rect_filled(r, CornerRadius::same(RADIUS - 2), p.surface);
            ui.painter().rect_stroke(r, CornerRadius::same(RADIUS - 2), Stroke::new(1.0, p.border), StrokeKind::Inside);
        } else if resp.hovered() {
            ui.painter().rect_filled(r, CornerRadius::same(RADIUS - 2), p.hover);
        }
        let color = if selected || resp.hovered() { p.text } else { p.text_muted };
        ui.painter().galley(r.center() - galley.size() / 2.0, galley, color);
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
    let galley = ui.painter().layout_no_wrap(
        keys.to_string(),
        FontId::proportional(style::SMALL),
        p.text_muted,
    );
    let size = galley.size() + vec2(10.0, 4.0);
    let (rect, resp) = ui.allocate_exact_size(size, Sense::hover());
    if ui.is_rect_visible(rect) {
        paint_kbd(ui.painter(), rect, &p);
        ui.painter()
            .galley(rect.center() - galley.size() / 2.0, galley, p.text_muted);
    }
    resp
}

/// The key cap behind a shortcut hint.
pub fn paint_kbd(painter: &egui::Painter, rect: Rect, p: &style::Palette) {
    painter.rect(
        rect,
        CornerRadius::same(4),
        p.subtle,
        Stroke::new(1.0, p.border),
        StrokeKind::Inside,
    );
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
