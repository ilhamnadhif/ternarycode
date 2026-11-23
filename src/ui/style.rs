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
///
/// Two layers carry the window: the **base** (`chrome`: the window itself, under the sidebar,
/// the top bar and the status bar) and the **sheets** (`canvas`: panes, drawers, panels, Grid
/// tiles, where the work is; the terminal's background). What is raised above a sheet (the
/// selected row, the tab in front, a card, a dialog, a menu) has its own colour. Lines and
/// hover fills are translucent, so they work over any layer.
#[derive(Clone, Copy, Debug)]
pub struct Palette {
    /// The sheets: panes, drawers, panels, tiles (the terminal's background).
    pub canvas: Color32,
    /// The base: the window under the sidebar, the top bar, the status bar and between sheets.
    pub chrome: Color32,
    /// Raised over a sheet or the base: the selected row, the tab in front, cards, fields'
    /// buttons.
    pub surface: Color32,
    /// Dialogs and the palette.
    pub overlay: Color32,
    /// Menus, tooltips, toasts.
    pub menu: Color32,
    /// Tracks: segmented controls, switches, text fields.
    pub inset: Color32,
    /// Hovered rows and quiet button fills (translucent).
    pub hover: Color32,
    /// Pressed state and subtle fills (translucent).
    pub subtle: Color32,
    /// Hairlines (translucent).
    pub border: Color32,
    /// Lines of controls (translucent).
    pub border_strong: Color32,
    /// The strongest line: a switch's track when off, tree guides (translucent).
    pub border_heavy: Color32,
    /// The 1 px light along the top edge of raised things (translucent).
    pub highlight: Color32,
    pub text: Color32,
    pub text_muted: Color32,
    pub text_faint: Color32,
    /// The quietest text: counts beside a title, line numbers, separators.
    pub text_ghost: Color32,
    pub accent: Color32,
    pub accent_hover: Color32,
    /// Text on an accent fill.
    pub on_accent: Color32,
    /// Background of selected rows and "on" toggles (neutral).
    pub accent_soft: Color32,
    /// The accent, faint: the chosen row of a list in a dialog, a scope token.
    pub accent_tint: Color32,
    /// Accent-colored text (readable on both the canvas and `accent_soft`).
    pub accent_text: Color32,
    /// The ring around what has the keyboard.
    pub ring: Color32,
    /// Done, added.
    pub ok: Color32,
    /// Needs input.
    pub warn: Color32,
    /// Failed, removed.
    pub err: Color32,
    /// Running.
    pub info: Color32,
    /// Account rate-limited / quota exhausted.
    pub rate_limited: Color32,
    /// Account cooling down before retry.
    pub cooldown: Color32,
    /// Idle.
    pub idle: Color32,
    /// Folder icons in file lists and the sidebar.
    pub folder: Color32,
}

/// White at `a`/255 opacity (premultiplied).
const fn white(a: u8) -> Color32 {
    Color32::from_rgba_premultiplied(a, a, a, a)
}
/// Black at `a`/255 opacity.
const fn black(a: u8) -> Color32 {
    Color32::from_rgba_premultiplied(0, 0, 0, a)
}
/// `rgb` at `a`/255 opacity.
const fn rgba(r: u8, g: u8, b: u8, a: u8) -> Color32 {
    const fn m(c: u8, a: u8) -> u8 {
        ((c as u16 * a as u16 + 127) / 255) as u8
    }
    Color32::from_rgba_premultiplied(m(r, a), m(g, a), m(b, a), a)
}

// Slate (v3): neutral greys, hairlines, one accent (the blue of the app icon). Values are the
// `Theme` variables of design/TernaryCode Redesign.figcraft, Dark and Light modes.
const LIGHT: Palette = Palette {
    // The sheet is the "TernaryCode Light" terminal's background, so the terminal blends in.
    canvas: Color32::from_rgb(0xfb, 0xfb, 0xfc),
    chrome: Color32::from_rgb(0xed, 0xed, 0xef),
    surface: Color32::from_rgb(0xff, 0xff, 0xff),
    overlay: Color32::from_rgb(0xff, 0xff, 0xff),
    menu: Color32::from_rgb(0xff, 0xff, 0xff),
    inset: Color32::from_rgb(0xe9, 0xe9, 0xec),
    hover: black(10),
    subtle: black(18),
    border: black(15),
    border_strong: black(26),
    border_heavy: black(43),
    highlight: white(0),
    text: Color32::from_rgb(0x17, 0x17, 0x1a),
    text_muted: Color32::from_rgb(0x56, 0x56, 0x60),
    text_faint: Color32::from_rgb(0x80, 0x80, 0x8a),
    text_ghost: Color32::from_rgb(0xab, 0xab, 0xb3),
    accent: Color32::from_rgb(0x1f, 0x5f, 0xbf),
    accent_hover: Color32::from_rgb(0x1a, 0x52, 0xa8),
    on_accent: Color32::WHITE,
    accent_soft: black(13),
    accent_tint: rgba(0x1f, 0x5f, 0xbf, 26),
    accent_text: Color32::from_rgb(0x1a, 0x56, 0xb3),
    ring: rgba(0x1f, 0x5f, 0xbf, 102),
    ok: Color32::from_rgb(0x15, 0x80, 0x4e),
    warn: Color32::from_rgb(0xb2, 0x6a, 0x00),
    err: Color32::from_rgb(0xc7, 0x35, 0x2d),
    info: Color32::from_rgb(0x25, 0x63, 0xc9),
    rate_limited: Color32::from_rgb(0xc2, 0x56, 0x0a),
    cooldown: Color32::from_rgb(0x2d, 0x6c, 0x9e),
    idle: Color32::from_rgb(0x8e, 0x8e, 0x98),
    folder: Color32::from_rgb(0x66, 0x73, 0x8a),
};

const DARK: Palette = Palette {
    // The sheet is the "TernaryCode Dark" terminal's background (#111113).
    canvas: Color32::from_rgb(0x11, 0x11, 0x13),
    chrome: Color32::from_rgb(0x09, 0x09, 0x0b),
    surface: Color32::from_rgb(0x1a, 0x1a, 0x1d),
    overlay: Color32::from_rgb(0x14, 0x14, 0x16),
    menu: Color32::from_rgb(0x1c, 0x1c, 0x20),
    inset: Color32::from_rgb(0x0c, 0x0c, 0x0e),
    hover: white(10),
    subtle: white(20),
    border: white(15),
    border_strong: white(26),
    border_heavy: white(43),
    highlight: white(0),
    text: Color32::from_rgb(0xed, 0xed, 0xf0),
    text_muted: Color32::from_rgb(0xa1, 0xa1, 0xaa),
    text_faint: Color32::from_rgb(0x6c, 0x6c, 0x75),
    text_ghost: Color32::from_rgb(0x48, 0x48, 0x4f),
    accent: Color32::from_rgb(0x2e, 0x6b, 0xe6),
    accent_hover: Color32::from_rgb(0x3a, 0x78, 0xf0),
    on_accent: Color32::WHITE,
    accent_soft: white(15),
    accent_tint: rgba(0x2e, 0x6b, 0xe6, 41),
    accent_text: Color32::from_rgb(0x8b, 0xb2, 0xff),
    ring: rgba(0x4f, 0x8c, 0xff, 153),
    ok: Color32::from_rgb(0x4d, 0xc4, 0x8c),
    warn: Color32::from_rgb(0xf2, 0xb1, 0x4b),
    err: Color32::from_rgb(0xf0, 0x6a, 0x64),
    info: Color32::from_rgb(0x6a, 0xa5, 0xff),
    rate_limited: Color32::from_rgb(0xf2, 0x94, 0x41),
    cooldown: Color32::from_rgb(0x5d, 0xb6, 0xdc),
    idle: Color32::from_rgb(0x6c, 0x6c, 0x75),
    folder: Color32::from_rgb(0x8c, 0x96, 0xa5),
};

impl Palette {
    /// Barely visible fill for hovered list rows.
    pub fn faint_row(&self) -> Color32 {
        self.hover
    }

    /// The same colours, quieter: a row shown only for what is under it (a project in the
    /// search's results for one of its worktrees).
    pub fn dimmed(&self) -> Palette {
        let mut d = *self;
        for c in [&mut d.text, &mut d.text_muted, &mut d.text_faint, &mut d.ok, &mut d.warn, &mut d.err, &mut d.info, &mut d.accent_text] {
            *c = c.gamma_multiply(0.55);
        }
        d
    }

    /// The tint of a status colour behind its own text: a pill, a waiting row, a tile's foot.
    pub fn tinted(&self, c: Color32, dark: bool) -> Color32 {
        tint(c, dark)
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

/// A color at `alpha` (0–1) opacity, for soft fills behind colored text and icons. A colour
/// that is already translucent (a line, a hover fill) gets `alpha` of its own opacity.
pub fn soft(c: Color32, alpha: f32) -> Color32 {
    if c.a() < 255 {
        return c.gamma_multiply(alpha.clamp(0.0, 1.0));
    }
    Color32::from_rgba_unmultiplied(c.r(), c.g(), c.b(), (alpha.clamp(0.0, 1.0) * 255.0) as u8)
}

/// The soft fill behind status-coloured text (pills, notices, icon tiles): 12% in the dark
/// theme, 9% in the light one (the `status/*-tint` tokens).
pub fn tint(c: Color32, dark: bool) -> Color32 {
    soft(c, if dark { 0.12 } else { 0.09 })
}

/// `c` at `alpha` laid over `base`, opaque (a tinted surface that hides what is behind it).
/// A translucent `base` (a line) gives a colour with its opacity.
pub fn over(base: Color32, c: Color32, alpha: f32) -> Color32 {
    let mix = |a: u8, b: u8| (a as f32 + (b as f32 - a as f32) * alpha).round() as u8;
    if base.a() < 255 {
        let a = base.a().max((alpha * 255.0) as u8);
        return Color32::from_rgba_unmultiplied(c.r(), c.g(), c.b(), a);
    }
    Color32::from_rgb(mix(base.r(), c.r()), mix(base.g(), c.g()), mix(base.b(), c.b()))
}

// ---------------------------------------------------------------------------------------------
// Type
// ---------------------------------------------------------------------------------------------

/// Body text size of the UI (Body / Body Strong, 13/18).
pub const BODY: f32 = 13.0;
/// Small text: details, notes (Small / Small Strong, 12/16).
pub const SMALL: f32 = 12.0;
/// Labels: a group's title (Label, medium 11/14).
pub const LABEL: f32 = 11.0;
/// Data in the UI's monospace (Mono Small 11/14: branches, times, sizes, shortcuts).
pub const DATA: f32 = 11.0;
/// Mono 12/16: paths and commands in rows.
pub const MONO: f32 = 12.0;
/// Heading: semibold 14/20.
pub const HEADING: f32 = 14.0;
/// Title: semibold 17/24 (dialogs, page heads).
pub const TITLE: f32 = 17.0;
/// Display: semibold 24/30 (settings pages, onboarding).
pub const DISPLAY: f32 = 24.0;

/// Inter Medium: labels that need a little weight (tabs, buttons, row titles).
pub fn medium(size: f32) -> FontId {
    // Made once: a family's name is an `Arc<str>`, cloned here instead of allocated per call.
    static FAMILY: std::sync::LazyLock<FontFamily> = std::sync::LazyLock::new(|| FontFamily::Name(fonts::MEDIUM.into()));
    FontId::new(size, FAMILY.clone())
}

/// Inter SemiBold: titles and headings.
pub fn semibold(size: f32) -> FontId {
    static FAMILY: std::sync::LazyLock<FontFamily> = std::sync::LazyLock::new(|| FontFamily::Name(fonts::SEMIBOLD.into()));
    FontId::new(size, FAMILY.clone())
}

/// JetBrains Mono, whatever the terminal's font: the UI's data (branches, times, sizes,
/// addresses, shortcuts). Its figures line up, where Inter's (without `tnum`) don't.
pub fn mono(size: f32) -> FontId {
    static FAMILY: std::sync::LazyLock<FontFamily> = std::sync::LazyLock::new(|| FontFamily::Name(fonts::MONO.into()));
    FontId::new(size, FAMILY.clone())
}

/// JetBrains Mono Medium: key caps.
pub fn mono_medium(size: f32) -> FontId {
    static FAMILY: std::sync::LazyLock<FontFamily> = std::sync::LazyLock::new(|| FontFamily::Name(fonts::MONO_MEDIUM.into()));
    FontId::new(size, FAMILY.clone())
}

/// JetBrains Mono SemiBold: a project's letters, counts in badges.
pub fn mono_semibold(size: f32) -> FontId {
    static FAMILY: std::sync::LazyLock<FontFamily> = std::sync::LazyLock::new(|| FontFamily::Name(fonts::MONO_SEMIBOLD.into()));
    FontId::new(size, FAMILY.clone())
}

/// The filled icons (status glyphs: Needs input, Done, Failed).
pub fn icon_fill(size: f32) -> FontId {
    static FAMILY: std::sync::LazyLock<FontFamily> = std::sync::LazyLock::new(|| FontFamily::Name(fonts::ICONS_FILL.into()));
    FontId::new(size, FAMILY.clone())
}

/// The icon font alone (icons also render inside ordinary text).
pub fn icon_font(size: f32) -> FontId {
    static FAMILY: std::sync::LazyLock<FontFamily> = std::sync::LazyLock::new(|| FontFamily::Name(fonts::ICONS.into()));
    FontId::new(size, FAMILY.clone())
}

// ---------------------------------------------------------------------------------------------
// Style
// ---------------------------------------------------------------------------------------------

/// Buttons, fields, rows (radius/sm 6).
pub const RADIUS: u8 = 6;
/// Sheets (panes, panels, tiles) (radius/lg 12).
pub const RADIUS_LG: u8 = 12;
/// Rows of a list: sidebar places, palette and inbox rows, cards (7–8).
pub const RADIUS_ROW: u8 = 7;
/// Dialogs, the palette, settings (radius/xl 16).
pub const RADIUS_DIALOG: u8 = 16;
/// Cards and grouped settings (radius/md 8–10).
pub const RADIUS_CARD: u8 = 10;
/// The gap between sheets, and between the window's edge and the sheets.
pub const GAP: f32 = 6.0;
/// The window's edge beside the sheets (right and, with the sidebar hidden, left).
pub const EDGE: f32 = 8.0;

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
            s.menu_width = 232.0;
            s.scroll = egui::style::ScrollStyle::floating();
            s.scroll.bar_width = 8.0;
            s.scroll.floating_width = 3.0;
            s.scroll.floating_allocated_width = 0.0;
            // Selectable labels sense clicks and drags and would steal them from the rows,
            // buttons and tables they sit in; text that should be copyable opts in explicitly.
            style.interaction.selectable_labels = false;

            let v = &mut style.visuals;
            v.text_cursor.blink = false; // a blinking caret would repaint twice a second
            v.text_cursor.stroke = Stroke::new(1.5, p.accent_text);
            v.panel_fill = p.canvas;
            // Menus, tooltips and popups; dialogs paint their own (the overlay colour).
            v.window_fill = p.menu;
            v.extreme_bg_color = p.inset;
            v.text_edit_bg_color = Some(p.inset);
            v.faint_bg_color = p.hover;
            v.code_bg_color = p.subtle;
            v.hyperlink_color = p.accent_text;
            v.warn_fg_color = p.warn;
            v.error_fg_color = p.err;
            v.override_text_color = None;
            v.weak_text_color = Some(p.text_muted);
            v.selection.bg_fill = soft(p.accent, if dark { 0.35 } else { 0.18 });
            // The text color of selected rows and items, and egui's ring around a focused text
            // field.
            v.selection.stroke = Stroke::new(1.0, p.accent_text);
            v.window_corner_radius = CornerRadius::same(RADIUS_DIALOG);
            v.menu_corner_radius = CornerRadius::same(10);
            v.window_stroke = Stroke::new(1.0, p.border_strong);
            // Shadow/Dialog and Shadow/Menu of the design.
            v.window_shadow = Shadow {
                offset: [0, 24],
                blur: 64,
                spread: 0,
                color: Color32::from_black_alpha(if dark { 128 } else { 40 }),
            };
            v.popup_shadow = Shadow {
                offset: [0, 8],
                blur: 24,
                spread: 0,
                color: Color32::from_black_alpha(if dark { 89 } else { 26 }),
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
            // Buttons, inputs, checkboxes at rest: an outlined raised fill.
            w.inactive.bg_fill = p.subtle;
            w.inactive.weak_bg_fill = p.surface;
            w.inactive.bg_stroke = Stroke::new(1.0, p.border_strong);
            w.inactive.fg_stroke = Stroke::new(1.0, p.text);
            w.inactive.corner_radius = radius;
            w.inactive.expansion = 0.0;
            w.hovered.bg_fill = p.subtle;
            w.hovered.weak_bg_fill = p.hover;
            w.hovered.bg_stroke = Stroke::new(1.0, p.border_heavy);
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
                    egui::TextStyle::Heading => semibold(TITLE),
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
