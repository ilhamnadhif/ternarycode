//! Modal dialogs: add project, quit and remove confirmations (FR-L2, FR-P6), rename, the diff
//! shown before installing hooks, Run Agent, and messages.
//!
//! Every dialog is one frame (Slate): the overlay colour, a line, rounded 16, over a scrim. A
//! header (an icon tile tinted by what the dialog is for: the accent to do, amber to decide, red
//! for what can't be undone, green when done; the title, one sentence, ×), the body, and a
//! footer on the hover fill with a quiet hint at the left and the buttons at the right:
//! Secondary, then Primary with its ↵. A dialog that destroys something has a Danger button
//! without ↵, and Enter does nothing there; Esc and a click outside always cancel. Widths: 440
//! to confirm, 480 for a confirmation with a list, 560 for a form, 760 for lists and diffs, 860
//! for Run agent.

use eframe::egui::{self, Align2, Color32, CornerRadius, FontId, Key, Margin, Rect, RichText, Sense, Stroke, StrokeKind, Ui, pos2, vec2};
use tc_core::agent::AgentKind;

use super::{icons, kit, style};
use crate::actions::Action;
use crate::app::{App, Busy, Dialog};
use crate::i18n::{t, tf};

/// A confirmation.
pub const W_CONFIRM: f32 = 440.0;
/// A confirmation with a list (programs, a file).
pub const W_LIST: f32 = 480.0;
/// A form.
pub const W_FORM: f32 = 560.0;
/// Lists and diffs.
pub const W_WIDE: f32 = 760.0;
/// Run agent.
pub const W_RUN: f32 = 860.0;

/// Space between the dialog's sides and its content.
const PAD: i8 = 20;

/// The card every dialog sits on (its parts add their own margins): the overlay colour, a
/// line, rounded 16, Shadow/Dialog.
pub fn frame(ctx: &egui::Context) -> egui::Frame {
    let dark = ctx.global_style().visuals.dark_mode;
    let p = style::palette(dark);
    egui::Frame::NONE
        .fill(p.overlay)
        .stroke(Stroke::new(1.0, p.border_strong))
        .corner_radius(CornerRadius::same(style::RADIUS_DIALOG))
        .shadow(egui::epaint::Shadow { offset: [0, 24], blur: 64, spread: 0, color: Color32::from_black_alpha(if dark { 128 } else { 40 }) })
}

/// The dimming behind a dialog (bg/scrim: black 65% dark, 30% light).
pub fn scrim(dark: bool) -> Color32 {
    Color32::from_black_alpha(if dark { 166 } else { 77 })
}

/// Starts a dialog `w` wide (less on a narrow window), its parts edge to edge. Returns the
/// width of its content.
pub fn begin(ui: &mut Ui, w: f32) -> f32 {
    let w = w.min(ui.ctx().content_rect().width() - 32.0).max(320.0) - 2.0;
    ui.set_width(w);
    ui.spacing_mut().item_spacing.y = 0.0;
    w
}

/// What a dialog is for, as its icon tile says at a glance.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Intent {
    /// Something to do: the accent.
    Do,
    /// A choice to make: amber.
    Decide,
    /// What can't be undone: red.
    Danger,
    /// Done: green.
    Done,
}

impl Intent {
    /// The intent a colour stands for: red, amber, green, else the accent.
    pub fn of(color: Color32, p: &style::Palette) -> Self {
        if color == p.err {
            Intent::Danger
        } else if color == p.warn {
            Intent::Decide
        } else if color == p.ok {
            Intent::Done
        } else {
            Intent::Do
        }
    }

    /// The tile's fill and the glyph's colour.
    fn colors(self, p: &style::Palette, dark: bool) -> (Color32, Color32) {
        match self {
            Intent::Do => (p.accent_tint, p.accent_text),
            Intent::Decide => (style::tint(p.warn, dark), p.warn),
            Intent::Danger => (style::tint(p.err, dark), p.err),
            Intent::Done => (style::tint(p.ok, dark), p.ok),
        }
    }
}

/// The tile an icon sits in at the head of a dialog: 34, rounded 9, tinted by what the dialog
/// is for (`color`: the accent, amber, red or green).
pub fn paint_dialog_icon(ui: &mut Ui, icon: &str, color: Color32) -> egui::Response {
    let p = style::pal(ui);
    icon_tile(ui, icon, Intent::of(color, &p))
}

fn icon_tile(ui: &mut Ui, icon: &str, intent: Intent) -> egui::Response {
    let p = style::pal(ui);
    let (fill, ink) = intent.colors(&p, ui.visuals().dark_mode);
    let (rect, resp) = ui.allocate_exact_size(vec2(34.0, 34.0), Sense::hover());
    ui.painter().rect_filled(rect, CornerRadius::same(9), fill);
    ui.painter().text(rect.center(), Align2::CENTER_CENTER, icon, style::icon_font(17.0), ink);
    resp
}

/// A dialog's header: the icon tile in `color`'s intent, the title, one sentence. Returns
/// whether its close button was clicked.
pub fn header(ui: &mut Ui, icon: &str, color: Color32, title: &str, sub: Option<&str>) -> bool {
    let p = style::pal(ui);
    let intent = Intent::of(color, &p);
    head(ui, &mut |ui: &mut Ui| {
        icon_tile(ui, icon, intent);
    }, title, sub, false)
}

/// A header with what `lead` draws before the title (a 34 tile, or nothing); `sub` in the data
/// font when `mono_sub`. Returns whether its close button was clicked.
pub fn head(ui: &mut Ui, lead: &mut dyn FnMut(&mut Ui), title: &str, sub: Option<&str>, mono_sub: bool) -> bool {
    let p = style::pal(ui);
    let mut close = false;
    egui::Frame::NONE.inner_margin(Margin { left: PAD, right: 14, top: 16, bottom: 6 }).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal_top(|ui| {
            ui.spacing_mut().item_spacing = vec2(14.0, 0.0);
            lead(ui);
            let text_w = (ui.available_width() - 24.0 - 14.0).max(80.0);
            ui.vertical(|ui| {
                ui.set_width(text_w);
                ui.spacing_mut().item_spacing.y = 2.0;
                ui.add(egui::Label::new(RichText::new(title).font(style::semibold(style::TITLE)).color(p.text).line_height(Some(24.0))).wrap());
                if let Some(sub) = sub {
                    let (font, color) = if mono_sub { (style::mono(style::DATA), p.text_faint) } else { (FontId::proportional(style::SMALL), p.text_muted) };
                    ui.add(egui::Label::new(RichText::new(sub).font(font).color(color).line_height(Some(16.0))).wrap());
                }
            });
            close = kit::icon_button_ex(ui, icons::X, t("Close"), false, 24.0).clicked();
        });
    });
    close
}

/// The content between the header and the footer (its parts 12 apart). Empty, it still leaves
/// the room the design has between the two.
pub fn body<R>(ui: &mut Ui, add: impl FnOnce(&mut Ui) -> R) -> R {
    egui::Frame::NONE
        .inner_margin(Margin { left: PAD, right: PAD, top: 10, bottom: 18 })
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 12.0;
            // Fields are sunken, so they read as places to type.
            let p = style::pal(ui);
            ui.visuals_mut().text_edit_bg_color = Some(p.inset);
            ui.visuals_mut().extreme_bg_color = p.inset;
            add(ui)
        })
        .inner
}

/// A label over what follows it (Small Strong).
pub(super) fn field_label(ui: &mut Ui, text: &str) {
    let p = style::pal(ui);
    ui.label(RichText::new(text).font(style::medium(style::SMALL)).color(p.text));
}

/// A single-line text field the width of the dialog (minus `leave` for what follows it).
fn text_field(ui: &mut Ui, text: &mut String, hint: &str, leave: f32) -> egui::Response {
    let id = ui.next_auto_id();
    let w = (ui.available_width() - leave).max(80.0);
    kit::field(ui, id, text, None, hint, w, false)
}

/// A field in the data font (a folder, a command), `w` wide: as `kit::field`, in Mono 12.
pub(super) fn mono_field(ui: &mut Ui, id: egui::Id, text: &mut String, hint: &str, w: f32) -> egui::Response {
    let p = style::pal(ui);
    let (rect, outer) = ui.allocate_exact_size(vec2(w, 30.0), Sense::click());
    if outer.clicked() {
        ui.memory_mut(|m| m.request_focus(id));
    }
    let focused = ui.ctx().memory(|m| m.has_focus(id));
    if focused {
        ui.painter().rect_stroke(rect.expand(3.0), CornerRadius::same(style::RADIUS + 4), Stroke::new(3.0, style::soft(p.accent, 0.25)), StrokeKind::Inside);
    }
    ui.painter().rect(rect, CornerRadius::same(style::RADIUS + 1), p.inset, Stroke::new(1.0, if focused { p.accent } else { p.border_strong }), StrokeKind::Inside);
    let inner = rect.shrink2(vec2(10.0, 1.0));
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(inner).layout(egui::Layout::left_to_right(egui::Align::Center)));
    child.add_sized(
        inner.size(),
        egui::TextEdit::singleline(text)
            .id(id)
            .char_limit(4_096)
            .frame(egui::Frame::NONE)
            .margin(Margin::ZERO)
            .font(style::mono(style::MONO))
            .hint_text(RichText::new(hint).font(style::mono(style::MONO)).color(p.text_faint))
            .vertical_align(egui::Align::Center),
    )
}

/// The main button asks (↵ on it, Enter confirms), or destroys something (Danger: no ↵, Enter
/// does nothing, "esc cancel" at the left).
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Confirm {
    Primary,
    Danger,
}

/// The usual footer: `hint` at the left ("esc cancel" for a Danger one without), Cancel and the
/// main button at the right. Returns (cancel, ok).
pub(super) fn footer(ui: &mut Ui, hint: Option<&str>, ok: &str, kind: Confirm, enabled: bool) -> (bool, bool) {
    let mut out = (false, false);
    footer_left(
        ui,
        |ui| match hint {
            Some(h) => self::hint(ui, h),
            None if kind == Confirm::Danger => esc_hint(ui),
            None => {}
        },
        |ui| {
            let okr = ui
                .add_enabled_ui(enabled, |ui| match kind {
                    Confirm::Primary => kit::button_keys(ui, kit::Kind::Primary, None, ok, Some("↵"), false),
                    Confirm::Danger => kit::button_ex(ui, kit::Kind::Danger, None, ok, false),
                })
                .inner;
            #[cfg(debug_assertions)]
            crate::devtest::record("dialog-ok", okr.rect);
            out.1 = okr.clicked();
            out.0 = kit::secondary(ui, None, t("Cancel")).clicked();
        },
    );
    out
}

/// A footer's quiet hint at its left (Mono Small, tertiary).
pub fn hint(ui: &mut Ui, text: &str) {
    let p = style::pal(ui);
    ui.add(egui::Label::new(RichText::new(text).font(style::mono(style::DATA)).color(p.text_faint)).truncate());
}

/// A footer's hint that does something when clicked ("Run anyway", "Review first").
pub fn hint_link(ui: &mut Ui, text: &str) -> egui::Response {
    let p = style::pal(ui);
    let g = ui.painter().layout_no_wrap(text.to_string(), style::mono(style::DATA), Color32::PLACEHOLDER);
    let (rect, resp) = ui.allocate_exact_size(vec2(g.size().x, style::CONTROL_H), Sense::click());
    let hovered = resp.hovered();
    if hovered {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, text));
    let pos = pos2(rect.left(), rect.center().y - g.size().y / 2.0);
    let w = g.size().x;
    ui.painter().galley(pos, g, if hovered { p.text } else { p.text_faint });
    if hovered {
        ui.painter().hline(rect.left()..=rect.left() + w, pos.y + 14.0, Stroke::new(1.0, p.text_muted));
    }
    resp
}

/// "esc cancel" at a footer's left.
pub fn esc_hint(ui: &mut Ui) {
    hint(ui, &format!("esc {}", t("cancel")));
}

/// A footer with its own buttons: `hint` at the left, and what `buttons` adds at the right
/// (right to left: the main button first).
pub fn footer_with(ui: &mut Ui, hint: Option<&str>, buttons: impl FnOnce(&mut Ui)) {
    footer_left(
        ui,
        |ui| {
            if let Some(h) = hint {
                self::hint(ui, h);
            }
        },
        buttons,
    );
}

/// The footer, on the hover fill: what `left` adds at its left (a hint, a quiet choice) and
/// `buttons` at its right (right to left: the main button first). The buttons are placed
/// first, so a long hint is cut before it reaches them.
pub fn footer_left(ui: &mut Ui, left: impl FnOnce(&mut Ui), buttons: impl FnOnce(&mut Ui)) {
    let p = style::pal(ui);
    let r = style::RADIUS_DIALOG - 1;
    egui::Frame::NONE
        .fill(p.hover)
        .corner_radius(CornerRadius { nw: 0, ne: 0, sw: r, se: r })
        .inner_margin(Margin { left: PAD, right: 16, top: 12, bottom: 12 })
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.set_min_height(style::CONTROL_H);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.spacing_mut().item_spacing.x = 8.0;
                    buttons(ui);
                    ui.add_space(8.0);
                    ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                        ui.spacing_mut().item_spacing.x = 8.0;
                        left(ui);
                    });
                });
            });
        });
}

/// Programs that will be stopped and files not saved, in an inset well: rows of 28, the
/// program (an agent's logo, a terminal, a file), and what it is about at the right.
fn busy_list(ui: &mut Ui, items: &[Busy]) {
    let p = style::pal(ui);
    // "{name} · worktree, its folder stays", as Remove builds it.
    let worktree_tail = tf("{} · worktree, its folder stays", &[""]);
    egui::Frame::NONE
        .fill(p.inset)
        .corner_radius(CornerRadius::same(8))
        .inner_margin(Margin::same(4))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            egui::ScrollArea::vertical().max_height(180.0).auto_shrink([false, true]).show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 0.0;
                for b in items {
                    let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), 28.0), Sense::hover());
                    let worktree = !b.file && b.text.ends_with(&worktree_tail);
                    let (text, detail) = match b.text.rsplit_once(" · ") {
                        Some((a, d)) => (a.to_string(), d.to_string()),
                        None if b.file => (b.text.clone(), String::new()),
                        None => (b.text.clone(), t("running").to_string()),
                    };
                    let agent = if b.file || worktree { None } else { AgentKind::ALL.into_iter().find(|a| a.label() == text) };
                    let painter = ui.painter();
                    let cy = r.center().y;
                    let icon_c = pos2(r.left() + 10.0 + 7.0, cy);
                    if b.file {
                        painter.text(icon_c, Align2::CENTER_CENTER, icons::FILE_CODE, style::icon_font(14.0), p.warn);
                    } else if worktree {
                        painter.text(icon_c, Align2::CENTER_CENTER, icons::GIT_FORK, style::icon_font(14.0), p.text_muted);
                    } else if !agent.is_some_and(|a| super::agent_logo::paint(painter, Rect::from_center_size(icon_c, vec2(14.0, 14.0)), a, p.text, false)) {
                        painter.text(icon_c, Align2::CENTER_CENTER, icons::TERMINAL_WINDOW, style::icon_font(14.0), p.text_muted);
                    }
                    let mut right = r.right() - 10.0;
                    if !detail.is_empty() {
                        let g = painter.layout_no_wrap(detail, FontId::proportional(style::SMALL), p.text_faint);
                        right -= g.size().x;
                        painter.galley(pos2(right, cy - g.size().y / 2.0), g, p.text_faint);
                        right -= 8.0;
                    }
                    let x = r.left() + 10.0 + 14.0 + 8.0;
                    let font = if agent.is_some() || worktree { FontId::proportional(style::SMALL) } else { style::mono(style::DATA) };
                    let mut job = egui::text::LayoutJob::simple_singleline(text, font, p.text);
                    job.wrap = egui::text::TextWrapping::truncate_at_width((right - x).max(20.0));
                    let g = painter.layout_job(job);
                    painter.galley(pos2(x, cy - g.size().y / 2.0), g, p.text);
                }
            });
        });
}

/// A quiet sentence under a dialog's body (Small, tertiary).
fn note(ui: &mut Ui, text: &str) {
    let p = style::pal(ui);
    ui.add(egui::Label::new(RichText::new(text).size(style::SMALL).color(p.text_faint).line_height(Some(16.0))).wrap());
}

/// An agent's avatar (L 28, M 20, S 16): its mark on a quiet tile; a terminal for a program
/// that isn't one.
pub(super) fn paint_agent_avatar(painter: &egui::Painter, rect: Rect, agent: Option<AgentKind>, p: &style::Palette) {
    let s = rect.width();
    let r = if s >= 26.0 { 7 } else if s >= 19.0 { 5 } else { 4 };
    painter.rect_filled(rect, CornerRadius::same(r), p.subtle);
    let m = (s * 0.68).round();
    let mark = Rect::from_center_size(rect.center(), vec2(m, m));
    if !agent.is_some_and(|a| super::agent_logo::paint(painter, mark, a, p.text, false)) {
        painter.text(rect.center(), Align2::CENTER_CENTER, icons::TERMINAL, style::icon_font((s * 0.6).round()), p.text_muted);
    }
}

impl App {
    pub fn dialogs(&mut self, ctx: &egui::Context) {
        let Some(mut dialog) = self.dialog.take() else { return };
        let mut keep = true;
        let enter = ctx.input(|i| i.key_pressed(Key::Enter));
        let p = style::palette(ctx.global_style().visuals.dark_mode);
        let modal = egui::Modal::new(egui::Id::new("dialog"))
            .frame(frame(ctx))
            .backdrop_color(scrim(ctx.global_style().visuals.dark_mode))
            .show(ctx, |ui| match &mut dialog {
                Dialog::AddProject { path, name, then, found } => {
                    begin(ui, W_FORM);
                    if header(ui, icons::FOLDER_SIMPLE_PLUS, p.accent, t("Add project"), Some(t("Its terminals start in this folder."))) {
                        keep = false;
                    }
                    let presets = self.presets();
                    let next_color = {
                        let (r, g, b) = crate::project::COLORS[self.projects.len() % crate::project::COLORS.len()];
                        Color32::from_rgb(r, g, b)
                    };
                    body(ui, |ui| {
                        // Label at the left (70), the control at the right.
                        let row = |ui: &mut Ui, label: &str, add: &mut dyn FnMut(&mut Ui)| {
                            ui.horizontal(|ui| {
                                ui.spacing_mut().item_spacing.x = 10.0;
                                let (r, _) = ui.allocate_exact_size(vec2(70.0, 30.0), Sense::hover());
                                ui.painter().text(r.left_center(), Align2::LEFT_CENTER, label, FontId::proportional(style::SMALL), p.text_faint);
                                add(ui);
                            });
                        };
                        row(ui, t("Folder"), &mut |ui| {
                            let choose_w = kit::button_width(ui, Some(icons::FOLDER_OPEN), t("Choose…"), None, false);
                            let w = (ui.available_width() - choose_w - 10.0).max(80.0);
                            let id = egui::Id::new("add-project-folder");
                            let r = mono_field(ui, id, path, "~/code/app", w);
                            let init_id = egui::Id::new("add-project-init-focus");
                            if !ui.data(|d| d.get_temp::<bool>(init_id).unwrap_or(false)) {
                                ui.data_mut(|d| d.insert_temp(init_id, true));
                                r.request_focus();
                            }
                            if kit::button_ex(ui, kit::Kind::Secondary, Some(icons::FOLDER_OPEN), t("Choose…"), false).clicked()
                                && let Some(dir) = rfd::FileDialog::new().pick_folder()
                            {
                                *path = dir.display().to_string();
                            }
                        });
                        row(ui, t("Name"), &mut |ui| {
                            let folder_name = std::path::Path::new(path.trim()).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                            let hint = if folder_name.is_empty() { t("The folder's name").to_string() } else { folder_name };
                            text_field(ui, name, &hint, 0.0);
                        });
                        // What is at that folder (looked at again when the path changes): its
                        // letter in the colour it will get, its kind and branch.
                        let typed = path.trim().to_string();
                        let letters_id = egui::Id::new("add-project-letters");
                        if found.as_ref().is_none_or(|(checked, _)| *checked != typed) {
                            let summary = folder_summary(&typed);
                            let letters = if summary.is_some() {
                                let dir = crate::app::absolute(&typed);
                                let folder = dir.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                                crate::project::initial(&folder)
                            } else {
                                String::new()
                            };
                            ui.data_mut(|d| d.insert_temp(letters_id, (typed.clone(), letters)));
                            *found = Some((typed.clone(), summary));
                        }
                        if let Some((_, Some(line))) = found.as_ref() {
                            ui.horizontal(|ui| {
                                ui.spacing_mut().item_spacing.x = 10.0;
                                ui.allocate_exact_size(vec2(70.0, 1.0), Sense::hover());
                                let w = ui.available_width();
                                let (r, _) = ui.allocate_exact_size(vec2(w, 36.0), Sense::hover());
                                let dark = ui.visuals().dark_mode;
                                ui.painter().rect_filled(r, CornerRadius::same(8), p.hover);
                                let letters = ui
                                    .data(|d| d.get_temp::<(String, String)>(letters_id))
                                    .filter(|(k, _)| k == &typed)
                                    .map(|(_, l)| l)
                                    .unwrap_or_else(|| {
                                        let dir = crate::app::absolute(&typed);
                                        let folder = dir.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                                        crate::project::initial(&folder)
                                    });
                                let tile = Rect::from_min_size(pos2(r.left() + 10.0, r.center().y - 10.0), vec2(20.0, 20.0));
                                kit::paint_project_tile(ui.painter(), tile, &letters, next_color, dark);
                                // "TypeScript · ⑂ main".
                                let small = FontId::proportional(style::SMALL);
                                let mut job = egui::text::LayoutJob::default();
                                let parts: Vec<&str> = line.split(" · ").collect();
                                job.append(parts.first().copied().unwrap_or(""), 0.0, egui::TextFormat::simple(small.clone(), p.text_muted));
                                if let Some(branch) = parts.get(1) {
                                    job.append(" ·", 0.0, egui::TextFormat::simple(small, p.text_muted));
                                    job.append(icons::GIT_BRANCH, 8.0, egui::TextFormat::simple(style::icon_font(12.0), p.text_faint));
                                    job.append(branch, 8.0, egui::TextFormat::simple(style::mono(style::DATA), p.text_muted));
                                }
                                job.wrap = egui::text::TextWrapping::truncate_at_width(w - 48.0);
                                let g = ui.painter().layout_job(job);
                                ui.painter().galley(pos2(tile.right() + 8.0, r.center().y - g.size().y / 2.0), g, p.text_muted);
                            });
                        }
                        // An agent to start in it once added.
                        row(ui, t("Then run"), &mut |ui| {
                            let mut options = vec![kit::Seg::new(t("Nothing"), "")];
                            options.extend(presets.iter().take(3).map(|x| kit::Seg::new(x.name.as_str(), x.command.as_str())));
                            if let Some(i) = kit::segmented(ui, &options, *then) {
                                *then = i;
                            }
                        });
                    });
                    let ready = !path.trim().is_empty();
                    let (cancel, ok) = footer(ui, None, t("Add project"), Confirm::Primary, ready);
                    if cancel {
                        keep = false;
                    }
                    if (ok || enter) && ready {
                        let p = crate::app::absolute(path.trim());
                        let n = Some(name.clone()).filter(|n| !n.trim().is_empty());
                        self.actions.push(Action::AddProjectPath(p.clone()));
                        if let Some(n) = n {
                            self.pending_rename = Some((p, n));
                        }
                        if *then > 0
                            && let Some(preset) = presets.get(*then - 1)
                        {
                            self.actions.push(Action::RunAgent(preset.id.clone(), crate::actions::Launch::New, 0));
                        }
                        keep = false;
                    }
                }
                Dialog::ConfirmUninstall(agent) => {
                    begin(ui, W_LIST);
                    if header(
                        ui,
                        icons::PLUGS,
                        p.warn,
                        &tf("Uninstall hooks for {}?", &[agent.label()]),
                        Some(t("Its settings file goes back to how it was before Install; changes you made since are kept.")),
                    ) {
                        keep = false;
                        self.actions.push(Action::Integrations);
                    }
                    body(ui, |_| {});
                    let (cancel, ok) = footer(ui, None, t("Uninstall"), Confirm::Danger, true);
                    if cancel {
                        keep = false;
                        self.actions.push(Action::Integrations);
                    }
                    if ok {
                        self.actions.push(Action::UninstallNow(*agent));
                        self.actions.push(Action::Integrations);
                        keep = false;
                    }
                }
                Dialog::RunAgent { selected, session, place, wt } => {
                    keep = self.run_agent_dialog(ui, selected, session, place, wt, enter);
                }
                Dialog::MergeWorktree { id, commit_first, message, message_init, committing, error } => {
                    keep = self.merge_worktree_dialog(ui, *id, commit_first, message, message_init, committing, error, enter);
                }
                Dialog::RemoveWorktree(id, branch) => {
                    keep = self.remove_worktree_dialog(ui, *id, branch);
                }
                Dialog::WorktreeSetup { id, copy, adding, run, fresh } => {
                    keep = self.worktree_setup_dialog(ui, *id, copy, adding, run, fresh, enter);
                }
                Dialog::AgentMissing { preset, launch, place, profile, program, npm } => {
                    keep = self.agent_missing_dialog(ui, preset, launch, *place, profile.as_deref(), program, *npm, enter);
                }
                Dialog::ConfirmCloseFile(project, path, name) => {
                    begin(ui, W_LIST);
                    if header(ui, icons::FLOPPY_DISK, p.warn, &tf("Save changes to {}?", &[name]), Some(t("Your changes are lost if you don’t save them."))) {
                        keep = false;
                    }
                    // The file: its path in the project, and how many lines were edited.
                    let root = self.project(*project).map(|x| (x.path.clone(), x.name.clone()));
                    let edited = self.editors.get(project).and_then(|e| e.files.iter().find(|f| f.path == *path)).map(|f| {
                        let (a, b): (Vec<&str>, Vec<&str>) = (f.saved.lines().collect(), f.text.lines().collect());
                        (0..a.len().max(b.len())).filter(|i| a.get(*i) != b.get(*i)).count()
                    });
                    body(ui, |ui| {
                        let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), 36.0), Sense::hover());
                        ui.painter().rect_filled(r, CornerRadius::same(8), p.inset);
                        ui.painter().text(pos2(r.left() + 12.0 + 7.0, r.center().y), Align2::CENTER_CENTER, icons::FILE_CODE, style::icon_font(14.0), p.text_muted);
                        let rel = root.as_ref().and_then(|(dir, _)| path.strip_prefix(dir).ok()).map_or_else(|| path.display().to_string(), |r| r.display().to_string());
                        let mut right = r.right() - 12.0;
                        let mut what = Vec::new();
                        if let Some((_, pname)) = &root {
                            what.push(pname.clone());
                        }
                        if let Some(n) = edited.filter(|n| *n > 0) {
                            what.push(crate::i18n::count(n, t("1 line edited"), t("{} lines edited")));
                        }
                        if !what.is_empty() {
                            let g = ui.painter().layout_no_wrap(what.join(" · "), FontId::proportional(style::SMALL), p.text_faint);
                            right -= g.size().x;
                            ui.painter().galley(pos2(right, r.center().y - g.size().y / 2.0), g, p.text_faint);
                            right -= 8.0;
                        }
                        let x = r.left() + 12.0 + 14.0 + 8.0;
                        let mut job = egui::text::LayoutJob::simple_singleline(rel, style::mono(style::DATA), p.text);
                        job.wrap = egui::text::TextWrapping::truncate_at_width((right - x).max(20.0));
                        let g = ui.painter().layout_job(job);
                        ui.painter().galley(pos2(x, r.center().y - g.size().y / 2.0), g, p.text);
                    });
                    let mut save = enter;
                    // ⌘⌫ (Ctrl+⌫) is Don't Save, as in the system's own dialogs.
                    let (mut discard, mut cancel) = (ui.input(|i| i.modifiers.command && i.key_pressed(Key::Backspace)), false);
                    footer_left(
                        ui,
                        |ui| {
                            discard |= hint_link(ui, &format!("⌫ {}", t("don’t save"))).clicked();
                        },
                        |ui| {
                            let okr = kit::button_keys(ui, kit::Kind::Primary, None, t("Save"), Some("↵"), false);
                            #[cfg(debug_assertions)]
                            crate::devtest::record("dialog-ok", okr.rect);
                            save |= okr.clicked();
                            cancel = kit::secondary(ui, None, t("Cancel")).clicked();
                        },
                    );
                    if cancel {
                        keep = false;
                    }
                    if discard {
                        self.actions.push(Action::DiscardAndCloseFile(*project, path.clone()));
                        keep = false;
                        save = false;
                    }
                    if save {
                        self.actions.push(Action::SaveAndCloseFile(*project, path.clone()));
                        keep = false;
                    }
                }
                Dialog::ConfirmQuit(busy) => {
                    begin(ui, W_LIST);
                    if header(
                        ui,
                        icons::SIGN_OUT,
                        p.err,
                        t("Quit TernaryCode?"),
                        Some(t("Programs still running are stopped, and changes not saved are lost.")),
                    ) {
                        keep = false;
                    }
                    body(ui, |ui| {
                        busy_list(ui, busy);
                        note(ui, t("Closing the window instead keeps them running in the background."));
                    });
                    let (cancel, ok) = footer(ui, None, t("Quit"), Confirm::Danger, true);
                    if cancel {
                        keep = false;
                    }
                    if ok {
                        self.actions.push(Action::QuitNow);
                        keep = false;
                    }
                }
                Dialog::ConfirmUpdate(busy) => {
                    begin(ui, W_LIST);
                    // Restarting in place, the programs keep running: only unsaved files are
                    // listed.
                    let in_place = crate::update::restarts_in_place();
                    let (lost, after) = if in_place {
                        (t("Changes not saved are lost."), t("TernaryCode restarts as the new version; programs and agents keep running."))
                    } else {
                        (
                            t("Programs still running are stopped, and changes not saved are lost."),
                            t("TernaryCode opens again with your projects and tabs; agents that support it can be resumed."),
                        )
                    };
                    if header(ui, icons::ARROW_CIRCLE_UP, p.warn, t("Restart to update?"), Some(lost)) {
                        keep = false;
                    }
                    body(ui, |ui| {
                        busy_list(ui, busy);
                        note(ui, after);
                    });
                    let (cancel, ok) = footer(ui, None, t("Restart and update"), Confirm::Danger, true);
                    if cancel {
                        keep = false;
                    }
                    if ok {
                        self.actions.push(Action::RestartToUpdateNow);
                        keep = false;
                    }
                }
                Dialog::ConfirmRemove(id, busy) => {
                    begin(ui, W_LIST);
                    let name = self.project(*id).map(|x| x.name.clone()).unwrap_or_default();
                    let sub = if busy.iter().any(|b| b.file) {
                        t("The folder is not deleted. Programs still running are stopped, and changes not saved are lost.")
                    } else {
                        t("The folder is not deleted. Programs still running are stopped.")
                    };
                    if header(ui, icons::TRASH, p.err, &tf("Remove {} from the workspace?", &[&name]), Some(sub)) {
                        keep = false;
                    }
                    body(ui, |ui| busy_list(ui, busy));
                    let (cancel, ok) = footer(ui, None, t("Remove"), Confirm::Danger, true);
                    if cancel {
                        keep = false;
                    }
                    if ok {
                        self.actions.push(Action::RemoveProjectNow(*id));
                        keep = false;
                    }
                }
                Dialog::ConfirmTrash(path) => {
                    begin(ui, W_CONFIRM);
                    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                    if header(ui, icons::TRASH, p.err, &tf("Move \"{}\" to the Trash?", &[&name]), Some(t("You can put it back from the Trash."))) {
                        keep = false;
                    }
                    body(ui, |_| {});
                    let (cancel, ok) = footer(ui, None, t("Move to Trash"), Confirm::Danger, true);
                    if cancel {
                        keep = false;
                    }
                    if ok {
                        self.actions.push(Action::TrashPathNow(path.clone()));
                        keep = false;
                    }
                }
                Dialog::ConfirmTrashMany(paths) => {
                    begin(ui, W_CONFIRM);
                    if header(
                        ui,
                        icons::TRASH,
                        p.err,
                        &tf("Move {} items to the Trash?", &[&paths.len().to_string()]),
                        Some(t("You can put them back from the Trash.")),
                    ) {
                        keep = false;
                    }
                    body(ui, |ui| {
                        ui.spacing_mut().item_spacing.y = 3.0;
                        for path in paths.iter().take(6) {
                            let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                            ui.label(egui::RichText::new(name).font(style::mono(style::DATA)).color(p.text_muted));
                        }
                        if paths.len() > 6 {
                            ui.label(
                                egui::RichText::new(tf("… and {} more", &[&(paths.len() - 6).to_string()]))
                                    .size(style::SMALL)
                                    .color(p.text_faint),
                            );
                        }
                    });
                    let (cancel, ok) = footer(ui, None, t("Move to Trash"), Confirm::Danger, true);
                    if cancel {
                        keep = false;
                    }
                    if ok {
                        self.actions.push(Action::TrashPathsNow(paths.clone()));
                        keep = false;
                    }
                }
                Dialog::ConfirmClearBrowsing => {
                    begin(ui, W_CONFIRM);
                    if header(
                        ui,
                        icons::TRASH,
                        p.err,
                        t("Clear logins, cookies and history?"),
                        Some(t("Every browser page is signed out of every site, in every project. This can't be undone.")),
                    ) {
                        keep = false;
                    }
                    body(ui, |_| {});
                    let (cancel, ok) = footer(ui, None, t("Clear"), Confirm::Danger, true);
                    if cancel {
                        keep = false;
                    }
                    if ok {
                        self.clear_browsing_data();
                        keep = false;
                    }
                }
                Dialog::ConfirmClosePane(id, what) => {
                    begin(ui, W_CONFIRM);
                    if header(ui, icons::WARNING, p.err, t("Close this pane?"), Some(&tf("{} is still running in it and will be stopped.", &[what]))) {
                        keep = false;
                    }
                    // What runs: its name, its process group, since when, the project.
                    let pane = self.panes.get(id);
                    let pgid = pane.and_then(|x| x.session.as_ref()).and_then(|s| s.pty.foreground_pgid());
                    let since = pane.map(|x| super::status::age(x.tracker.since));
                    let project = self.projects.iter().find(|x| x.panes().contains(id)).map(|x| x.name.clone());
                    body(ui, |ui| {
                        let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), 58.0), Sense::hover());
                        ui.painter().rect_filled(r, CornerRadius::same(8), p.inset);
                        super::status::paint_glyph(ui.painter(), pos2(r.left() + 12.0 + 7.0, r.center().y), tc_core::status::Status::Running, &p, 14.0);
                        let x = r.left() + 12.0 + 14.0 + 10.0;
                        let max_w = (r.right() - 12.0 - x).max(20.0);
                        let mut job1 = egui::text::LayoutJob::simple_singleline(what.clone(), style::mono(style::MONO), p.text);
                        job1.wrap = egui::text::TextWrapping::truncate_at_width(max_w);
                        let g1 = ui.painter().layout_job(job1);
                        ui.painter().galley(pos2(x, r.top() + 12.0 + (16.0 - g1.size().y) / 2.0), g1, p.text);
                        let mut parts = Vec::new();
                        if let Some(g) = pgid {
                            parts.push(tf("pid {}", &[&g.to_string()]));
                        }
                        if let Some(a) = since {
                            parts.push(tf("running {}", &[&a]));
                        }
                        if let Some(n) = project {
                            parts.push(n);
                        }
                        let mut job2 = egui::text::LayoutJob::simple_singleline(parts.join(" · "), FontId::proportional(style::SMALL), p.text_faint);
                        job2.wrap = egui::text::TextWrapping::truncate_at_width(max_w);
                        let g2 = ui.painter().layout_job(job2);
                        ui.painter().galley(pos2(x, r.top() + 12.0 + 18.0 + (16.0 - g2.size().y) / 2.0), g2, p.text_faint);
                    });
                    let (cancel, ok) = footer(ui, None, t("Close pane"), Confirm::Danger, true);
                    if cancel {
                        keep = false;
                    }
                    if ok {
                        self.actions.push(Action::ClosePaneId(*id));
                        keep = false;
                    }
                }
                Dialog::ConfirmCloseTab(panes, busy) => {
                    begin(ui, W_LIST);
                    if header(ui, icons::WARNING, p.err, t("Close this tab?"), Some(t("These programs will be stopped."))) {
                        keep = false;
                    }
                    body(ui, |ui| busy_list(ui, busy));
                    let (cancel, ok) = footer(ui, None, t("Close tab"), Confirm::Danger, true);
                    if cancel {
                        keep = false;
                    }
                    if ok {
                        self.actions.push(Action::ClosePanesNow(panes.clone()));
                        keep = false;
                    }
                }
                Dialog::Rename(id, name) => {
                    begin(ui, W_CONFIRM);
                    if header(ui, icons::PENCIL_SIMPLE, p.accent, t("Rename project"), None) {
                        keep = false;
                    }
                    body(ui, |ui| {
                        let r = text_field(ui, name, "", 0.0);
                        let init_id = egui::Id::new("rename-init-focus");
                        if !ui.data(|d| d.get_temp::<bool>(init_id).unwrap_or(false)) {
                            ui.data_mut(|d| d.insert_temp(init_id, true));
                            r.request_focus();
                        }
                    });
                    let ready = !name.trim().is_empty();
                    let (cancel, ok) = footer(ui, None, t("Rename"), Confirm::Primary, ready);
                    if cancel {
                        keep = false;
                    }
                    if (ok || enter) && ready {
                        self.actions.push(Action::RenameProject(*id, name.trim().to_string()));
                        keep = false;
                    }
                }
                Dialog::RenamePane(id, name) => {
                    begin(ui, W_CONFIRM);
                    if header(ui, icons::PENCIL_SIMPLE, p.accent, t("Rename pane"), Some(t("Leave it empty to name it after what runs in it."))) {
                        keep = false;
                    }
                    body(ui, |ui| {
                        let r = text_field(ui, name, "", 0.0);
                        let init_id = egui::Id::new("rename-pane-init-focus");
                        if !ui.data(|d| d.get_temp::<bool>(init_id).unwrap_or(false)) {
                            ui.data_mut(|d| d.insert_temp(init_id, true));
                            r.request_focus();
                        }
                    });
                    let (cancel, ok) = footer(ui, None, t("Rename"), Confirm::Primary, true);
                    if cancel {
                        keep = false;
                    }
                    if ok || enter {
                        self.actions.push(Action::RenamePane(*id, name.trim().to_string()));
                        keep = false;
                    }
                }
                Dialog::Install(plan, diff) => {
                    begin(ui, W_WIDE);
                    if header(
                        ui,
                        icons::PLUGS,
                        p.accent,
                        &tf("Install hooks for {}?", &[plan.agent.label()]),
                        Some(t("So agents you start by hand report Needs input, Done and Failed too.")),
                    ) {
                        keep = false;
                        self.actions.push(Action::Integrations);
                    }
                    body(ui, |ui| install_preview(ui, &plan.path, plan.before.as_deref(), diff));
                    // Where the original is kept.
                    let backup = crate::paths::contract_tilde(&crate::paths::integrations_dir().join("backup").join(format!("{}-original", plan.agent.slug())));
                    let (cancel, ok) = footer(ui, Some(&tf("Backup in {}", &[&backup])), t("Install"), Confirm::Primary, true);
                    if cancel {
                        keep = false;
                        self.actions.push(Action::Integrations);
                    }
                    if ok || enter {
                        self.actions.push(Action::ConfirmInstall);
                    }
                }
                Dialog::Message(head, text) => {
                    begin(ui, W_CONFIRM);
                    if header(ui, icons::INFO, p.accent, head, None) {
                        keep = false;
                    }
                    body(ui, |ui| {
                        ui.add(egui::Label::new(RichText::new(text.as_str()).size(style::SMALL).color(p.text_muted).line_height(Some(16.0))).wrap());
                    });
                    footer_with(ui, None, |ui| {
                        if kit::button_keys(ui, kit::Kind::Primary, None, t("OK"), Some("↵"), false).clicked() || enter {
                            keep = false;
                        }
                    });
                }
                Dialog::Settings(page) => {
                    if !self.settings_ui(ui, page) {
                        keep = false;
                    }
                }
            });
        if modal.should_close() {
            keep = false;
            // As its Cancel: back to the integrations it was asked from.
            if matches!(dialog, Dialog::Install(..) | Dialog::ConfirmUninstall(..)) {
                self.actions.push(Action::Integrations);
            }
            match &dialog {
                Dialog::MergeWorktree { id, .. } => ctx.data_mut(|d| d.remove::<(usize, u32, u32)>(egui::Id::new(("merge-stat", *id)))),
                Dialog::RemoveWorktree(id, _) => ctx.data_mut(|d| d.remove::<Vec<(String, String)>>(egui::Id::new(("remove-commits", *id)))),
                _ => {}
            }
        }
        // A dialog that opened another while it was drawn (Settings' "Clear…"): that one shows,
        // and this one comes back after it.
        if self.dialog.is_some() {
            if keep {
                self.dialog_back = Some(dialog);
            }
            return;
        }
        // Run Agent's "Edit…": the worktree setup opens over it, and it comes back after.
        if let Some(id) = self.pending_setup.take() {
            self.dialog_back = Some(dialog);
            let setup = self.worktree_setup_of(id);
            let fresh = self.project(id).is_some_and(|p| p.worktree_setup.is_none());
            self.dialog = Some(Dialog::WorktreeSetup { id, copy: setup.copy, adding: None, run: setup.run, fresh });
            return;
        }
        // `ConfirmInstall` takes the dialog itself.
        let install_confirmed = self.actions.contains(&Action::ConfirmInstall);
        if keep || install_confirmed {
            self.dialog = Some(dialog);
        } else {
            ctx.data_mut(|d| {
                d.remove::<bool>(egui::Id::new("add-project-init-focus"));
                d.remove::<bool>(egui::Id::new("rename-init-focus"));
                d.remove::<bool>(egui::Id::new("rename-pane-init-focus"));
                d.remove::<Vec<(String, bool)>>(egui::Id::new(NO_HOOKS_ID));
            });
            if matches!(dialog, Dialog::Settings(_)) {
                self.settings_search.clear();
            }
            if let Some(back) = self.dialog_back.take() {
                self.dialog = Some(back);
            } else {
                // In the Grid, the tile typed into.
                self.focus_request = self.target_pane().or_else(|| self.current_pane());
            }
        }
    }
}

/// What is at a folder typed into Add Project: its kind and its git branch ("Rust · main");
/// nothing when it is not a folder.
fn folder_summary(typed: &str) -> Option<String> {
    if typed.is_empty() {
        return None;
    }
    let dir = crate::app::absolute(typed);
    if !dir.is_dir() {
        return None;
    }
    let kind = crate::project::Kind::detect(&dir).label();
    let branch = std::fs::read_to_string(dir.join(".git").join("HEAD"))
        .ok()
        .and_then(|h| h.trim().strip_prefix("ref: refs/heads/").map(str::to_string));
    // "Go module · main" (its kind, then its branch when it is a git repository).
    let kind = if kind.is_empty() { t("A folder").to_string() } else { kind.to_string() };
    Some(match branch {
        Some(b) => format!("{kind} · {b}"),
        None => kind,
    })
}

/// The change Install makes: the file (where, whether it is there, how much changes), what it
/// does in three, and the lines themselves with their old and new numbers (long runs of
/// unchanged lines folded).
fn install_preview(ui: &mut Ui, path: &std::path::Path, before: Option<&str>, diff: &str) {
    let p = style::pal(ui);
    let dark = ui.visuals().dark_mode;
    let lines: Vec<&str> = diff.lines().collect();
    let added = lines.iter().filter(|l| l.starts_with("+ ")).count();
    let removed = lines.iter().filter(|l| l.starts_with("- ")).count();
    let w = ui.available_width();
    let small = FontId::proportional(style::SMALL);
    // The file.
    let (head, _) = ui.allocate_exact_size(vec2(w, 36.0), Sense::hover());
    let painter = ui.painter();
    let cy = head.center().y;
    painter.text(pos2(head.left() + 12.0 + 7.0, cy), Align2::CENTER_CENTER, icons::FILE_CODE, style::icon_font(14.0), p.text_muted);
    let mut cjob = egui::text::LayoutJob::default();
    cjob.append(&format!("+{added}"), 0.0, egui::TextFormat::simple(style::mono(style::DATA), p.ok));
    cjob.append(&format!("−{removed}"), 6.0, egui::TextFormat::simple(style::mono(style::DATA), if removed > 0 { p.err } else { p.ok }));
    let cg = painter.layout_job(cjob);
    let right = head.right() - 12.0 - cg.size().x;
    painter.galley(pos2(right, cy - cg.size().y / 2.0), cg, p.text);
    let state = match before {
        Some(text) => tf("exists, {} lines", &[&text.lines().count().to_string()]),
        None => t("a new file").to_string(),
    };
    let mut job = egui::text::LayoutJob::default();
    job.append(&crate::paths::contract_tilde(path), 0.0, egui::TextFormat::simple(style::mono(style::DATA), p.text));
    job.append(&state, 8.0, egui::TextFormat::simple(small.clone(), p.text_faint));
    let x = head.left() + 12.0 + 14.0 + 8.0;
    job.wrap = egui::text::TextWrapping::truncate_at_width((right - 8.0 - x).max(40.0));
    let g = painter.layout_job(job);
    painter.galley(pos2(x, cy - g.size().y / 2.0), g, p.text);
    // What it does, in three cells.
    let items = [
        (crate::i18n::count(added, t("Adds 1 line"), t("Adds {} lines")), t("Its hooks, after yours").to_string()),
        (
            t("Keeps what is there").to_string(),
            if removed == 0 { t("Nothing is removed").to_string() } else { crate::i18n::count(removed, t("1 line is rewritten"), t("{} lines are rewritten")) },
        ),
        (t("Keeps a backup").to_string(), t("Uninstall restores it byte for byte").to_string()),
    ];
    let (strip, _) = ui.allocate_exact_size(vec2(w, 54.0), Sense::hover());
    let cw = (strip.width() - 16.0) / 3.0;
    for (i, (title, sub)) in items.iter().enumerate() {
        let cell = Rect::from_min_size(pos2(strip.left() + i as f32 * (cw + 8.0), strip.top()), vec2(cw, strip.height()));
        let painter = ui.painter();
        painter.rect_filled(cell, CornerRadius::same(8), p.hover);
        let tw = (cell.width() - 20.0).max(20.0);
        let mut job = egui::text::LayoutJob::simple_singleline(title.clone(), style::medium(style::SMALL), p.text);
        job.wrap = egui::text::TextWrapping::truncate_at_width(tw);
        let g = painter.layout_job(job);
        painter.galley(pos2(cell.left() + 10.0, cell.top() + 10.0 + (16.0 - g.size().y) / 2.0), g, p.text);
        let mut job = egui::text::LayoutJob::simple_singleline(sub.clone(), small.clone(), p.text_faint);
        job.wrap = egui::text::TextWrapping::truncate_at_width(tw);
        let g = painter.layout_job(job);
        painter.galley(pos2(cell.left() + 10.0, cell.top() + 28.0 + (16.0 - g.size().y) / 2.0), g, p.text_faint);
    }
    // The change: unchanged lines more than three away from it folded.
    let syntax = crate::editor::syntax_for(path);
    let changed = |l: &&str| l.starts_with("+ ") || l.starts_with("- ");
    let near = |i: usize| lines[i.saturating_sub(3)..(i + 4).min(lines.len())].iter().any(changed);
    // Each line's old and new numbers.
    let mut numbers = Vec::with_capacity(lines.len());
    let (mut old, mut new) = (0, 0);
    for l in &lines {
        let (add, del) = (l.starts_with("+ "), l.starts_with("- "));
        if !add {
            old += 1;
        }
        if !del {
            new += 1;
        }
        numbers.push(((!add).then_some(old), (!del).then_some(new)));
    }
    let shown: Vec<usize> = (0..lines.len()).filter(|i| near(*i)).collect();
    let hunk = match (shown.first(), shown.last()) {
        (Some(first), Some(last)) => {
            let span = &numbers[*first..=*last];
            let o = span.iter().filter_map(|(o, _)| *o).collect::<Vec<_>>();
            let n = span.iter().filter_map(|(_, n)| *n).collect::<Vec<_>>();
            let (os, ns) = (o.first().copied().unwrap_or(0), n.first().copied().unwrap_or(0));
            Some((format!("@@ -{os},{} +{ns},{} @@", o.len(), n.len()), lines[*first].get(2..).unwrap_or("").trim().to_string()))
        }
        _ => None,
    };
    egui::Frame::NONE
        .fill(p.canvas)
        .stroke(Stroke::new(1.0, p.border))
        .corner_radius(CornerRadius::same(8))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            egui::ScrollArea::vertical().max_height(320.0).auto_shrink([false, true]).show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 0.0;
                let font = style::mono(style::MONO);
                let rw = ui.available_width();
                if let Some((head, context)) = &hunk {
                    let (r, _) = ui.allocate_exact_size(vec2(rw, 28.0), Sense::hover());
                    ui.painter().rect_filled(r, CornerRadius { nw: 7, ne: 7, sw: 0, se: 0 }, p.accent_tint);
                    let mut job = egui::text::LayoutJob::default();
                    job.append(head, 0.0, egui::TextFormat::simple(style::mono(style::DATA), p.text_faint));
                    job.append(context, 10.0, egui::TextFormat::simple(style::mono(style::DATA), p.text_muted));
                    job.wrap = egui::text::TextWrapping::truncate_at_width(rw - 24.0);
                    let g = ui.painter().layout_job(job);
                    ui.painter().galley(pos2(r.left() + 12.0, r.center().y - g.size().y / 2.0), g, p.text);
                }
                let fold_row = |ui: &mut Ui, n: usize| {
                    let (r, _) = ui.allocate_exact_size(vec2(rw, 26.0), Sense::hover());
                    ui.painter().rect_filled(r, CornerRadius::ZERO, p.hover);
                    ui.painter().text(pos2(r.left() + 12.0 + 7.0, r.center().y), Align2::CENTER_CENTER, icons::DOTS_THREE, style::icon_font(14.0), p.text_faint);
                    ui.painter().text(pos2(r.left() + 12.0 + 14.0 + 8.0, r.center().y), Align2::LEFT_CENTER, tf("{} unchanged lines", &[&n.to_string()]), FontId::proportional(style::SMALL), p.text_faint);
                };
                let mut folded = 0;
                for (i, line) in lines.iter().enumerate() {
                    let (sign, text) = line.split_at(line.len().min(2));
                    let add = sign == "+ ";
                    let del = sign == "- ";
                    if !add && !del && !near(i) {
                        folded += 1;
                        continue;
                    }
                    // The lines before the first shown: the hunk's header says where it starts.
                    if folded > 0 && (folded < i || hunk.is_none()) {
                        fold_row(ui, folded);
                    }
                    folded = 0;
                    let (r, _) = ui.allocate_exact_size(vec2(rw, 20.0), Sense::hover());
                    if !ui.is_rect_visible(r) {
                        continue;
                    }
                    let (bg, sc) = if add {
                        (style::soft(p.ok, if dark { 0.09 } else { 0.07 }), p.ok)
                    } else if del {
                        (style::soft(p.err, if dark { 0.09 } else { 0.06 }), p.err)
                    } else {
                        (Color32::TRANSPARENT, p.text_faint)
                    };
                    let painter = ui.painter();
                    painter.rect_filled(r, CornerRadius::ZERO, bg);
                    let cy = r.center().y;
                    let (o, n) = numbers[i];
                    // Old number, new number (30 each, right-aligned), the sign, the code.
                    if let Some(o) = o {
                        painter.text(pos2(r.left() + 4.0 + 30.0, cy), Align2::RIGHT_CENTER, o.to_string(), style::mono(style::DATA), p.text_faint);
                    }
                    if let Some(n) = n {
                        painter.text(pos2(r.left() + 4.0 + 30.0 + 8.0 + 30.0, cy), Align2::RIGHT_CENTER, n.to_string(), style::mono(style::DATA), p.text_faint);
                    }
                    let sx = r.left() + 4.0 + 30.0 + 8.0 + 30.0 + 8.0;
                    painter.text(pos2(sx, cy), Align2::LEFT_CENTER, if add { "+" } else if del { "−" } else { "" }, font.clone(), sc);
                    let clipped_text = match text.char_indices().nth(2_000) {
                        Some((cut, _)) => &text[..cut],
                        None => text,
                    };
                    let mut job = egui::text::LayoutJob::default();
                    for (c, piece) in crate::editor::line_colors(clipped_text, &syntax, dark, if add || del { p.text } else { p.text_muted }) {
                        job.append(&piece, 0.0, egui::TextFormat::simple(font.clone(), c));
                    }
                    let cx = sx + 12.0 + 8.0;
                    job.wrap = egui::text::TextWrapping::truncate_at_width((r.right() - 12.0 - cx).max(40.0));
                    let g = painter.layout_job(job);
                    painter.galley(pos2(cx, cy - g.size().y / 2.0), g, p.text);
                }
                if folded > 0 {
                    fold_row(ui, folded);
                }
            });
        });
}

/// Where Run Agent keeps, per preset, whether its agent reports no status (read once per
/// opening: `App::installed` reads a file).
const NO_HOOKS_ID: &str = "run-agent-no-hooks";

impl App {
    /// The Run Agent dialog: which agent (with how it tells TernaryCode that it needs you),
    /// whether to continue its last session, and where it runs. Returns whether it stays open.
    #[allow(clippy::too_many_arguments)]
    fn run_agent_dialog(&mut self, ui: &mut Ui, selected: &mut usize, session: &mut usize, place: &mut usize, wt: &mut crate::worktree::Form, enter: bool) -> bool {
        use crate::actions::Launch;
        let p = style::pal(ui);
        let dark = ui.visuals().dark_mode;
        let presets = self.presets();
        let here = self.run_dialog_idle_pane().is_some();
        let project = self.active_project().map(|x| (x.name.clone(), crate::paths::contract_tilde(&x.path), x.path.clone()));
        begin(ui, W_RUN);
        let mut keep = true;
        let sub = project.as_ref().map(|x| tf("in {} · {}", &[&x.0, &x.1]));
        if header(ui, icons::PLAY, p.accent, t("Run an agent"), sub.as_deref()) {
            keep = false;
        }
        // A new worktree is made of the project (of a worktree: of its project), a git one.
        let name_id = egui::Id::new("worktree-name");
        let naming = ui.memory(|m| m.has_focus(name_id));
        let root = self
            .active
            .and_then(|id| self.root_project(id))
            .filter(|r| r.git.is_some() && !r.missing)
            .map(|r| (r.id, r.name.clone(), r.path.clone(), r.git.as_ref().map_or(0, |g| g.changed)));
        if root.is_none() {
            wt.on = false;
        }
        // Digits choose the agent (not while a worktree's name is typed).
        let n = presets.len();
        let before = *selected;
        ui.input(|i| {
            for (k, d) in [(Key::Num1, 0), (Key::Num2, 1), (Key::Num3, 2), (Key::Num4, 3), (Key::Num5, 4), (Key::Num6, 5), (Key::Num7, 6), (Key::Num8, 7), (Key::Num9, 8)] {
                if d < n && i.key_pressed(k) && !naming {
                    *selected = d;
                }
            }
        });
        *selected = (*selected).min(n.saturating_sub(1));
        if *selected != before {
            *session = 0;
        }
        // Which presets' agents report no status (the "no hooks" beside their name).
        let ids: Vec<String> = presets.iter().map(|x| x.id.clone()).collect();
        let cache_id = egui::Id::new(NO_HOOKS_ID);
        let no_hooks: Vec<bool> = match ui.data(|d| d.get_temp::<Vec<(String, bool)>>(cache_id)) {
            Some(c) if c.iter().map(|(id, _)| id).eq(ids.iter()) => c.into_iter().map(|(_, b)| b).collect(),
            _ => {
                let list: Vec<(String, bool)> = presets
                    .iter()
                    .map(|x| (x.id.clone(), x.agent.is_some_and(|a| a != AgentKind::Custom && self.hooks_note(a) == t("no hooks"))))
                    .collect();
                ui.data_mut(|d| d.insert_temp(cache_id, list.clone()));
                list.into_iter().map(|(_, b)| b).collect()
            }
        };

        // The chosen agent's sessions: its earlier ones in this project when they can be listed,
        // else its last one when it has a command for that.
        enum Rows {
            Listing,
            Sessions(Vec<tc_agent::sessions::Session>),
            Last(String),
            Nothing,
        }
        let chosen = presets.get(*selected).cloned();
        let agent = chosen.as_ref().and_then(|x| x.agent);
        let resume_cmd = chosen.as_ref().and_then(|x| x.resume.clone()).filter(|r| !r.trim().is_empty());
        let rows = match (agent, project.as_ref()) {
            (Some(a), Some(x)) if tc_agent::sessions::listable(a) => match self.sessions(a, &x.2) {
                None => Rows::Listing,
                Some(list) => Rows::Sessions(list.to_vec()),
            },
            _ => match resume_cmd {
                Some(c) => Rows::Last(c),
                None => Rows::Nothing,
            },
        };
        let count = match &rows {
            Rows::Sessions(v) => v.len(),
            Rows::Last(_) => 1,
            _ => 0,
        };
        // ↑ ↓ choose the session.
        let mut moved = false;
        let sessions_shown = !wt.on;
        ui.input(|i| {
            if !sessions_shown {
                return;
            }
            if i.key_pressed(Key::ArrowDown) {
                *session = (*session + 1) % (count + 1);
                moved = true;
            }
            if i.key_pressed(Key::ArrowUp) {
                *session = (*session + count) % (count + 1);
                moved = true;
            }
        });
        *session = (*session).min(count);

        // The new worktree: what the repository has, a free name for the agent chosen, where
        // it starts from.
        let facts = root.as_ref().and_then(|r| self.worktree_facts(&r.2).cloned());
        if let Some(r) = &root
            && !wt.typed
        {
            let stem = chosen.as_ref().map(|x| x.agent.map_or(x.name.clone(), |a| a.slug().to_string())).unwrap_or_default();
            wt.name = self.suggest_worktree_name(r.0, &stem);
        }
        let base = wt.base.clone().or_else(|| facts.as_ref().and_then(|f| f.current.clone())).unwrap_or_default();
        let problem = root.as_ref().and_then(|r| self.worktree_name_problem(r.0, &wt.name));
        let wt_ready = facts.is_some() && problem.is_none() && !base.is_empty();
        let mut edit_setup = false;
        // What will be typed, exactly, and where.
        let command_line = chosen.as_ref().map(|preset| {
            let base_cmd = match (&rows, *session, wt.on) {
                (Rows::Sessions(v), k, false) if k > 0 => v
                    .get(k - 1)
                    .and_then(|s| agent.and_then(|a| tc_agent::sessions::resume_command(a, &preset.command, &s.id)))
                    .unwrap_or_else(|| preset.command.clone()),
                (Rows::Last(c), 1, false) => c.clone(),
                _ => preset.command.clone(),
            };
            let mut line = self.launch_command(preset, &base_cmd);
            if wt.on
                && let Some(r) = &root
            {
                let setup_run = self.worktree_setup_of(r.0).run;
                let setup_run = setup_run.trim();
                if !setup_run.is_empty() {
                    line = format!("{setup_run} && {line}");
                }
            }
            line
        });
        // A new session's line: its command with the values of its flags left out.
        let new_session_line = chosen
            .as_ref()
            .map(|x| self.launch_command(x, &x.command).split(' ').map(|w| if w.trim_matches('"').starts_with('$') { "…" } else { w }).collect::<Vec<_>>().join(" "))
            .unwrap_or_default();

        let mut run = enter;
        body(ui, |ui| {
            let inner_w = ui.available_width();
            let list_w = RUN_AGENT_LIST_W.min((inner_w * 0.32).round());
            ui.horizontal_top(|ui| {
                ui.spacing_mut().item_spacing = vec2(20.0, 0.0);
                // ----- the agents, in an inset well -------------------------------------------
                egui::Frame::NONE.fill(p.inset).corner_radius(CornerRadius::same(10)).inner_margin(Margin::same(6)).show(ui, |ui| ui.vertical(|ui| {
                    ui.set_width(list_w - 12.0);
                    egui::ScrollArea::vertical().id_salt("run-agent-presets").max_height(420.0).auto_shrink([false, true]).show(ui, |ui| {
                        ui.set_width(list_w - 12.0);
                        ui.spacing_mut().item_spacing.y = 2.0;
                        for (i, preset) in presets.iter().enumerate() {
                            let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 52.0), Sense::click());
                            #[cfg(debug_assertions)]
                            crate::devtest::record(format!("agent:{}", preset.id), rect);
                            let on = i == *selected;
                            let painter = ui.painter();
                            if on {
                                painter.rect_filled(rect.translate(vec2(0.0, 1.0)), CornerRadius::same(9), Color32::from_black_alpha(if dark { 64 } else { 18 }));
                                painter.rect(rect, CornerRadius::same(9), p.surface, Stroke::new(1.0, p.border), StrokeKind::Inside);
                            } else if resp.hovered() {
                                painter.rect_filled(rect, CornerRadius::same(9), p.hover);
                            }
                            let cy = rect.center().y;
                            let tile = Rect::from_center_size(pos2(rect.left() + 10.0 + 14.0, cy), vec2(28.0, 28.0));
                            paint_agent_avatar(painter, tile, preset.agent.filter(|a| *a != AgentKind::Custom), &p);
                            let x = tile.right() + 12.0;
                            let mut right = rect.right() - 10.0;
                            if i < 9 {
                                let key = (i + 1).to_string();
                                let kw = kit::kbd_width(painter, &key);
                                kit::paint_kbd_text(painter, pos2(right - kw, cy), &key, &p);
                                right -= kw + 12.0;
                            }
                            if no_hooks.get(i).copied().unwrap_or(false) {
                                let g = painter.layout_no_wrap(t("no hooks").to_string(), FontId::proportional(style::SMALL), p.warn);
                                right -= g.size().x;
                                painter.galley(pos2(right, cy - g.size().y / 2.0), g, p.warn);
                                right -= 12.0;
                            }
                            let mut job = egui::text::LayoutJob::simple_singleline(preset.name.clone(), style::medium(style::BODY), p.text);
                            job.wrap = egui::text::TextWrapping::truncate_at_width((right - x).max(20.0));
                            let g = painter.layout_job(job);
                            painter.galley(pos2(x, cy - 17.0 + (18.0 - g.size().y) / 2.0), g, p.text);
                            let mut job = egui::text::LayoutJob::simple_singleline(preset.command.clone(), style::mono(style::DATA), p.text_faint);
                            job.wrap = egui::text::TextWrapping::truncate_at_width((right - x).max(20.0));
                            let g = painter.layout_job(job);
                            painter.galley(pos2(x, cy + 3.0 + (14.0 - g.size().y) / 2.0), g, p.text_faint);
                            if resp.hovered() {
                                ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                            }
                            if resp.clicked() && *selected != i {
                                *selected = i;
                                *session = 0;
                            }
                            if resp.double_clicked() {
                                run = true;
                            }
                        }
                        // A preset of your own: Settings → Agents.
                        let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 36.0), Sense::click());
                        if resp.hovered() {
                            ui.painter().rect_filled(rect, CornerRadius::same(9), p.hover);
                            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                        }
                        let c = if resp.hovered() { p.text } else { p.text_muted };
                        ui.painter().text(pos2(rect.left() + 12.0 + 7.0, rect.center().y), Align2::CENTER_CENTER, icons::PLUS, style::icon_font(14.0), p.text_faint);
                        ui.painter().text(pos2(rect.left() + 12.0 + 14.0 + 8.0, rect.center().y), Align2::LEFT_CENTER, t("Add a preset…"), FontId::proportional(style::SMALL), c);
                        if resp.clicked() {
                            self.actions.push(Action::Integrations);
                            keep = false;
                        }
                    });
                }));
                // ----- what to run --------------------------------------------------------------
                ui.vertical(|ui| {
                    let w = (inner_w - list_w - 20.0).max(200.0);
                    ui.set_width(w);
                    ui.spacing_mut().item_spacing.y = 0.0;
                    let mut first_row = true;
                    // A row: its label (96) at the left, the control and a note under it.
                    let mut option_row = |ui: &mut Ui, label: &str, add: &mut dyn FnMut(&mut Ui)| {
                        if !first_row {
                            let y = ui.cursor().top();
                            ui.painter().hline(ui.max_rect().x_range(), y + 0.5, Stroke::new(1.0, p.border));
                        }
                        first_row = false;
                        egui::Frame::NONE.inner_margin(Margin::symmetric(0, 12)).show(ui, |ui| {
                            ui.set_width(ui.available_width());
                            ui.horizontal_top(|ui| {
                                ui.spacing_mut().item_spacing.x = 12.0;
                                let (r, _) = ui.allocate_exact_size(vec2(96.0, 16.0), Sense::hover());
                                ui.painter().text(r.left_center(), Align2::LEFT_CENTER, label, FontId::proportional(style::SMALL), p.text_muted);
                                ui.vertical(|ui| {
                                    ui.set_width(ui.available_width());
                                    ui.spacing_mut().item_spacing.y = 6.0;
                                    add(ui);
                                });
                            });
                        });
                    };
                    // Where it works: the project's folder, or a new worktree (git projects).
                    if let Some(r) = &root {
                        option_row(ui, t("Work in"), &mut |ui: &mut Ui| {
                            let options = [kit::Seg::new(t("This folder"), "").icon(icons::FOLDER_SIMPLE), kit::Seg::new(t("New worktree"), "").icon(icons::GIT_FORK)];
                            if let Some(i) = kit::segmented(ui, &options, wt.on as usize) {
                                wt.on = i == 1;
                                if let Some(project) = self.project_mut(r.0) {
                                    project.new_worktree = wt.on;
                                }
                                self.mark_dirty();
                            }
                            if wt.on {
                                note(ui, t("A folder and a branch of its own, so agents in one repository don’t overwrite each other."));
                            }
                        });
                    }
                    if wt.on
                        && let Some(r) = &root
                    {
                        option_row(ui, t("Worktree"), &mut |ui: &mut Ui| {
                            ui.spacing_mut().item_spacing.y = 8.0;
                            edit_setup = self.worktree_box(ui, r.0, &r.1, r.3, wt, &base, facts.as_ref(), problem.as_deref(), name_id);
                        });
                    } else {
                        // The session: a new one, or one to continue.
                        option_row(ui, t("Session"), &mut |ui: &mut Ui| {
                            egui::ScrollArea::vertical().id_salt("run-agent-sessions").max_height(226.0).auto_shrink([false, true]).show(ui, |ui| {
                                ui.spacing_mut().item_spacing.y = 2.0;
                                // One row: a radio, two lines, and a pill at the right.
                                let mut row = |ui: &mut Ui, index: usize, title: &str, sub: &str, pill: bool| {
                                    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 48.0), Sense::click());
                                    #[cfg(debug_assertions)]
                                    crate::devtest::record(format!("session:{index}"), rect);
                                    let on = index == *session;
                                    if on && moved {
                                        ui.scroll_to_rect(rect, None);
                                    }
                                    let painter = ui.painter_at(rect);
                                    if on {
                                        painter.rect_filled(rect, CornerRadius::same(8), p.accent_tint);
                                    } else if resp.hovered() {
                                        painter.rect_filled(rect, CornerRadius::same(8), p.hover);
                                    }
                                    let cy = rect.center().y;
                                    // The radio (14).
                                    let rc = pos2(rect.left() + 12.0 + 7.0, cy);
                                    if on {
                                        painter.circle_filled(rc, 7.0, p.accent);
                                        painter.circle_filled(rc, 3.0, Color32::WHITE);
                                    } else {
                                        painter.circle_stroke(rc, 6.25, Stroke::new(1.5, p.border_heavy));
                                    }
                                    let x = rect.left() + 12.0 + 14.0 + 12.0;
                                    let mut right = rect.right() - 12.0;
                                    if pill {
                                        let text = t("Open in a pane");
                                        let w = kit::state_pill_width(&painter, tc_core::status::Status::Running, text);
                                        kit::paint_state_pill(&painter, pos2(right - w, cy), tc_core::status::Status::Running, text, &p, dark);
                                        right -= w + 12.0;
                                    }
                                    let font = if on { style::medium(style::BODY) } else { FontId::proportional(style::BODY) };
                                    let mut job = egui::text::LayoutJob::simple_singleline(title.to_string(), font, p.text);
                                    job.wrap = egui::text::TextWrapping::truncate_at_width((right - x).max(20.0));
                                    let g = painter.layout_job(job);
                                    painter.galley(pos2(x, cy - 17.0 + (18.0 - g.size().y) / 2.0), g, p.text);
                                    let mut job = egui::text::LayoutJob::simple_singleline(sub.to_string(), style::mono(style::DATA), p.text_faint);
                                    job.wrap = egui::text::TextWrapping::truncate_at_width((right - x).max(20.0));
                                    let g = painter.layout_job(job);
                                    painter.galley(pos2(x, cy + 3.0 + (14.0 - g.size().y) / 2.0), g, p.text_faint);
                                    if resp.hovered() && !on {
                                        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                                    }
                                    if resp.clicked() {
                                        *session = index;
                                    }
                                    if resp.double_clicked() {
                                        run = true;
                                    }
                                };
                                row(ui, 0, t("New session"), &new_session_line, false);
                                let quiet = |ui: &mut Ui, text: &str| {
                                    let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), 28.0), Sense::hover());
                                    ui.painter().text(pos2(r.left() + 12.0 + 14.0 + 12.0, r.center().y), Align2::LEFT_CENTER, text, FontId::proportional(style::SMALL), p.text_faint);
                                };
                                match &rows {
                                    Rows::Listing => quiet(ui, t("Looking for sessions…")),
                                    Rows::Sessions(v) if v.is_empty() => quiet(ui, t("No earlier sessions in this project.")),
                                    Rows::Sessions(v) => {
                                        for (k, s) in v.iter().enumerate() {
                                            let mut sub = vec![tf("{} ago", &[&ago(s.updated)])];
                                            if let Some(b) = &s.branch {
                                                sub.push(b.clone());
                                            }
                                            sub.push(s.id.chars().take(8).collect());
                                            // Already open in a pane: choosing it goes there.
                                            let open = self.panes.values().any(|x| x.agent_session.as_deref() == Some(s.id.as_str()));
                                            row(ui, k + 1, &s.title, &sub.join(" · "), open);
                                        }
                                    }
                                    Rows::Last(cmd) => row(ui, 1, t("Last session"), cmd, false),
                                    Rows::Nothing => quiet(ui, t("This agent has no command that resumes a session.")),
                                }
                            });
                        });
                        option_row(ui, t("Open in"), &mut |ui: &mut Ui| {
                            // "This tab": a shell at its prompt with nothing running or typed takes the agent.
                            let this_tip = if here { "" } else { t("No shell here is idle at its prompt") };
                            let options = [
                                kit::Seg::new(t("This tab"), this_tip).enabled(here),
                                kit::Seg::new(t("New tab"), ""),
                                kit::Seg::new(t("Split right"), ""),
                                kit::Seg::new(t("Split down"), ""),
                            ];
                            // Place 0 is this tab when its shell is idle, else a new tab; 3 is
                            // always a new tab.
                            let on = match *place {
                                0 if here => 0,
                                0 | 3 => 1,
                                1 => 2,
                                _ => 3,
                            };
                            match kit::segmented(ui, &options, on) {
                                Some(0) if here => *place = 0,
                                Some(1) => *place = 3,
                                Some(2) => *place = 1,
                                Some(3) => *place = 2,
                                _ => {}
                            }
                        });
                    }
                    option_row(ui, t("Permissions"), &mut |ui: &mut Ui| {
                        self.skip_prompts_row(ui, agent);
                    });
                    // How its status reaches TernaryCode.
                    let kind = agent.unwrap_or(AgentKind::Custom);
                    let hooks = self.hooks_note(kind);
                    let hooked = self.installed(kind).is_some()
                        || (matches!(kind, AgentKind::Claude | AgentKind::Codex)
                            && self.settings.integrations.get(kind.slug()).copied().unwrap_or_default() == tc_core::model::Integration::PerLaunch);
                    option_row(ui, t("Status"), &mut |ui: &mut Ui| {
                        ui.horizontal_top(|ui| {
                            ui.spacing_mut().item_spacing.x = 8.0;
                            let name = chosen.as_ref().map(|x| x.name.clone()).unwrap_or_default();
                            let (status, text, note_text) = if hooked {
                                let label = tf("Hooks · {}", &[hooks.trim_start_matches("hooks · ")]);
                                (Some(tc_core::status::Status::Done), label, tf("Needs input, Done and Failed come from {} itself.", &[&name]))
                            } else if kind == AgentKind::Custom {
                                (None, t("Custom").to_string(), t("The status comes from the terminal.").to_string())
                            } else {
                                (Some(tc_core::status::Status::Unknown), t("No hooks · shows “Active”").to_string(), t("Settings → Agents connects it.").to_string())
                            };
                            match status {
                                Some(s) => {
                                    let w = kit::state_pill_width(ui.painter(), s, &text);
                                    let (r, _) = ui.allocate_exact_size(vec2(w, kit::PILL_H), Sense::hover());
                                    kit::paint_state_pill(ui.painter(), r.left_center(), s, &text, &p, dark);
                                }
                                None => {
                                    kit::pill(ui, None, &text, p.text_muted, p.accent_soft);
                                }
                            }
                            ui.add(egui::Label::new(RichText::new(note_text).size(style::SMALL).color(p.text_faint).line_height(Some(20.0))).wrap());
                        });
                    });
                });
            });
            if let Some(line) = &command_line {
                let whereto = if wt.on {
                    t("a new tab of the new worktree").to_string()
                } else {
                    match (*place, here) {
                        (0, true) => t("this tab, its shell is idle").to_string(),
                        (0 | 3, _) => t("a new tab").to_string(),
                        (1, _) => t("a split at the right").to_string(),
                        _ => t("a split below").to_string(),
                    }
                };
                run_command_bar(ui, line, &whereto);
            }
        });
        if edit_setup && let Some(r) = &root {
            self.pending_setup = Some(r.0);
        }
        let name = chosen.as_ref().map(|x| x.name.clone()).unwrap_or_default();
        let digits = n.clamp(1, 9).to_string();
        let (hint, label) = if wt.on {
            (tf("1–{} agent · ↵ create and run", &[&digits]), t("Create worktree and run").to_string())
        } else {
            (tf("1–{} agent · ↑↓ session · ↵ run", &[&digits]), if *session > 0 { tf("Resume {}", &[&name]) } else { tf("Run {}", &[&name]) })
        };
        let enabled = !wt.on || wt_ready;
        footer_with(ui, Some(&hint), |ui| {
            let r = ui.add_enabled_ui(enabled, |ui| kit::button_keys(ui, kit::Kind::Primary, None, &label, Some("↵"), false)).inner;
            #[cfg(debug_assertions)]
            crate::devtest::record("dialog-ok", r.rect);
            run |= r.clicked();
            if kit::secondary(ui, None, t("Cancel")).clicked() {
                keep = false;
            }
        });
        if run && wt.on {
            if wt_ready && let Some(preset) = &chosen {
                self.actions.push(Action::NewWorktree { preset: preset.id.clone(), name: wt.name.trim().to_string(), base });
                keep = false;
            }
            return keep;
        }
        if run && let Some(preset) = &chosen {
            let launch = match (&rows, *session) {
                (_, 0) => Launch::New,
                (Rows::Sessions(v), k) => v.get(k - 1).map_or(Launch::New, |s| Launch::Session(s.id.clone())),
                (Rows::Last(_), _) => Launch::Last,
                _ => Launch::New,
            };
            let dispatch_place = if *place == 0 && self.idle_command_pane().is_none() && self.run_dialog_idle_pane().is_some() {
                4
            } else {
                *place
            };
            self.actions.push(Action::RunAgent(preset.id.clone(), launch, dispatch_place));
            keep = false;
        }
        keep
    }
}

/// Run Agent's agents' well.
const RUN_AGENT_LIST_W: f32 = 260.0;

/// The bar under Run Agent's columns: what will be typed (`$ claude --settings …`) and where.
fn run_command_bar(ui: &mut Ui, line: &str, whereto: &str) {
    let p = style::pal(ui);
    let (bar, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 36.0), Sense::hover());
    let painter = ui.painter_at(bar);
    painter.rect_filled(bar, CornerRadius::same(8), p.inset);
    let cy = bar.center().y;
    let where_g = painter.layout_no_wrap(whereto.to_string(), FontId::proportional(style::SMALL), p.text_faint);
    let ww = where_g.size().x;
    painter.galley(pos2(bar.right() - 12.0 - ww, cy - where_g.size().y / 2.0), where_g, p.text_faint);
    // "$", the program bright, its flags quieter.
    let mut job = egui::text::LayoutJob::default();
    job.append("$", 0.0, egui::TextFormat::simple(style::mono(style::MONO), p.text_faint));
    let (program, rest) = line.split_once(' ').unwrap_or((line, ""));
    job.append(program, 8.0, egui::TextFormat::simple(style::mono(style::MONO), p.text));
    if !rest.is_empty() {
        job.append(rest, 8.0, egui::TextFormat::simple(style::mono(style::DATA), p.text_faint));
    }
    job.wrap = egui::text::TextWrapping::truncate_at_width((bar.width() - 24.0 - ww - 16.0).max(40.0));
    let g = painter.layout_job(job);
    painter.galley(pos2(bar.left() + 12.0, cy - g.size().y / 2.0), g, p.text);
    resp.on_hover_text(line);
}

/// How long ago, short: "just now", "5 min", "3 h", "2 d" (shown as "… ago").
pub(crate) fn ago(at: std::time::SystemTime) -> String {
    let s = std::time::SystemTime::now().duration_since(at).map(|d| d.as_secs()).unwrap_or(0);
    match s {
        0..60 => t("a moment").to_string(),
        60..3600 => tf("{} min", &[&(s / 60).to_string()]),
        3600..86400 => tf("{} h", &[&(s / 3600).to_string()]),
        _ => tf("{} d", &[&(s / 86400).to_string()]),
    }
}

impl App {
    /// Run Agent found no program for a preset: how to install it (Install in terminal types the
    /// command), or Run anyway (an alias or a function the check can't see).
    #[allow(clippy::too_many_arguments)]
    fn agent_missing_dialog(
        &mut self,
        ui: &mut Ui,
        preset: &str,
        launch: &crate::actions::Launch,
        place: Option<usize>,
        profile: Option<&str>,
        program: &str,
        npm: bool,
        enter: bool,
    ) -> bool {
        let p = style::pal(ui);
        begin(ui, W_FORM);
        let mut keep = true;
        let found = self.presets().into_iter().find(|x| x.id == preset);
        let name = found.as_ref().map_or_else(|| program.to_string(), |x| x.name.clone());
        let install = found.as_ref().and_then(|x| x.agent).and_then(tc_core::agent::install_command);
        let brew = cfg!(target_os = "macos") && ["/opt/homebrew/bin/brew", "/usr/local/bin/brew"].iter().any(|b| std::path::Path::new(b).exists());
        let node = if npm { tc_core::agent::node_install_command(brew) } else { None };
        // What Install in terminal types: Node.js first when npm is missing, else the agent.
        let now = if npm { node } else { install };
        if header(ui, icons::DOWNLOAD_SIMPLE, p.warn, &tf("{} isn’t installed", &[&name]), None) {
            keep = false;
        }
        let mut copied = None;
        body(ui, |ui| {
            let text = tf("TernaryCode starts “{}” from your PATH, and it isn’t there. Install it, then start the agent again — no restart needed.", &[program]);
            ui.add(egui::Label::new(RichText::new(text).size(style::SMALL).color(p.text_muted).line_height(Some(16.0))).wrap());
            if npm {
                let text = if node.is_some() {
                    t("It is installed with npm, which comes with Node.js: install Node.js first.")
                } else {
                    t("It is installed with npm, which comes with Node.js: install Node.js first, from nodejs.org.")
                };
                ui.horizontal_top(|ui| {
                    ui.spacing_mut().item_spacing.x = 8.0;
                    ui.label(RichText::new(icons::WARNING).font(style::icon_fill(14.0)).color(p.warn));
                    ui.add(egui::Label::new(RichText::new(text).size(style::SMALL).color(p.text).line_height(Some(16.0))).wrap());
                });
            }
            for command in [node, install].into_iter().flatten() {
                if command_box(ui, command) {
                    copied = Some(command);
                }
            }
        });
        if let Some(command) = copied {
            ui.ctx().copy_text(command.to_string());
            self.toast_ok(t("Install command copied.").into());
        }
        let mut install_now = false;
        let (mut anyway, mut cancel) = (false, false);
        footer_left(
            ui,
            |ui| {
                let r = hint_link(ui, t("Run anyway"));
                #[cfg(debug_assertions)]
                crate::devtest::record("dialog-anyway", r.rect);
                anyway = r.clicked();
            },
            |ui| {
                if now.is_some() {
                    let r = kit::button_keys(ui, kit::Kind::Primary, None, t("Install in terminal"), Some("↵"), false);
                    #[cfg(debug_assertions)]
                    crate::devtest::record("dialog-ok", r.rect);
                    install_now = r.clicked();
                }
                cancel = kit::secondary(ui, None, t("Cancel")).clicked();
            },
        );
        if cancel {
            keep = false;
        }
        if anyway {
            self.actions.push(Action::RunPresetAnyway(preset.to_string(), launch.clone(), place, profile.map(str::to_string)));
            keep = false;
        }
        if let Some(command) = now
            && (install_now || enter)
        {
            self.actions.push(Action::TypeInTerminal(command.to_string()));
            keep = false;
        }
        keep
    }
}

/// A command in an inset box with a copy button; true when it is clicked (to copy it).
fn command_box(ui: &mut Ui, command: &str) -> bool {
    let p = style::pal(ui);
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 38.0), Sense::click());
    let resp = resp.on_hover_text(t("Copy"));
    if resp.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    ui.painter().rect(rect, CornerRadius::same(8), p.inset, Stroke::new(1.0, p.border), StrokeKind::Inside);
    let cy = rect.center().y;
    let button = Rect::from_min_size(pos2(rect.right() - 6.0 - 24.0, cy - 12.0), vec2(24.0, 24.0));
    let mut job = egui::text::LayoutJob::default();
    job.append("$", 0.0, egui::TextFormat::simple(style::mono(style::MONO), p.text_faint));
    job.append(command, 8.0, egui::TextFormat::simple(style::mono(style::MONO), p.text));
    job.wrap = egui::text::TextWrapping::truncate_at_width((button.left() - 8.0 - rect.left() - 12.0).max(40.0));
    let g = ui.painter().layout_job(job);
    ui.painter().galley(pos2(rect.left() + 12.0, cy - g.size().y / 2.0), g, p.text);
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(button).id_salt(("command-copy", command)));
    let copy = kit::icon_button_ex(&mut child, icons::COPY, t("Copy"), false, 24.0);
    resp.clicked() || copy.clicked()
}
