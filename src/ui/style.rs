//! Design tokens (colors, type, spacing) and the egui style built from them. Shared components
//! (buttons, tabs, empty states) are in `kit.rs`, the dialogs' parts in `dialogs.rs`.
//!
//! Every screen draws with these tokens: [`Palette`] for colors (light and dark), [`medium`],
//! [`semibold`] and [`icon_font`] for type. The accent is the blue of the app icon.

use eframe::egui::{
    self, Color32, CornerRadius, FontFamily, FontId, Margin, Shadow, Stroke, Theme,
    ThemePreference, Ui, vec2,
};

use tc_term::fonts;
use tc_core::model::ThemeMode;

/// Colors of one theme.
#[derive(Clone, Copy, Debug)]
pub struct Palette {
    /// Content area: the terminal's surroundings, file lists, the welcome screen.
    pub canvas: Color32,
    /// Window chrome: sidebar, tab strip, status bar.
    pub chrome: Color32,
    /// Raised surfaces: inputs, cards, dialogs, menus.
    pub surface: Color32,
    /// Hovered rows and quiet button fills.
    pub hover: Color32,
    /// Pressed state and subtle fills (segmented control track, count pills).
    pub subtle: Color32,
    pub border: Color32,
    pub border_strong: Color32,
    pub text: Color32,
    pub text_muted: Color32,
    pub text_faint: Color32,
    pub accent: Color32,
    pub accent_hover: Color32,
    /// Text on an accent fill.
    pub on_accent: Color32,
    /// Background of selected rows and "on" toggles.
    pub accent_soft: Color32,
    /// Accent-colored text (readable on both the canvas and `accent_soft`).
    pub accent_text: Color32,
    pub ok: Color32,
    pub warn: Color32,
    pub err: Color32,
    pub info: Color32,
    /// Folder icons in file lists and the sidebar.
    pub folder: Color32,
}

// Neutral greys (no blue or violet cast), one quiet blue taken from the app icon for focus,
// selection and the main action, and status colours a little below full saturation.
const LIGHT: Palette = Palette {
    // The background of the "TernaryCode Light" terminal scheme, so the terminal blends in.
    canvas: Color32::from_rgb(0xfb, 0xfb, 0xfb),
    chrome: Color32::from_rgb(0xf3, 0xf3, 0xf3),
    surface: Color32::from_rgb(0xff, 0xff, 0xff),
    hover: Color32::from_rgb(0xea, 0xea, 0xea),
    subtle: Color32::from_rgb(0xe3, 0xe3, 0xe3),
    border: Color32::from_rgb(0xe0, 0xe0, 0xe0),
    border_strong: Color32::from_rgb(0xcb, 0xcb, 0xcb),
    text: Color32::from_rgb(0x1d, 0x1d, 0x1d),
    text_muted: Color32::from_rgb(0x5e, 0x5e, 0x5e),
    text_faint: Color32::from_rgb(0x8e, 0x8e, 0x8e),
    accent: Color32::from_rgb(0x25, 0x63, 0xc9),
    accent_hover: Color32::from_rgb(0x1d, 0x52, 0xae),
    on_accent: Color32::WHITE,
    accent_soft: Color32::from_rgb(0xe4, 0xe4, 0xe4),
    accent_text: Color32::from_rgb(0x1f, 0x5a, 0xbd),
    ok: Color32::from_rgb(0x1f, 0x84, 0x4b),
    warn: Color32::from_rgb(0xa6, 0x66, 0x00),
    err: Color32::from_rgb(0xc8, 0x32, 0x2e),
    info: Color32::from_rgb(0x25, 0x63, 0xc9),
    folder: Color32::from_rgb(0x6c, 0x78, 0x88),
};

const DARK: Palette = Palette {
    // The background of the "TernaryCode Dark" terminal scheme, so the terminal blends in.
    canvas: Color32::from_rgb(0x23, 0x23, 0x23),
    chrome: Color32::from_rgb(0x1c, 0x1c, 0x1c),
    surface: Color32::from_rgb(0x2c, 0x2c, 0x2c),
    hover: Color32::from_rgb(0x30, 0x30, 0x30),
    subtle: Color32::from_rgb(0x37, 0x37, 0x37),
    border: Color32::from_rgb(0x32, 0x32, 0x32),
    border_strong: Color32::from_rgb(0x45, 0x45, 0x45),
    text: Color32::from_rgb(0xe6, 0xe6, 0xe6),
    text_muted: Color32::from_rgb(0xa4, 0xa4, 0xa4),
    text_faint: Color32::from_rgb(0x76, 0x76, 0x76),
    accent: Color32::from_rgb(0x3d, 0x7b, 0xd9),
    accent_hover: Color32::from_rgb(0x52, 0x8c, 0xe2),
    on_accent: Color32::WHITE,
    accent_soft: Color32::from_rgb(0x36, 0x36, 0x36),
    accent_text: Color32::from_rgb(0x7f, 0xae, 0xec),
    ok: Color32::from_rgb(0x5f, 0xb8, 0x7e),
    warn: Color32::from_rgb(0xd6, 0xa2, 0x47),
    err: Color32::from_rgb(0xe8, 0x6b, 0x61),
    info: Color32::from_rgb(0x72, 0xa7, 0xe8),
    folder: Color32::from_rgb(0x9a, 0xa3, 0xae),
};

impl Palette {
    /// Barely visible fill for hovered list rows.
    pub fn faint_row(&self) -> Color32 {
        soft(self.hover, 0.6)
    }
}

/// Names in the file tree: a little softer than body text, so changed files stand out.
pub fn tree_text(p: &Palette) -> Color32 {
    soft(p.text, 0.84)
}

pub fn palette(dark: bool) -> Palette {
    if dark { DARK } else { LIGHT }
}

/// The palette of the theme `ui` is drawn with.
pub fn pal(ui: &Ui) -> Palette {
    palette(ui.visuals().dark_mode)
}

/// A color at `alpha` (0–1) opacity, for soft fills behind colored text and icons.
pub fn soft(c: Color32, alpha: f32) -> Color32 {
    Color32::from_rgba_unmultiplied(c.r(), c.g(), c.b(), (alpha.clamp(0.0, 1.0) * 255.0) as u8)
}

// ---------------------------------------------------------------------------------------------
// Type
// ---------------------------------------------------------------------------------------------

/// Body text size of the UI.
pub const BODY: f32 = 13.5;
/// Small text: details, hints, the status bar.
pub const SMALL: f32 = 11.5;

/// Inter Medium: labels that need a little weight (tabs, buttons, row titles).
pub fn medium(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name(fonts::MEDIUM.into()))
}

/// Inter SemiBold: titles and headings.
pub fn semibold(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name(fonts::SEMIBOLD.into()))
}

/// The icon font alone (icons also render inside ordinary text).
pub fn icon_font(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name(fonts::ICONS.into()))
}

// ---------------------------------------------------------------------------------------------
// Style
// ---------------------------------------------------------------------------------------------

pub const RADIUS: u8 = 6;
pub const RADIUS_LG: u8 = 10;

pub fn apply(ctx: &egui::Context, mode: ThemeMode) {
    ctx.set_theme(match mode {
        ThemeMode::System => ThemePreference::System,
        ThemeMode::Light => ThemePreference::Light,
        ThemeMode::Dark => ThemePreference::Dark,
    });
    for theme in [Theme::Dark, Theme::Light] {
        ctx.style_mut_of(theme, |style| {
            let dark = theme == Theme::Dark;
            let p = palette(dark);
            let s = &mut style.spacing;
            s.item_spacing = vec2(8.0, 6.0);
            s.button_padding = vec2(10.0, 4.0);
            s.interact_size = vec2(24.0, 26.0);
            s.window_margin = Margin::same(20);
            s.menu_margin = Margin::same(6);
            s.indent = 16.0;
            s.icon_width = 15.0;
            s.icon_width_inner = 9.0;
            s.icon_spacing = 6.0;
            s.combo_height = 320.0;
            s.tooltip_width = 360.0;
            s.menu_width = 220.0;
            s.scroll = egui::style::ScrollStyle::floating();
            s.scroll.bar_width = 8.0;
            s.scroll.floating_width = 3.0;
            s.scroll.floating_allocated_width = 0.0;
            // Selectable labels sense clicks and drags and would steal them from the rows,
            // buttons and tables they sit in; text that should be copyable opts in explicitly.
            style.interaction.selectable_labels = false;

            let v = &mut style.visuals;
            v.text_cursor.blink = false; // a blinking caret would repaint twice a second
            v.text_cursor.stroke = Stroke::new(1.5, p.accent);
            v.panel_fill = p.canvas;
            v.window_fill = p.surface;
            v.extreme_bg_color = p.surface;
            v.text_edit_bg_color = Some(p.surface);
            v.faint_bg_color = if dark {
                Color32::from_rgb(0x20, 0x20, 0x20)
            } else {
                Color32::from_rgb(0xf7, 0xf7, 0xf7)
            };
            v.code_bg_color = p.subtle;
            v.hyperlink_color = p.accent_text;
            v.warn_fg_color = p.warn;
            v.error_fg_color = p.err;
            v.override_text_color = None;
            v.weak_text_color = Some(p.text_muted);
            v.selection.bg_fill = soft(p.accent, if dark { 0.30 } else { 0.18 });
            // The text color of selected rows and items, and egui's ring around a focused text
            // field.
            v.selection.stroke = Stroke::new(1.0, p.accent_text);
            v.window_corner_radius = CornerRadius::same(RADIUS_LG);
            v.menu_corner_radius = CornerRadius::same(8);
            v.window_stroke = Stroke::new(1.0, p.border);
            v.window_shadow = Shadow {
                offset: [0, 10],
                blur: 30,
                spread: 0,
                color: Color32::from_black_alpha(if dark { 120 } else { 38 }),
            };
            v.popup_shadow = Shadow {
                offset: [0, 4],
                blur: 14,
                spread: 0,
                color: Color32::from_black_alpha(if dark { 100 } else { 28 }),
            };
            v.window_highlight_topmost = false;
            v.collapsing_header_frame = false;
            v.indent_has_left_vline = true;
            v.striped = false;
            v.slider_trailing_fill = true;
            v.handle_shape = egui::style::HandleShape::Circle;
            v.disabled_alpha = 0.45;

            let radius = CornerRadius::same(RADIUS);
            let w = &mut v.widgets;
            // Labels, separators, frames.
            w.noninteractive.bg_fill = p.surface;
            w.noninteractive.weak_bg_fill = p.surface;
            w.noninteractive.bg_stroke = Stroke::new(1.0, p.border);
            w.noninteractive.fg_stroke = Stroke::new(1.0, p.text);
            w.noninteractive.corner_radius = radius;
            // Buttons, inputs, checkboxes at rest: an outlined surface.
            w.inactive.bg_fill = p.subtle;
            w.inactive.weak_bg_fill = p.surface;
            w.inactive.bg_stroke = Stroke::new(1.0, p.border_strong);
            w.inactive.fg_stroke = Stroke::new(1.0, p.text);
            w.inactive.corner_radius = radius;
            w.inactive.expansion = 0.0;
            w.hovered.bg_fill = p.subtle;
            w.hovered.weak_bg_fill = p.hover;
            w.hovered.bg_stroke = Stroke::new(
                1.0,
                if dark {
                    p.border_strong
                } else {
                    Color32::from_rgb(0xbd, 0xbd, 0xbd)
                },
            );
            w.hovered.fg_stroke = Stroke::new(1.0, p.text);
            w.hovered.corner_radius = radius;
            w.hovered.expansion = 0.0;
            w.active.bg_fill = p.subtle;
            w.active.weak_bg_fill = p.subtle;
            w.active.bg_stroke = Stroke::new(1.0, p.accent);
            w.active.fg_stroke = Stroke::new(1.0, p.text);
            w.active.corner_radius = radius;
            w.active.expansion = 0.0;
            w.open.bg_fill = p.subtle;
            w.open.weak_bg_fill = p.hover;
            w.open.bg_stroke = Stroke::new(1.0, p.border_strong);
            w.open.fg_stroke = Stroke::new(1.0, p.text);
            w.open.corner_radius = radius;

            for (ts, font) in style.text_styles.iter_mut() {
                *font = match ts {
                    egui::TextStyle::Heading => semibold(18.0),
                    egui::TextStyle::Small => FontId::proportional(SMALL),
                    egui::TextStyle::Monospace => FontId::monospace(12.5),
                    _ => FontId::proportional(BODY),
                };
            }
        });
    }
}

/// Height of a form control (text field, button, menu).
pub const CONTROL_H: f32 = 28.0;

/// How wide `text` is in the body font.
pub fn text_width(ui: &Ui, text: &str) -> f32 {
    let font = egui::TextStyle::Body.resolve(ui.style());
    ui.painter()
        .layout_no_wrap(text.to_string(), font, Color32::WHITE)
        .size()
        .x
}
