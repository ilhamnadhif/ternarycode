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
        A::ToggleCompactSidebar => ("⌥⌘B", "Ctrl+Alt+B"),
        A::ToggleEditor => ("⌘E", "Ctrl+Shift+Y"),
        A::ToggleZenMode => ("⌥⌘Z", "Ctrl+Alt+Z"),
        A::ToggleDrawer => ("⇧⌘L", "Ctrl+Shift+L"),
        A::QuickOpen => ("⌘O", "Ctrl+Shift+O"),
        A::AddProject => ("⇧⌘O", "Ctrl+Shift+N"),
        A::PickPreset => ("⇧⌘A", "Ctrl+Shift+A"),
        A::SplitRight => ("⌘D", "Ctrl+Shift+D"),
        A::SplitDown => ("⇧⌘D", "Ctrl+Shift+E"),
        A::NewTab => ("⌘T", "Ctrl+Shift+T"),
        A::ShowBrowser => ("⇧⌘B", "Ctrl+Shift+Alt+B"),
        A::NewBrowserTab => ("⌥⇧⌘B", ""),
        A::ReopenClosedPage => ("⇧⌘T", "Ctrl+Shift+Alt+T"),
        A::BrowserAddress => ("⌘L", ""),
        A::BrowserReload => ("⌘R", ""),
        A::ClosePane => ("⌘W", "Ctrl+Shift+W"),
        A::Review => ("⇧⌘R", "Ctrl+Shift+R"),
        A::Commit => ("⇧⌘C", "Ctrl+Shift+Alt+C"),
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
    button_impl(ui, kind, icon, text, None, if small { Size::Small } else { Size::Normal }, on)
}

/// A button followed by its key hint, quieter (`Run Agent ⇧⌘A`, `Open in Editor ↵`).
pub fn button_keys(ui: &mut Ui, kind: Kind, icon: Option<&str>, text: &str, keys: Option<&str>, small: bool) -> Response {
    button_impl(ui, kind, icon, text, keys.filter(|k| !k.is_empty()), if small { Size::Small } else { Size::Normal }, false)
}

/// A large button (34 high): the one action of an empty screen.
pub fn button_large(ui: &mut Ui, kind: Kind, icon: Option<&str>, text: &str, keys: Option<&str>) -> Response {
    button_impl(ui, kind, icon, text, keys.filter(|k| !k.is_empty()), Size::Large, false)
}

/// A button's three sizes: 24, 28 and 34 high.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Size {
    Small,
    Normal,
    Large,
}

impl Size {
    fn height(self) -> f32 {
        match self {
            Size::Small => 24.0,
            Size::Normal => style::CONTROL_H,
            Size::Large => 34.0,
        }
    }
    fn pad(self) -> f32 {
        match self {
            Size::Small => 8.0,
            Size::Normal => 12.0,
            Size::Large => 16.0,
        }
    }
    fn font(self) -> f32 {
        match self {
            Size::Small => 12.0,
            Size::Normal => 13.0,
            Size::Large => 13.5,
        }
    }
    fn icon(self) -> f32 {
        match self {
            Size::Small => 13.0,
            Size::Normal => 14.0,
            Size::Large => 15.0,
        }
    }
    fn gap(self) -> f32 {
        6.0
    }
    fn radius(self) -> u8 {
        match self {
            Size::Small | Size::Normal => 6,
            Size::Large => 8,
        }
    }
}

/// Width a button takes (for placing it before drawing).
pub fn button_width(ui: &Ui, icon: Option<&str>, text: &str, keys: Option<&str>, small: bool) -> f32 {
    let size = if small { Size::Small } else { Size::Normal };
    let w = |t: &str, f: FontId| ui.painter().layout_no_wrap(t.to_string(), f, Color32::PLACEHOLDER).size().x;
    let text_w = if text.is_empty() { 0.0 } else { w(text, style::medium(size.font())) };
    let icon_w = icon.map_or(0.0, |i| w(i, style::icon_font(size.icon())));
    let keys_w = keys.filter(|k| !k.is_empty()).map_or(0.0, |k| w(k, style::mono(11.0)) + size.gap());
    let gap = if text_w > 0.0 && icon_w > 0.0 { size.gap() } else { 0.0 };
    if text.is_empty() && keys_w == 0.0 {
        return size.height();
    }
    (text_w + icon_w + gap + keys_w + size.pad() * 2.0).max(size.height())
}

/// The 1 px light along the top edge of something raised (a button, a selected segment, a
/// dialog): what makes it read as lifted in the dark theme.
pub fn paint_highlight(painter: &egui::Painter, rect: Rect, radius: u8, p: &style::Palette) {
    let r = radius as f32;
    painter.hline((rect.left() + r * 0.7)..=(rect.right() - r * 0.7), rect.top() + 0.5, Stroke::new(1.0, p.highlight));
}

fn button_impl(
    ui: &mut Ui,
    kind: Kind,
    icon: Option<&str>,
    text: &str,
    keys: Option<&str>,
    size: Size,
    on: bool,
) -> Response {
    let p = style::pal(ui);
    let dark = ui.visuals().dark_mode;
    let height = size.height();
    let pad = size.pad();
    let enabled = ui.is_enabled();
    let text_color = match kind {
        Kind::Primary => p.on_accent,
        Kind::Danger | Kind::DangerFilled => p.err,
        _ if on => p.accent_text,
        Kind::Secondary => p.text,
        Kind::Ghost => p.text_muted,
    };
    let galley = (!text.is_empty()).then(|| {
        ui.painter()
            .layout_no_wrap(text.to_string(), style::medium(size.font()), Color32::PLACEHOLDER)
    });
    let icon_galley = icon.map(|i| {
        ui.painter().layout_no_wrap(
            i.to_string(),
            style::icon_font(size.icon()),
            Color32::PLACEHOLDER,
        )
    });
    let keys_galley = keys.map(|k| ui.painter().layout_no_wrap(k.to_string(), style::mono(11.0), Color32::PLACEHOLDER));
    let keys_color = match kind {
        Kind::Primary => p.on_accent.gamma_multiply(0.65),
        Kind::Danger | Kind::DangerFilled => style::soft(p.err, 0.7),
        _ => p.text_faint,
    };
    let gap = if galley.is_some() && icon_galley.is_some() { size.gap() } else { 0.0 };
    let keys_w = keys_galley.as_ref().map_or(0.0, |g| g.size().x + size.gap());
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
    if enabled && resp.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    resp.widget_info(|| WidgetInfo::selected(WidgetType::Button, enabled, on, text));
    if ui.is_rect_visible(rect) {
        let hovered = enabled && resp.hovered();
        let pressed = enabled && resp.is_pointer_button_down_on();
        let tint = style::tint(p.err, dark);
        // Slate buttons are flat: a fill, a hairline on Secondary, no highlight or drop shadow.
        let (fill, stroke) = match kind {
            Kind::Primary => (
                if !enabled {
                    p.accent.gamma_multiply(0.45)
                } else if pressed || hovered {
                    p.accent_hover
                } else {
                    p.accent
                },
                Stroke::NONE,
            ),
            Kind::Danger | Kind::DangerFilled => (
                if pressed {
                    style::soft(p.err, if dark { 0.26 } else { 0.18 })
                } else if hovered {
                    style::soft(p.err, if dark { 0.19 } else { 0.14 })
                } else {
                    tint
                },
                Stroke::NONE,
            ),
            Kind::Secondary => (
                if on {
                    p.accent_tint
                } else if pressed {
                    style::over(p.surface, if dark { Color32::WHITE } else { Color32::BLACK }, 0.07)
                } else if hovered {
                    style::over(p.surface, if dark { Color32::WHITE } else { Color32::BLACK }, 0.04)
                } else {
                    p.surface
                },
                Stroke::new(1.0, if on { style::soft(p.accent, 0.45) } else { p.border_strong }),
            ),
            Kind::Ghost => (
                if on {
                    p.accent_tint
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
        let radius = CornerRadius::same(size.radius());
        painter.rect(rect, radius, fill, stroke, StrokeKind::Inside);
        if resp.has_focus() {
            painter.rect_stroke(
                rect.expand(2.0),
                CornerRadius::same(size.radius() + 2),
                Stroke::new(2.0, p.ring),
                StrokeKind::Outside,
            );
        }
        let mut x = rect.center().x - content_w / 2.0;
        let fade = |c: Color32| {
            if enabled { c } else { c.gamma_multiply(0.45) }
        };
        let hover_text = if kind == Kind::Ghost && hovered && !on { p.text } else { text_color };
        let icon_color = match kind {
            Kind::Primary | Kind::DangerFilled | Kind::Danger => text_color,
            _ if galley.is_none() || on => hover_text,
            _ => p.text_muted,
        };
        if let Some(g) = icon_galley {
            let pos = egui::pos2(x, rect.center().y - g.size().y / 2.0);
            x += g.size().x + gap;
            painter.galley(pos, g, fade(icon_color));
        }
        if let Some(g) = galley {
            let pos = egui::pos2(x, rect.center().y - g.size().y / 2.0);
            x += g.size().x;
            painter.galley(pos, g, fade(hover_text));
        }
        if let Some(g) = keys_galley {
            x += size.gap();
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
    if enabled && resp.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    resp.widget_info(|| WidgetInfo::selected(WidgetType::Button, enabled, on, tooltip));
    if ui.is_rect_visible(rect) {
        let hovered = enabled && resp.hovered();
        let painter = ui.painter();
        let r = if side >= 21.0 { RADIUS } else { 5 };
        let radius = CornerRadius::same(r);
        if on {
            // Selected: a toggle that is on (a panel open) is the accent's tint.
            painter.rect_filled(rect, radius, p.accent_tint);
        } else if enabled && resp.is_pointer_button_down_on() {
            painter.rect_filled(rect, radius, p.subtle);
        } else if hovered {
            painter.rect_filled(rect, radius, p.hover);
        }
        if resp.has_focus() {
            painter.rect_stroke(rect, radius, Stroke::new(1.5, p.ring), StrokeKind::Inside);
        }
        let color = if on {
            p.accent_text
        } else if hovered {
            p.text
        } else if side < 20.0 {
            p.text_faint
        } else {
            p.text_muted
        };
        let color = if enabled {
            color
        } else {
            color.gamma_multiply(0.45)
        };
        // 28 → 16, 24 → 14, 22 → 13, 18 → 11.
        let size = if side >= 26.0 { 16.0 } else if side >= 23.0 { 14.0 } else if side >= 21.0 { 13.0 } else { 11.0 };
        painter.text(
            rect.center(),
            Align2::CENTER_CENTER,
            icon,
            style::icon_font(size),
            color,
        );
    }
    resp
}

/// Height of a segmented control.
pub const SEGMENTED_H: f32 = 28.0;
/// Room each side of a segment's content, and between segments.
const SEGMENT_PAD: f32 = 10.0;
const SEGMENT_GAP: f32 = 2.0;

/// One option of a segmented control: an optional icon, its word, an optional count after it
/// (in its own colour) and a tooltip.
#[derive(Clone)]
pub struct Seg<'a> {
    pub icon: Option<&'a str>,
    /// An agent's logo before the word (in place of an icon).
    pub agent: Option<tc_core::agent::AgentKind>,
    pub text: &'a str,
    pub count: Option<(String, Color32)>,
    pub tip: &'a str,
}

impl<'a> Seg<'a> {
    pub fn new(text: &'a str, tip: &'a str) -> Self {
        Self { icon: None, agent: None, text, count: None, tip }
    }
    pub fn agent(mut self, agent: tc_core::agent::AgentKind) -> Self {
        if super::agent_logo::has(agent) {
            self.agent = Some(agent);
        }
        self
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
    style::medium(12.0)
}

/// Width of a segment's content (icon, word, count).
fn segment_content(ui: &Ui, s: &Seg) -> f32 {
    let w = |t: &str, f: FontId| ui.painter().layout_no_wrap(t.to_string(), f, Color32::PLACEHOLDER).size().x;
    let mut x = w(s.text, segment_font());
    if s.agent.is_some() {
        x += 12.0 + 6.0;
    } else if let Some(i) = s.icon {
        x += w(i, style::icon_font(13.0)) + 5.0;
    }
    if let Some((n, _)) = &s.count {
        x += w(n, style::mono(10.5)) + 6.0;
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
    let dark = ui.visuals().dark_mode;
    let font = segment_font();
    // A sunken track (no line), the selected segment raised out of it.
    ui.painter().rect_filled(rect, CornerRadius::same(8), p.inset);
    let mut out = None;
    for (i, (r, s)) in segments.iter().zip(options).enumerate() {
        let resp = ui.interact(*r, id.with(("option", i)), Sense::click());
        let resp = if s.tip.is_empty() { resp } else { resp.on_hover_text(s.tip) };
        let selected = i == on;
        if resp.hovered() && !selected {
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        }
        resp.widget_info(|| WidgetInfo::selected(WidgetType::Button, true, selected, s.text));
        if selected {
            ui.painter().rect_filled(r.translate(vec2(0.0, 1.0)), CornerRadius::same(6), Color32::from_black_alpha(if dark { 64 } else { 18 }));
            ui.painter().rect(*r, CornerRadius::same(6), p.surface, Stroke::new(1.0, p.border), StrokeKind::Inside);
        } else if resp.hovered() {
            ui.painter().rect_filled(*r, CornerRadius::same(6), p.hover);
        }
        let color = if selected || resp.hovered() { p.text } else { p.text_muted };
        let content = segment_content(ui, s);
        let seg_painter = ui.painter().with_clip_rect(r.intersect(ui.clip_rect()));
        let mut x = (r.center().x - content / 2.0).max(r.left() + 4.0);
        let cy = r.center().y;
        if let Some(k) = s.agent {
            super::agent_logo::paint(&seg_painter, Rect::from_min_size(egui::pos2(x, cy - 6.0), vec2(12.0, 12.0)), k, p.text, !(selected || resp.hovered()));
            x += 12.0 + 6.0;
        } else if let Some(icon) = s.icon {
            let g = seg_painter.layout_no_wrap(icon.to_string(), style::icon_font(13.0), color);
            seg_painter.galley(egui::pos2(x, cy - g.size().y / 2.0), g.clone(), color);
            x += g.size().x + 5.0;
        }
        let g = seg_painter.layout_no_wrap(s.text.to_string(), font.clone(), color);
        let w = g.size().x;
        seg_painter.galley(egui::pos2(x, cy - g.size().y / 2.0), g, color);
        x += w + 6.0;
        if let Some((n, c)) = &s.count {
            // A count in its own colour (amber for waiting), else quiet: ghost, faint when on.
            let c = if *c == p.text_faint { if selected { p.text_faint } else { p.text_ghost } } else { *c };
            let g = seg_painter.layout_no_wrap(n.clone(), style::mono(10.5), c);
            seg_painter.galley(egui::pos2(x, cy - g.size().y / 2.0), g, c);
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
                if resp.hovered() && !selected {
                    ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                }
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
    let w = kbd_width(ui.painter(), keys);
    let (rect, resp) = ui.allocate_exact_size(vec2(w, KBD_H), Sense::hover());
    if ui.is_rect_visible(rect) {
        paint_kbd_text(ui.painter(), rect.left_center(), keys, &p);
    }
    resp
}

/// Height of a key cap.
pub const KBD_H: f32 = 18.0;

/// How wide `paint_kbd_text` draws `keys`.
pub fn kbd_width(painter: &egui::Painter, keys: &str) -> f32 {
    (painter.layout_no_wrap(keys.to_string(), style::mono(11.0), Color32::PLACEHOLDER).size().x + 10.0).max(KBD_H)
}

/// Paints a key cap with its text at `left_center`; returns its width.
pub fn paint_kbd_text(painter: &egui::Painter, left_center: egui::Pos2, keys: &str, p: &style::Palette) -> f32 {
    paint_kbd_sized(painter, left_center, keys, p, KBD_H)
}

/// [`paint_kbd_text`] of another height (16 in the status bar).
pub fn paint_kbd_sized(painter: &egui::Painter, left_center: egui::Pos2, keys: &str, p: &style::Palette, h: f32) -> f32 {
    let size = if h < KBD_H { 10.0 } else { 11.0 };
    let galley = painter.layout_no_wrap(keys.to_string(), style::mono(size), p.text_muted);
    let rect = Rect::from_min_size(egui::pos2(left_center.x, left_center.y - h / 2.0), vec2((galley.size().x + 10.0).max(h), h));
    paint_kbd(painter, rect, p);
    painter.galley(rect.center() - galley.size() / 2.0, galley, p.text_muted);
    rect.width()
}

/// The key cap behind a shortcut hint: a faint fill and a hairline.
pub fn paint_kbd(painter: &egui::Painter, rect: Rect, p: &style::Palette) {
    painter.rect(
        rect,
        CornerRadius::same(4),
        p.hover,
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
        .corner_radius(CornerRadius::same(style::RADIUS_CARD))
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
        // A quiet icon tile, a title, one sentence, at most one action (Slate empty state).
        let (rect, _) = ui.allocate_exact_size(Vec2::splat(48.0), Sense::hover());
        ui.painter().rect(rect, CornerRadius::same(12), p.hover, Stroke::new(1.0, p.border), StrokeKind::Inside);
        ui.painter().text(rect.center(), Align2::CENTER_CENTER, icon, style::icon_font(22.0), p.text_muted);
        ui.add_space(10.0);
        ui.label(RichText::new(title).font(style::semibold(style::HEADING)).color(p.text));
        ui.add_space(2.0);
        ui.scope(|ui| {
            ui.set_max_width(420.0);
            ui.label(RichText::new(body).font(egui::FontId::proportional(style::SMALL)).color(p.text_muted));
        });
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
pub const PILL_H: f32 = 20.0;

/// How wide a pill with `icon` and `text` is.
pub fn pill_width(painter: &egui::Painter, icon: Option<&str>, text: &str) -> f32 {
    let w = painter.layout_no_wrap(text.to_string(), style::medium(12.0), Color32::PLACEHOLDER).size().x;
    let i = icon.map_or(0.0, |i| painter.layout_no_wrap(i.to_string(), style::icon_fill(12.0), Color32::PLACEHOLDER).size().x + 5.0);
    w + i + if icon.is_some() { 14.0 } else { 16.0 }
}

/// Paints a pill (soft fill, coloured icon and text) with its left edge at `left_center`;
/// returns its rectangle. Status icons (bell, check, cross) are drawn filled.
pub fn paint_pill(painter: &egui::Painter, left_center: egui::Pos2, icon: Option<&str>, text: &str, fg: Color32, bg: Color32) -> Rect {
    let w = pill_width(painter, icon, text);
    let rect = Rect::from_min_size(egui::pos2(left_center.x, left_center.y - PILL_H / 2.0), vec2(w, PILL_H));
    painter.rect_filled(rect, CornerRadius::same(10), bg);
    let mut x = rect.left() + if icon.is_some() { 6.0 } else { 8.0 };
    if let Some(i) = icon {
        let g = painter.layout_no_wrap(i.to_string(), style::icon_fill(12.0), fg);
        painter.galley(egui::pos2(x, rect.center().y - g.size().y / 2.0), g.clone(), fg);
        x += g.size().x + 5.0;
    }
    let g = painter.layout_no_wrap(text.to_string(), style::medium(12.0), fg);
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

/// How wide [`paint_state_pill`] draws.
pub fn state_pill_width(painter: &egui::Painter, s: tc_core::status::Status, text: &str) -> f32 {
    let w = painter.layout_no_wrap(text.to_string(), style::medium(12.0), Color32::PLACEHOLDER).size().x;
    let glyph = if super::status::has_glyph(s) { 12.0 + 5.0 } else { 0.0 };
    w + glyph + 14.0
}

/// A pane's status as a pill: its glyph (the lattice while it runs) and `text` in its colour,
/// on the colour's tint; a quiet status on the hover fill. Returns its rectangle.
pub fn paint_state_pill(painter: &egui::Painter, left_center: egui::Pos2, s: tc_core::status::Status, text: &str, p: &style::Palette, dark: bool) -> Rect {
    let w = state_pill_width(painter, s, text);
    let rect = Rect::from_min_size(egui::pos2(left_center.x, left_center.y - PILL_H / 2.0), vec2(w, PILL_H));
    let c = super::status::color(s, p);
    let quiet = matches!(s, tc_core::status::Status::Idle | tc_core::status::Status::Ended | tc_core::status::Status::Unknown);
    painter.rect_filled(rect, CornerRadius::same(10), if quiet { p.accent_soft } else { style::tint(c, dark) });
    let mut x = rect.left() + 6.0;
    if super::status::has_glyph(s) {
        super::status::paint_glyph(painter, egui::pos2(x + 6.0, rect.center().y), s, p, 12.0);
        x += 12.0 + 5.0;
    }
    let fg = if quiet { p.text_muted } else { c };
    let g = painter.layout_no_wrap(text.to_string(), style::medium(12.0), fg);
    painter.galley(egui::pos2(x, rect.center().y - g.size().y / 2.0), g, fg);
    rect
}

/// A count in a filled capsule (16 high), its right edge at `right_center`; returns its
/// rectangle.
pub fn paint_badge(painter: &egui::Painter, right_center: egui::Pos2, text: &str, fill: Color32, fg: Color32, height: f32) -> Rect {
    let g = painter.layout_no_wrap(text.to_string(), style::mono_semibold(if height < 16.0 { 9.5 } else { 10.0 }), fg);
    let w = (g.size().x + 8.0).max(height);
    let rect = Rect::from_min_size(egui::pos2(right_center.x - w, right_center.y - height / 2.0), vec2(w, height));
    painter.rect_filled(rect, CornerRadius::same((height / 2.0) as u8), fill);
    painter.galley(rect.center() - g.size() / 2.0, g, fg);
    rect
}

/// The text colour on a status-filled badge: dark on amber and green in the dark theme, white
/// otherwise.
pub fn badge_text(fill: Color32, p: &style::Palette, dark: bool) -> Color32 {
    if dark && (fill == p.warn || fill == p.ok) { Color32::from_rgb(0x11, 0x11, 0x11) } else { Color32::WHITE }
}

/// A section's title ("Projects"), in sentence case; returns the galley to place.
pub fn section_galley(painter: &egui::Painter, text: &str, color: Color32) -> std::sync::Arc<egui::Galley> {
    painter.layout_no_wrap(text.to_string(), style::medium(style::LABEL), color)
}

/// A key hint as plain quiet text (`⌘J`), as menus and rows show them.
pub fn keys_galley(painter: &egui::Painter, keys: &str, color: Color32) -> std::sync::Arc<egui::Galley> {
    painter.layout_no_wrap(keys.to_string(), style::mono(10.5), color)
}

// ---------------------------------------------------------------------------------------------
// Lattice, project tiles, chips
// ---------------------------------------------------------------------------------------------

/// The lattice: the 3 × 3 grid of the app icon, `size` across (10, 14 or 22), in `color`.
/// Running, a light travels along the icon's "</" path (dots 2 → 4 → 8, then 3 → 5) once every
/// 1.2 s, and the caller's frame is asked again; still, the path shows brighter.
pub fn paint_lattice(painter: &egui::Painter, center: egui::Pos2, size: f32, color: Color32, running: bool) {
    let dot = (size * 0.2).max(2.0);
    let step = (size - dot) / 2.0;
    // Row-major: 0 1 2 / 3 4 5 / 6 7 8. The path: 1, 3, 7, then 2, 4 (0-based).
    const PATH: [(usize, f64); 5] = [(1, 0.0), (3, 0.15), (7, 0.3), (2, 0.6), (4, 0.75)];
    let t = painter.ctx().input(|i| i.time);
    for i in 0..9 {
        let (cx, cy) = ((i % 3) as f32 - 1.0, (i / 3) as f32 - 1.0);
        let on_path = PATH.iter().find(|(d, _)| *d == i);
        let alpha = if running {
            match on_path {
                Some((_, delay)) => {
                    let phase = ((t - delay).rem_euclid(1.2) / 1.2) as f32;
                    if phase < 0.35 { 1.0 - 0.78 * phase / 0.35 } else { 0.22 }
                }
                None => 0.22,
            }
        } else if on_path.is_some_and(|(d, _)| *d != 4) {
            0.9
        } else {
            0.45
        };
        let c = egui::pos2(center.x + cx * step, center.y + cy * step);
        painter.rect_filled(Rect::from_center_size(c, Vec2::splat(dot)), CornerRadius::same((dot / 2.0) as u8), color.gamma_multiply(alpha));
    }
    if running {
        painter.ctx().request_repaint_after(std::time::Duration::from_millis(33));
    }
}

/// A project's identity tile (the Project avatar): its letter in its hue, on the hue's tint,
/// `rect` square (16 to 32): 20 → radius 5, 16 → 4.
pub fn paint_project_tile(painter: &egui::Painter, rect: Rect, letters: &str, color: Color32, dark: bool) {
    let side = rect.width();
    let radius = if side >= 30.0 { 8 } else if side >= 24.0 { 6 } else if side >= 19.0 { 5 } else { 4 };
    let ink = crate::project::hue_for_theme(color, dark);
    let fill = style::soft(ink, if dark { 0.14 } else { 0.10 });
    painter.rect_filled(rect, CornerRadius::same(radius), fill);
    let font = if side >= 30.0 { 13.0 } else if side >= 24.0 { 12.0 } else if side >= 19.0 { 11.0 } else { 10.0 };
    painter.text(rect.center() + vec2(0.0, 0.5), Align2::CENTER_CENTER, letters, style::mono_semibold(font), ink);
}

/// A chip in the top bar (branch, changes, server): an icon, text, a hover fill. `mono` sets
/// the text in the data font. Returns the response.
pub fn chip(ui: &mut Ui, icon: Option<&str>, text: &str, mono: bool, trailing: Option<&str>) -> Response {
    let p = style::pal(ui);
    let font = if mono { style::mono(11.5) } else { egui::FontId::proportional(12.0) };
    let g = ui.painter().layout_no_wrap(text.to_string(), font, p.text_muted);
    let iw = icon.map_or(0.0, |_| 13.0 + 6.0);
    let tw = trailing.map_or(0.0, |_| 6.0 + 11.0);
    let (rect, resp) = ui.allocate_exact_size(vec2(8.0 + iw + g.size().x + tw + 8.0, 26.0), Sense::click());
    if ui.is_rect_visible(rect) {
        let painter = ui.painter();
        if resp.hovered() {
            painter.rect_filled(rect, CornerRadius::same(RADIUS), p.hover);
        }
        let mut x = rect.left() + 8.0;
        if let Some(i) = icon {
            painter.text(egui::pos2(x + 6.5, rect.center().y), Align2::CENTER_CENTER, i, style::icon_font(13.0), p.text_faint);
            x += 13.0 + 6.0;
        }
        let c = if resp.hovered() { p.text } else { p.text_muted };
        let gw = g.size().x;
        painter.galley(egui::pos2(x, rect.center().y - g.size().y / 2.0), g, c);
        if let Some(t) = trailing {
            painter.text(egui::pos2(x + gw + 6.0 + 5.5, rect.center().y), Align2::CENTER_CENTER, t, style::icon_font(11.0), p.text_ghost);
        }
    }
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// A vertical hairline between groups of the top bar (16 high).
pub fn vsep(ui: &mut Ui) {
    let p = style::pal(ui);
    let (rect, _) = ui.allocate_exact_size(vec2(9.0, 16.0), Sense::hover());
    ui.painter().vline(rect.center().x, rect.y_range(), Stroke::new(1.0, p.border_strong));
}

// ---------------------------------------------------------------------------------------------
// Form controls
// ---------------------------------------------------------------------------------------------

/// An on/off switch (26 × 16). Clicking flips `on`; the response says it changed.
pub fn switch(ui: &mut Ui, on: &mut bool) -> Response {
    let p = style::pal(ui);
    let (rect, mut resp) = ui.allocate_exact_size(vec2(26.0, 16.0), Sense::click());
    if resp.clicked() {
        *on = !*on;
        resp.mark_changed();
    }
    let enabled = ui.is_enabled();
    if enabled && resp.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    resp.widget_info(|| WidgetInfo::selected(WidgetType::Checkbox, enabled, *on, ""));
    if ui.is_rect_visible(rect) {
        let t = ui.ctx().animate_bool_with_time(resp.id, *on, 0.12);
        let track = if *on { p.accent } else { p.border_heavy };
        let fade = |c: Color32| if enabled { c } else { c.gamma_multiply(0.45) };
        let painter = ui.painter();
        painter.rect_filled(rect, CornerRadius::same(8), fade(track));
        let x = egui::lerp(rect.left() + 8.0..=rect.right() - 8.0, t);
        let c = egui::pos2(x, rect.center().y);
        painter.circle_filled(c + vec2(0.0, 0.5), 6.0, Color32::from_black_alpha(60));
        painter.circle_filled(c, 6.0, fade(Color32::WHITE));
        if resp.has_focus() {
            painter.rect_stroke(rect.expand(2.0), CornerRadius::same(10), Stroke::new(2.0, p.ring), StrokeKind::Outside);
        }
    }
    resp
}

/// A number with − and + on either side (28 high). Returns −1 or +1 when one is clicked.
pub fn stepper(ui: &mut Ui, text: &str) -> i32 {
    let p = style::pal(ui);
    let g = ui.painter().layout_no_wrap(text.to_string(), style::mono(12.0), p.text);
    let w = 26.0 * 2.0 + g.size().x + 18.0 + 2.0;
    let (rect, resp) = ui.allocate_exact_size(vec2(w, style::CONTROL_H), Sense::hover());
    let mut out = 0;
    if ui.is_rect_visible(rect) || resp.hovered() {
        let painter = ui.painter().clone();
        let fill = if ui.visuals().dark_mode { p.inset } else { Color32::WHITE };
        painter.rect(rect, CornerRadius::same(RADIUS), fill, Stroke::new(1.0, p.border_strong), StrokeKind::Inside);
        let minus = Rect::from_min_size(rect.min + vec2(1.0, 1.0), vec2(26.0, rect.height() - 2.0));
        let plus = Rect::from_min_size(egui::pos2(rect.right() - 27.0, rect.top() + 1.0), vec2(26.0, rect.height() - 2.0));
        for (r, glyph, step, salt) in [(minus, super::icons::MINUS, -1, "minus"), (plus, super::icons::PLUS, 1, "plus")] {
            let b = ui.interact(r, resp.id.with(salt), Sense::click());
            if b.hovered() {
                ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                painter.rect_filled(r, CornerRadius::same(RADIUS - 1), p.hover);
            }
            painter.text(r.center(), Align2::CENTER_CENTER, glyph, style::icon_font(12.0), if b.hovered() { p.text } else { p.text_faint });
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
    let g = ui.painter().layout_no_wrap(current.to_string(), egui::FontId::proportional(style::BODY), p.text);
    let w = (g.size().x + 10.0 + 8.0 + 11.0 + 8.0).max(min_width);
    let (rect, resp) = ui.allocate_exact_size(vec2(w, style::CONTROL_H), Sense::click());
    let resp = resp.on_hover_cursor(egui::CursorIcon::PointingHand);
    let id = ui.id().with(id_salt);
    if ui.is_rect_visible(rect) {
        let dark = ui.visuals().dark_mode;
        let fill = if resp.hovered() { style::over(p.surface, if dark { Color32::WHITE } else { Color32::BLACK }, 0.035) } else { p.surface };
        ui.painter().rect(rect, CornerRadius::same(RADIUS + 1), fill, Stroke::new(1.0, p.border_strong), StrokeKind::Inside);
        ui.painter().galley(egui::pos2(rect.left() + 10.0, rect.center().y - g.size().y / 2.0), g, p.text);
        ui.painter().text(egui::pos2(rect.right() - 9.0, rect.center().y), Align2::RIGHT_CENTER, super::icons::CARET_UP_DOWN, style::icon_font(12.0), p.text_faint);
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
    let font = egui::FontId::proportional(if small { 12.0 } else { 13.0 });
    let (rect, outer) = ui.allocate_exact_size(vec2(width, h), Sense::click());
    if outer.clicked() {
        ui.ctx().memory_mut(|m| m.request_focus(id));
    }
    let focused = ui.ctx().memory(|m| m.has_focus(id));
    let painter = ui.painter();
    if focused {
        painter.rect_stroke(rect.expand(3.0), CornerRadius::same(RADIUS + 4), Stroke::new(3.0, style::soft(p.accent, 0.25)), StrokeKind::Inside);
    }
    painter.rect(rect, CornerRadius::same(RADIUS + 1), p.inset, Stroke::new(1.0, if focused { p.accent } else { p.border_strong }), StrokeKind::Inside);
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
    ui.spacing_mut().item_spacing = vec2(0.0, 1.0);
    ui.set_min_width(232.0);
}

/// A menu row: an icon (or room for one), a label, a key hint at the right. Hovered, it fills
/// with the accent. `danger` draws it in the error colour.
pub fn menu_item(ui: &mut Ui, icon: Option<&str>, label: &str, keys: Option<&str>, danger: bool) -> Response {
    let p = style::pal(ui);
    let enabled = ui.is_enabled();
    let label_g = ui.painter().layout_no_wrap(label.to_string(), egui::FontId::proportional(13.0), Color32::PLACEHOLDER);
    let keys_g = keys.filter(|k| !k.is_empty()).map(|k| keys_galley(ui.painter(), k, Color32::PLACEHOLDER));
    let want = 8.0 + 15.0 + 9.0 + label_g.size().x + keys_g.as_ref().map_or(0.0, |g| g.size().x + 18.0) + 8.0;
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width().max(want), 28.0), Sense::click());
    if enabled && resp.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    resp.widget_info(|| WidgetInfo::labeled(WidgetType::Button, enabled, label));
    if ui.is_rect_visible(rect) {
        let hovered = enabled && resp.hovered();
        let painter = ui.painter();
        if hovered {
            painter.rect_filled(rect, CornerRadius::same(6), p.subtle);
        }
        let fade = |c: Color32| if enabled { c } else { c.gamma_multiply(0.45) };
        let (text_c, icon_c, keys_c) = if danger {
            (p.err, p.err, p.text_faint)
        } else if hovered {
            (p.text, p.text, p.text_faint)
        } else if !enabled {
            (p.text_ghost, p.text_ghost, p.text_ghost)
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
    ui.painter().hline(rect.x_range().shrink(6.0), rect.center().y, Stroke::new(1.0, p.border));
}

/// A small title over a group of menu rows.
pub fn menu_header(ui: &mut Ui, text: &str) {
    let p = style::pal(ui);
    let g = ui.painter().layout_no_wrap(text.to_string(), style::medium(style::LABEL), p.text_faint);
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), g.size().y + 10.0), Sense::hover());
    ui.painter().galley(egui::pos2(rect.left() + 8.0, rect.top() + 6.0), g, p.text_faint);
}

/// A menu row that opens a submenu (a caret at its right); show the submenu with
/// `egui::containers::menu::SubMenu::new().show(ui, &response, …)`.
pub fn submenu_item(ui: &mut Ui, icon: Option<&str>, label: &str) -> Response {
    let r = menu_item(ui, icon, label, None, false);
    let p = style::pal(ui);
    let c = if r.hovered() { p.text } else { p.text_faint };
    ui.painter().text(egui::pos2(r.rect.right() - 10.0, r.rect.center().y), Align2::CENTER_CENTER, super::icons::CARET_RIGHT, style::icon_font(11.0), c);
    r
}

// ---------------------------------------------------------------------------------------------
// ADE Design System Primitives (Linear / Zed / Raycast / Warp restraint)
// ---------------------------------------------------------------------------------------------

/// Compact semantic status badge (`Running`, `Rate-Limited`, `Cooldown`, `Idle`, `Error`).
pub fn status_badge(ui: &mut Ui, label: &str, color: Color32, mono: bool) -> Response {
    let dark = ui.visuals().dark_mode;
    let font = if mono { style::mono_medium(10.5) } else { style::medium(11.0) };
    let g = ui.painter().layout_no_wrap(label.to_string(), font, color);
    let (rect, resp) = ui.allocate_exact_size(vec2(g.size().x + 18.0, 20.0), Sense::hover());
    if ui.is_rect_visible(rect) {
        let bg = style::tint(color, dark);
        let stroke = Stroke::new(1.0, style::soft(color, if dark { 0.32 } else { 0.28 }));
        ui.painter().rect(rect, CornerRadius::same(5), bg, stroke, StrokeKind::Inside);
        ui.painter().circle_filled(egui::pos2(rect.left() + 7.5, rect.center().y), 2.5, color);
        ui.painter().galley(egui::pos2(rect.left() + 13.0, rect.center().y - g.size().y / 2.0), g, color);
    }
    resp
}

/// Crisp, optically centered vector checkmark (replaces fuzzy/off-center font glyph in checkboxes).
pub fn paint_check_mark(painter: &egui::Painter, center: egui::Pos2, size: f32, color: Color32) {
    let p1 = egui::pos2(center.x - size * 0.31, center.y + size * 0.03);
    let p2 = egui::pos2(center.x - size * 0.07, center.y + size * 0.27);
    let p3 = egui::pos2(center.x + size * 0.34, center.y - size * 0.24);
    let stroke = Stroke::new((size * 0.145).clamp(1.4, 2.0), color);
    painter.line_segment([p1, p2], stroke);
    painter.line_segment([p2, p3], stroke);
    painter.circle_filled(p2, stroke.width * 0.45, color);
}

/// Crisp Linear/macOS-grade checkbox (`15×15` or `16×16`) with optically centered vector checkmark.
pub fn paint_checkbox(painter: &egui::Painter, rect: Rect, checked: bool, hovered: bool, p: &style::Palette) {
    let radius = CornerRadius::same(4);
    if checked {
        painter.rect_filled(rect, radius, p.accent);
        painter.rect_stroke(
            rect,
            radius,
            Stroke::new(1.0, style::soft(Color32::WHITE, 0.16)),
            StrokeKind::Inside,
        );
        paint_check_mark(painter, rect.center(), rect.width() * 0.72, p.on_accent);
    } else {
        let fill = if hovered { p.hover } else { p.inset };
        let border = if hovered { p.text_faint } else { p.border_heavy };
        painter.rect(rect, radius, fill, Stroke::new(1.0, border), StrokeKind::Inside);
    }
}

/// Hairline quota / usage progress meter (`pct` in `0.0..=100.0`).
pub fn paint_quota_bar(painter: &egui::Painter, rect: Rect, pct: f32, color: Color32, p: &style::Palette) {
    let clamped = (pct / 100.0).clamp(0.0, 1.0);
    let r = CornerRadius::same((rect.height() / 2.0).round() as u8);
    painter.rect_filled(rect, r, p.subtle);
    if clamped > 0.0 {
        let fill = Rect::from_min_size(rect.min, vec2((rect.width() * clamped).max(3.0), rect.height()));
        painter.rect_filled(fill, r, color);
    }
}

/// Interactive account profile chip (`claude1 · 82%`) with crisp 1 px border and status dot.
pub fn profile_chip(
    ui: &mut Ui,
    name: &str,
    detail: Option<&str>,
    dot_color: Color32,
    active: bool,
) -> Response {
    let p = style::pal(ui);
    let name_g = ui.painter().layout_no_wrap(name.to_string(), style::mono_medium(11.0), p.text);
    let detail_g = detail.map(|d| ui.painter().layout_no_wrap(d.to_string(), style::mono(10.5), p.text_faint));
    let dw = detail_g.as_ref().map_or(0.0, |g| 6.0 + g.size().x);
    let w = 8.0 + 6.0 + 6.0 + name_g.size().x + dw + 5.0 + 9.0 + 7.0;
    let (rect, resp) = ui.allocate_exact_size(vec2(w, 22.0), Sense::click());
    if ui.is_rect_visible(rect) {
        let fill = if active { p.surface } else if resp.hovered() { p.hover } else { Color32::TRANSPARENT };
        let stroke = Stroke::new(1.0, if active || resp.hovered() { p.border_strong } else { p.border });
        ui.painter().rect(rect, CornerRadius::same(6), fill, stroke, StrokeKind::Inside);
        let cy = rect.center().y;
        let mut x = rect.left() + 8.0;
        ui.painter().circle_filled(egui::pos2(x + 3.0, cy), 3.0, dot_color);
        x += 6.0 + 6.0;
        let nw = name_g.size().x;
        ui.painter().galley(egui::pos2(x, cy - name_g.size().y / 2.0), name_g, p.text);
        x += nw;
        if let Some(dg) = detail_g {
            x += 6.0;
            let dgw = dg.size().x;
            ui.painter().galley(egui::pos2(x, cy - dg.size().y / 2.0), dg, p.text_faint);
            x += dgw;
        }
        ui.painter().text(egui::pos2(x + 5.0 + 4.5, cy), Align2::CENTER_CENTER, super::icons::CARET_DOWN, style::icon_font(9.0), p.text_faint);
    }
    if resp.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    resp
}

/// Tab button inside the secondary drawer header (`Terminals`, `Agent Logs`, `Problems`).
pub fn drawer_tab(ui: &mut Ui, icon: &str, label: &str, count: Option<(usize, Color32)>, active: bool) -> Response {
    let p = style::pal(ui);
    let text_c = if active { p.text } else { p.text_muted };
    let label_g = ui.painter().layout_no_wrap(label.to_string(), style::medium(12.0), text_c);
    let badge_g = count.filter(|(n, _)| *n > 0).map(|(n, c)| (ui.painter().layout_no_wrap(n.to_string(), style::mono(10.0), c), c));
    let bw = badge_g.as_ref().map_or(0.0, |(g, _)| 6.0 + g.size().x + 8.0);
    let w = 8.0 + 13.0 + 6.0 + label_g.size().x + bw + 8.0;
    let (rect, resp) = ui.allocate_exact_size(vec2(w, 24.0), Sense::click());
    if ui.is_rect_visible(rect) {
        if active {
            ui.painter().rect(rect, CornerRadius::same(6), p.surface, Stroke::new(1.0, p.border_strong), StrokeKind::Inside);
        } else if resp.hovered() {
            ui.painter().rect_filled(rect, CornerRadius::same(6), p.hover);
        }
        let cy = rect.center().y;
        let mut x = rect.left() + 8.0;
        ui.painter().text(egui::pos2(x + 6.5, cy), Align2::CENTER_CENTER, icon, style::icon_font(12.5), if active { p.text } else { p.text_faint });
        x += 13.0 + 6.0;
        let lw = label_g.size().x;
        ui.painter().galley(egui::pos2(x, cy - label_g.size().y / 2.0), label_g, if active || resp.hovered() { p.text } else { p.text_muted });
        if let Some((bg, bc)) = badge_g {
            x += lw + 6.0;
            let pill = Rect::from_center_size(egui::pos2(x + (bg.size().x + 8.0) / 2.0, cy), vec2(bg.size().x + 8.0, 16.0));
            ui.painter().rect_filled(pill, CornerRadius::same(4), style::tint(bc, ui.visuals().dark_mode));
            ui.painter().galley(egui::pos2(pill.center().x - bg.size().x / 2.0, cy - bg.size().y / 2.0), bg, bc);
        }
    }
    if resp.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    resp
}

// ---------------------------------------------------------------------------------------------
// Notice: the one banner of panes, tiles and pages (Slate)
// ---------------------------------------------------------------------------------------------

/// What a notice is about: its colour and default glyph.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tone {
    /// An agent asks (permission, question, plan): amber.
    Input,
    /// Something failed: red.
    Failed,
    /// A turn or a command finished: green.
    Done,
    /// A session to resume, an agent using a page: the accent.
    Info,
    /// A usage limit: orange.
    Limited,
    /// A shell ended, the page is yours: the raised surface.
    Neutral,
}

impl Tone {
    pub fn color(self, p: &style::Palette) -> Color32 {
        match self {
            Tone::Input => p.warn,
            Tone::Failed => p.err,
            Tone::Done => p.ok,
            Tone::Info => p.accent_text,
            Tone::Limited => p.rate_limited,
            Tone::Neutral => p.text_muted,
        }
    }
    pub fn glyph(self) -> &'static str {
        use super::icons as i;
        match self {
            Tone::Input => i::CHAT_CIRCLE_DOTS,
            Tone::Failed => i::X_CIRCLE,
            Tone::Done => i::CHECK_CIRCLE,
            Tone::Info => i::CLOCK_COUNTER_CLOCKWISE,
            Tone::Limited => i::HOURGLASS_MEDIUM,
            Tone::Neutral => i::POWER,
        }
    }
    fn filled(self) -> bool {
        !matches!(self, Tone::Info | Tone::Neutral)
    }
}

/// What the user did with a notice this frame.
#[derive(Clone, Copy, Debug)]
pub struct NoticeOut {
    pub primary: bool,
    pub secondary: bool,
    pub dismissed: bool,
    pub rect: Rect,
}

/// Height of a notice with a second line (title + body), and with a title alone.
pub const NOTICE_H: f32 = 48.0;
pub const NOTICE_H_SHORT: f32 = 36.0;

/// The Notice: a glyph, a title, an optional mono second line, then up to two buttons (Secondary,
/// then Primary with its key) and a dismiss ×, on the tone's tint, `width` wide. Buttons that
/// don't fit drop out from the left (the primary stays).
#[allow(clippy::too_many_arguments)]
pub fn notice(
    ui: &mut Ui,
    width: f32,
    tone: Tone,
    glyph: Option<&str>,
    title: &str,
    body: Option<&str>,
    primary: Option<(&str, Option<&str>)>,
    secondary: Option<&str>,
    dismiss: bool,
) -> NoticeOut {
    let p = style::pal(ui);
    let dark = ui.visuals().dark_mode;
    let h = if body.is_some() { NOTICE_H } else { NOTICE_H_SHORT };
    let (rect, _) = ui.allocate_exact_size(vec2(width, h), Sense::hover());
    let mut out = NoticeOut { primary: false, secondary: false, dismissed: false, rect };
    if !ui.is_rect_visible(rect) {
        return out;
    }
    let c = tone.color(&p);
    let (fill, stroke) = if tone == Tone::Neutral {
        (p.surface, p.border_strong)
    } else {
        let t = style::tint(if tone == Tone::Info { p.accent } else { c }, dark);
        (t, t)
    };
    ui.painter().rect(rect, CornerRadius::same(9), fill, Stroke::new(1.0, stroke), StrokeKind::Inside);
    let cy = rect.center().y;
    // Right side, laid out from the right edge.
    let mut right = rect.right() - 8.0;
    let mut child = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(Rect::from_min_max(egui::pos2(rect.left() + 40.0, rect.top()), egui::pos2(right, rect.bottom())))
            .layout(egui::Layout::right_to_left(egui::Align::Center)),
    );
    child.spacing_mut().item_spacing.x = 6.0;
    if dismiss {
        let r = icon_button_ex(&mut child, super::icons::X, crate::i18n::t("Dismiss"), false, 24.0);
        out.dismissed = r.clicked();
        right -= 24.0 + 6.0;
    }
    let min_text = 160.0;
    if let Some((label, keys)) = primary {
        let w = button_width(&child, None, label, keys, true);
        if right - w - (rect.left() + 40.0) > min_text * 0.6 {
            out.primary = button_keys(&mut child, Kind::Primary, None, label, keys, true).clicked();
            right -= w + 6.0;
        }
    }
    if let Some(label) = secondary {
        let w = button_width(&child, None, label, None, true);
        if right - w - (rect.left() + 40.0) > min_text {
            out.secondary = button_ex(&mut child, Kind::Secondary, None, label, true).clicked();
            right -= w + 6.0;
        }
    }
    let painter = ui.painter();
    let gfont = if tone.filled() { style::icon_fill(16.0) } else { style::icon_font(16.0) };
    painter.text(egui::pos2(rect.left() + 12.0 + 8.0, cy), Align2::CENTER_CENTER, glyph.unwrap_or(tone.glyph()), gfont, c);
    let tx = rect.left() + 38.0;
    let max_w = (right - tx - 8.0).max(20.0);
    let one_line = |text: &str, font: FontId, color: Color32| {
        let mut job = egui::text::LayoutJob::single_section(text.to_string(), egui::TextFormat::simple(font, color));
        job.wrap = egui::text::TextWrapping { max_width: max_w, max_rows: 1, break_anywhere: true, overflow_character: Some('…') };
        painter.layout_job(job)
    };
    let tg = one_line(title, style::medium(style::SMALL), p.text);
    if let Some(b) = body {
        let bg = one_line(b, style::mono(style::DATA), p.text_muted);
        let total = tg.size().y + 2.0 + bg.size().y;
        let top = cy - total / 2.0;
        let th = tg.size().y;
        painter.galley(egui::pos2(tx, top), tg, p.text);
        painter.galley(egui::pos2(tx, top + th + 2.0), bg, p.text_muted);
    } else {
        painter.galley(egui::pos2(tx, cy - tg.size().y / 2.0), tg, p.text);
    }
    out
}
