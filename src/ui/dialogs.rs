//! Modal dialogs: add project, quit and remove confirmations (FR-L2, FR-P6), rename, the diff
//! shown before installing hooks, and messages.
//!
//! Every dialog has the same parts: a header (an icon in a soft circle, the title, a line
//! saying what happens, a close button), the body with its fields, and a footer with a hint at
//! the left and the buttons at the right (Esc and a click outside cancel, Enter confirms).

use eframe::egui::{self, Color32, CornerRadius, Key, Margin, RichText, Stroke, Ui};

use super::{icons, kit, style};
use crate::actions::Action;
use crate::app::{App, Busy, Dialog};
use crate::i18n::{t, tf};

/// Space between the dialog's edge and its content.
const PAD: i8 = 20;

/// The card every dialog sits on (its parts add their own margins): the overlay colour, a
/// line, rounded 14, a deep shadow.
pub fn frame(ctx: &egui::Context) -> egui::Frame {
    let dark = ctx.global_style().visuals.dark_mode;
    let p = style::palette(dark);
    egui::Frame::NONE
        .fill(p.overlay)
        .stroke(Stroke::new(1.0, p.border_strong))
        .corner_radius(CornerRadius::same(style::RADIUS_DIALOG))
        .shadow(egui::epaint::Shadow {
            offset: [0, 30],
            blur: 80,
            spread: 0,
            color: Color32::from_black_alpha(if dark { 166 } else { 46 }),
        })
}

/// The dimming behind a dialog.
pub fn scrim(dark: bool) -> Color32 {
    if dark { Color32::from_black_alpha(128) } else { Color32::from_rgba_premultiplied(4, 4, 4, 56) }
}

/// The square an icon sits in at the head of a dialog: 34, rounded 10, tinted by its colour
/// (neutral for the text colour).
pub fn paint_dialog_icon(ui: &mut Ui, icon: &str, color: Color32) -> egui::Response {
    let p = style::pal(ui);
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(34.0, 34.0), egui::Sense::hover());
    let neutral = color == p.text || color == p.text_muted;
    let (fill, line, ink) = if neutral {
        (p.surface, p.border_strong, p.text_muted)
    } else {
        (style::soft(color, 0.12), style::soft(color, 0.25), color)
    };
    ui.painter().rect(rect, CornerRadius::same(10), fill, Stroke::new(1.0, line), egui::StrokeKind::Inside);
    ui.painter().text(rect.center(), egui::Align2::CENTER_CENTER, icon, style::icon_font(17.0), ink);
    resp
}

/// A dialog's header. Returns whether its close button was clicked.
pub fn header(ui: &mut Ui, icon: &str, color: Color32, title: &str, sub: Option<&str>) -> bool {
    let p = style::pal(ui);
    let mut close = false;
    // The light along the dialog's top edge.
    let top = ui.max_rect();
    kit::paint_highlight(ui.painter(), egui::Rect::from_min_size(top.min, egui::vec2(top.width(), 1.0)), style::RADIUS_DIALOG, &p);
    egui::Frame::NONE
        .inner_margin(Margin { left: PAD, right: 14, top: 18, bottom: 6 })
        .show(ui, |ui| {
            ui.horizontal_top(|ui| {
                paint_dialog_icon(ui, icon, color);
                ui.add_space(14.0);
                let text_w = (ui.available_width() - 36.0).max(120.0);
                ui.vertical(|ui| {
                    ui.set_width(text_w);
                    ui.add_space(if sub.is_some() { 1.0 } else { 8.0 });
                    let mut job = egui::text::LayoutJob::default();
                    let mut f = egui::TextFormat::simple(style::semibold(15.0), p.text);
                    f.extra_letter_spacing = -0.15;
                    job.append(title, 0.0, f);
                    job.wrap.max_width = text_w;
                    ui.label(job);
                    if let Some(sub) = sub {
                        ui.add_space(4.0);
                        ui.spacing_mut().item_spacing.y = 2.0;
                        ui.add(
                            egui::Label::new(RichText::new(sub).size(12.5).color(p.text_muted).line_height(Some(18.75))).wrap(),
                        );
                    }
                });
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Min), |ui| {
                    close = kit::icon_button_ex(ui, icons::X, t("Close"), false, 22.0).clicked();
                });
            });
        });
    close
}

/// The content between the header and the footer.
pub fn body<R>(ui: &mut Ui, add: impl FnOnce(&mut Ui) -> R) -> R {
    egui::Frame::NONE
        .inner_margin(Margin { left: PAD, right: PAD, top: 12, bottom: 18 })
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 6.0;
            // Fields sit below the card (sunken), so they read as places to type.
            let p = style::pal(ui);
            ui.visuals_mut().text_edit_bg_color = Some(p.inset);
            ui.visuals_mut().extreme_bg_color = p.inset;
            add(ui)
        })
        .inner
}

/// A label over a field that takes the whole width.
pub(super) fn field_label(ui: &mut Ui, text: &str) {
    let p = style::pal(ui);
    ui.label(RichText::new(text).font(style::medium(12.0)).color(p.text_muted));
}

/// A single-line text field the width of the dialog (minus `leave` for what follows it).
fn text_field(ui: &mut Ui, text: &mut String, hint: &str, leave: f32) -> egui::Response {
    let id = ui.next_auto_id();
    let w = (ui.available_width() - leave).max(80.0);
    kit::field(ui, id, text, None, hint, w, false)
}

/// A field in the data font (a folder, a command), `w` wide.
fn mono_field(ui: &mut Ui, id: egui::Id, text: &mut String, hint: &str, w: f32) -> egui::Response {
    let p = style::pal(ui);
    let (rect, outer) = ui.allocate_exact_size(egui::vec2(w, 30.0), egui::Sense::click());
    if outer.clicked() {
        ui.memory_mut(|m| m.request_focus(id));
    }
    let focused = ui.ctx().memory(|m| m.has_focus(id));
    if focused {
        ui.painter().rect_stroke(rect.expand(3.0), CornerRadius::same(10), Stroke::new(3.0, p.accent_tint), egui::StrokeKind::Inside);
    }
    let fill = if ui.visuals().dark_mode { p.inset } else { Color32::WHITE };
    ui.painter().rect(rect, CornerRadius::same(7), fill, Stroke::new(1.0, if focused { p.accent } else { p.border_strong }), egui::StrokeKind::Inside);
    let inner = rect.shrink2(egui::vec2(10.0, 1.0));
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(inner).layout(egui::Layout::left_to_right(egui::Align::Center)));
    child.add_sized(
        inner.size(),
        egui::TextEdit::singleline(text)
            .id(id)
            .frame(egui::Frame::NONE)
            .margin(Margin::ZERO)
            .font(style::mono(12.0))
            .hint_text(RichText::new(hint).font(style::mono(12.0)).color(p.text_ghost))
            .vertical_align(egui::Align::Center),
    )
}


#[derive(Clone, Copy, PartialEq, Eq)]
enum Confirm {
    Primary,
    Danger,
}

/// The footer: a line, `hint` at the left, Cancel and the main button at the right. Returns
/// (cancel, ok).
/// The main button asks: ↵ on it, Cancel quiet. It destroys something: no ↵ (Enter does
/// nothing), Cancel outlined as the safe choice, and "esc cancel" at the left unless `hint`.
fn footer(ui: &mut Ui, hint: Option<&str>, ok: &str, kind: Confirm, enabled: bool) -> (bool, bool) {
    let mut out = (false, false);
    let esc = kind == Confirm::Danger && hint.is_none();
    footer_left(
        ui,
        |ui| {
            if esc {
                esc_hint(ui);
            } else if let Some(hint) = hint {
                let p = style::pal(ui);
                ui.label(RichText::new(hint).size(style::LABEL).color(p.text_faint));
            }
        },
        |ui| {
            let okr = ui
                .add_enabled_ui(enabled, |ui| match kind {
                    Confirm::Primary => kit::button_keys(ui, kit::Kind::Primary, None, ok, Some("↵"), false),
                    Confirm::Danger => kit::button_ex(ui, kit::Kind::DangerFilled, None, ok, false),
                })
                .inner;
            #[cfg(debug_assertions)]
            crate::devtest::record("dialog-ok", okr.rect);
            out.1 = okr.clicked();
            let cancel = if kind == Confirm::Danger { kit::Kind::Secondary } else { kit::Kind::Ghost };
            out.0 = kit::button_ex(ui, cancel, None, t("Cancel"), false).clicked();
        },
    );
    out
}

/// "esc cancel" at a footer's left: the key, then the word.
pub fn esc_hint(ui: &mut Ui) {
    let p = style::pal(ui);
    ui.spacing_mut().item_spacing.x = 6.0;
    kit::kbd(ui, "esc");
    ui.label(RichText::new(t("cancel")).size(style::LABEL).color(p.text_faint));
}

/// A footer with its own buttons: a line, `hint` at the left, and what `buttons` adds at the
/// right (right to left: the main button first).
pub fn footer_with(ui: &mut Ui, hint: Option<&str>, buttons: impl FnOnce(&mut Ui)) {
    footer_left(
        ui,
        |ui| {
            if let Some(hint) = hint {
                let p = style::pal(ui);
                ui.label(RichText::new(hint).size(style::LABEL).color(p.text_faint));
            }
        },
        buttons,
    );
}

/// A footer with what `left` adds at its left (a hint, a secondary choice) and `buttons` at
/// its right (right to left: the main button first).
pub fn footer_left(ui: &mut Ui, left: impl FnOnce(&mut Ui), buttons: impl FnOnce(&mut Ui)) {
    let p = style::pal(ui);
    let full = ui.max_rect();
    let y = ui.cursor().top();
    ui.painter().hline(full.x_range(), y, Stroke::new(1.0, p.border));
    egui::Frame::NONE
        .fill(style::over(p.overlay, p.chrome, 0.35))
        .corner_radius(CornerRadius {
            nw: 0,
            ne: 0,
            sw: style::RADIUS_DIALOG - 1,
            se: style::RADIUS_DIALOG - 1,
        })
        .inner_margin(Margin { left: PAD, right: 14, top: 12, bottom: 12 })
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                left(ui);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.spacing_mut().item_spacing.x = 8.0;
                    buttons(ui);
                });
            });
        });
}

/// Programs that will be stopped and files not saved, in a quiet box.
fn busy_list(ui: &mut Ui, items: &[Busy]) {
    let p = style::pal(ui);
    egui::Frame::NONE
        .fill(p.canvas)
        .stroke(Stroke::new(1.0, p.border))
        .corner_radius(CornerRadius::same(style::RADIUS + 1))
        .inner_margin(Margin::symmetric(10, 6))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            egui::ScrollArea::vertical().max_height(180.0).show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 0.0;
                for b in items {
                    let file = b.file;
                    let (r, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 24.0), egui::Sense::hover());
                    let icon = if file { icons::FILE_TEXT } else { icons::TERMINAL_WINDOW };
                    ui.painter().text(egui::pos2(r.left() + 6.5, r.center().y), egui::Align2::CENTER_CENTER, icon, style::icon_font(13.0), if file { p.warn } else { p.text_faint });
                    let text = if file { tf("{} · not saved", &[&b.text]) } else { b.text.clone() };
                    let mut job = egui::text::LayoutJob::simple_singleline(text, egui::FontId::proportional(12.5), p.text);
                    job.wrap = egui::text::TextWrapping::truncate_at_width((r.width() - 24.0).max(20.0));
                    let g = ui.painter().layout_job(job);
                    ui.painter().galley(egui::pos2(r.left() + 13.0 + 8.0, r.center().y - g.size().y / 2.0), g, p.text);
                }
            });
        });
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
                    ui.set_width(520.0);
                    if header(ui, icons::FOLDER_PLUS, p.accent, t("Add Project"), Some(t("Its terminals start in this folder."))) {
                        keep = false;
                    }
                    let presets = self.presets();
                    let next_color = {
                        let (r, g, b) = crate::project::COLORS[self.projects.len() % crate::project::COLORS.len()];
                        Color32::from_rgb(r, g, b)
                    };
                    body(ui, |ui| {
                        ui.spacing_mut().item_spacing.y = 10.0;
                        // Label at the left, the field at the right.
                        let row = |ui: &mut Ui, label: &str, add: &mut dyn FnMut(&mut Ui)| {
                            ui.horizontal(|ui| {
                                ui.spacing_mut().item_spacing.x = 0.0;
                                let (r, _) = ui.allocate_exact_size(egui::vec2(82.0, 30.0), egui::Sense::hover());
                                ui.painter().text(r.left_center(), egui::Align2::LEFT_CENTER, label, egui::FontId::proportional(12.5), p.text_muted);
                                add(ui);
                            });
                        };
                        row(ui, t("Folder"), &mut |ui| {
                            ui.spacing_mut().item_spacing.x = 8.0;
                            let choose_w = kit::button_width(ui, Some(icons::FOLDER_OPEN), t("Choose…"), None, false);
                            let w = (ui.available_width() - choose_w - 8.0).max(80.0);
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
                        // kind and branch, in a quiet band under the name.
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
                                ui.add_space(82.0);
                                let w = ui.available_width();
                                let (r, _) = ui.allocate_exact_size(egui::vec2(w, 34.0), egui::Sense::hover());
                                let dark = ui.visuals().dark_mode;
                                ui.painter().rect_filled(r, CornerRadius::same(7), p.hover);
                                let letters = ui
                                    .data(|d| d.get_temp::<(String, String)>(letters_id))
                                    .filter(|(k, _)| k == &typed)
                                    .map(|(_, l)| l)
                                    .unwrap_or_else(|| {
                                        let dir = crate::app::absolute(&typed);
                                        let folder = dir.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                                        crate::project::initial(&folder)
                                    });
                                let tile = egui::Rect::from_min_size(egui::pos2(r.left() + 9.0, r.center().y - 9.0), egui::vec2(18.0, 18.0));
                                kit::paint_project_tile(ui.painter(), tile, &letters, next_color, dark);
                                let mut job = egui::text::LayoutJob::default();
                                let parts: Vec<&str> = line.split(" · ").collect();
                                job.append(parts.first().copied().unwrap_or(""), 0.0, egui::TextFormat::simple(egui::FontId::proportional(12.0), p.text_muted));
                                if let Some(branch) = parts.get(1) {
                                    job.append("·", 6.0, egui::TextFormat::simple(egui::FontId::proportional(12.0), p.text_ghost));
                                    job.append(icons::GIT_BRANCH, 6.0, egui::TextFormat::simple(style::icon_font(11.0), p.text_faint));
                                    job.append(branch, 4.0, egui::TextFormat::simple(style::mono(11.0), p.text_faint));
                                }
                                job.wrap = egui::text::TextWrapping::truncate_at_width(w - 46.0);
                                let g = ui.painter().layout_job(job);
                                ui.painter().galley(egui::pos2(tile.right() + 9.0, r.center().y - g.size().y / 2.0), g, p.text_muted);
                            });
                        }
                        // An agent to start in it once added.
                        row(ui, t("Then run"), &mut |ui| {
                            let mut options = vec![kit::Seg::new(t("Nothing"), "")];
                            options.extend(presets.iter().take(3).map(|x| {
                                let s = kit::Seg::new(x.name.as_str(), x.command.as_str());
                                match x.agent.filter(|a| *a != tc_core::agent::AgentKind::Custom) {
                                    Some(a) => s.agent(a),
                                    None => s.icon(icons::TERMINAL_WINDOW),
                                }
                            }));
                            if let Some(i) = kit::segmented(ui, &options, *then) {
                                *then = i;
                            }
                        });
                    });
                    let ready = !path.trim().is_empty();
                    let (cancel, ok) = footer(ui, None, t("Add Project"), Confirm::Primary, ready);
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
                    ui.set_width(460.0);
                    if header(
                        ui,
                        icons::PLUGS_CONNECTED,
                        p.warn,
                        &tf("Uninstall hooks for {}?", &[agent.label()]),
                        Some(t("Its settings file goes back to how it was before Install; changes you made since are kept.")),
                    ) {
                        keep = false;
                        self.actions.push(Action::Integrations);
                    }
                    ui.add_space(12.0);
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
                Dialog::MergeWorktree(id) => {
                    keep = self.merge_worktree_dialog(ui, *id, enter);
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
                    ui.set_width(460.0);
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
                        let (r, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 36.0), egui::Sense::hover());
                        let dark = ui.visuals().dark_mode;
                        ui.painter().rect(r, CornerRadius::same(8), if dark { p.inset } else { Color32::WHITE }, Stroke::new(1.0, p.border), egui::StrokeKind::Inside);
                        ui.painter().text(egui::pos2(r.left() + 12.0 + 6.5, r.center().y), egui::Align2::CENTER_CENTER, icons::FILE_TEXT, style::icon_font(13.0), p.text_faint);
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
                            let g = ui.painter().layout_no_wrap(what.join(" · "), style::mono(10.5), p.text_faint);
                            right -= g.size().x;
                            ui.painter().galley(egui::pos2(right, r.center().y - g.size().y / 2.0), g, p.text_faint);
                            right -= 12.0;
                        }
                        let x = r.left() + 12.0 + 13.0 + 10.0;
                        let mut job = egui::text::LayoutJob::simple_singleline(rel, style::mono(12.0), p.text);
                        job.wrap = egui::text::TextWrapping::truncate_at_width((right - x).max(20.0));
                        let g = ui.painter().layout_job(job);
                        ui.painter().galley(egui::pos2(x, r.center().y - g.size().y / 2.0), g, p.text);
                    });
                    let mut save = enter;
                    let (mut discard, mut cancel) = (false, false);
                    footer_left(
                        ui,
                        |ui| {
                            discard = kit::button_ex(ui, kit::Kind::Ghost, None, t("Don’t Save"), false).clicked();
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
                    ui.set_width(460.0);
                    if header(
                        ui,
                        icons::SIGN_OUT,
                        p.err,
                        t("Quit TernaryCode?"),
                        Some(t("Programs still running are stopped, and changes not saved are lost:")),
                    ) {
                        keep = false;
                    }
                    body(ui, |ui| {
                        busy_list(ui, busy);
                        ui.add_space(4.0);
                        ui.label(
                            RichText::new(t("Closing the window instead keeps them running in the background."))
                                .size(style::SMALL)
                                .color(p.text_muted),
                        );
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
                    ui.set_width(460.0);
                    // Restarting in place, the programs keep running: only unsaved files are
                    // listed.
                    let in_place = crate::update::restarts_in_place();
                    let (lost, after) = if in_place {
                        ("Changes not saved are lost:", "TernaryCode restarts as the new version; programs and agents keep running.")
                    } else {
                        (
                            "Programs still running are stopped, and changes not saved are lost:",
                            "TernaryCode opens again with your projects and tabs; agents that support it can be resumed.",
                        )
                    };
                    if header(ui, icons::ARROW_CIRCLE_UP, p.accent, t("Restart to update?"), Some(t(lost))) {
                        keep = false;
                    }
                    body(ui, |ui| {
                        busy_list(ui, busy);
                        ui.add_space(4.0);
                        ui.label(RichText::new(t(after)).size(style::SMALL).color(p.text_muted));
                    });
                    let (cancel, ok) = footer(ui, None, t("Restart and Update"), Confirm::Danger, true);
                    if cancel {
                        keep = false;
                    }
                    if ok {
                        self.actions.push(Action::RestartToUpdateNow);
                        keep = false;
                    }
                }
                Dialog::ConfirmRemove(id, busy) => {
                    ui.set_width(460.0);
                    let name = self.project(*id).map(|x| x.name.clone()).unwrap_or_default();
                    if header(
                        ui,
                        icons::TRASH,
                        p.err,
                        &tf("Remove {} from the workspace?", &[&name]),
                        Some(t("The folder is not deleted. Programs still running are stopped, and changes not saved are lost:")),
                    ) {
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
                    ui.set_width(440.0);
                    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                    if header(
                        ui,
                        icons::TRASH,
                        p.err,
                        &tf("Move \"{}\" to the Trash?", &[&name]),
                        Some(t("You can put it back from the Trash.")),
                    ) {
                        keep = false;
                    }
                    ui.add_space(12.0);
                    let (cancel, ok) = footer(ui, None, t("Move to Trash"), Confirm::Danger, true);
                    if cancel {
                        keep = false;
                    }
                    if ok {
                        self.actions.push(Action::TrashPathNow(path.clone()));
                        keep = false;
                    }
                }
                Dialog::ConfirmClearBrowsing => {
                    ui.set_width(440.0);
                    if header(
                        ui,
                        icons::TRASH,
                        p.err,
                        t("Clear logins, cookies and history?"),
                        Some(t("Every browser pane is signed out of every site, in every project. This can't be undone.")),
                    ) {
                        keep = false;
                    }
                    ui.add_space(12.0);
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
                    ui.set_width(440.0);
                    if header(
                        ui,
                        icons::WARNING,
                        p.err,
                        t("Close this pane?"),
                        Some(&tf("{} is still running in it and will be stopped.", &[what])),
                    ) {
                        keep = false;
                    }
                    // What runs: its name, its process group, since when, the project.
                    let pane = self.panes.get(id);
                    let pgid = pane.and_then(|x| x.session.as_ref()).and_then(|s| s.pty.foreground_pgid());
                    let since = pane.map(|x| super::status::age(x.tracker.since));
                    let project = self.projects.iter().find(|x| x.panes().contains(id)).map(|x| x.name.clone());
                    body(ui, |ui| {
                        let (r, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 46.0), egui::Sense::hover());
                        let dark = ui.visuals().dark_mode;
                        ui.painter().rect(r, CornerRadius::same(8), if dark { p.inset } else { Color32::WHITE }, Stroke::new(1.0, p.border), egui::StrokeKind::Inside);
                        kit::paint_lattice(ui.painter(), egui::pos2(r.left() + 12.0 + 5.0, r.center().y), 10.0, p.info, true);
                        let x = r.left() + 12.0 + 10.0 + 12.0;
                        let max_w = (r.right() - 12.0 - x).max(20.0);
                        let mut job1 = egui::text::LayoutJob::simple_singleline(what.clone(), style::mono(12.0), p.text);
                        job1.wrap = egui::text::TextWrapping::truncate_at_width(max_w);
                        let g1 = ui.painter().layout_job(job1);
                        ui.painter().galley(egui::pos2(x, r.top() + 9.0), g1, p.text);
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
                        let mut job2 = egui::text::LayoutJob::simple_singleline(parts.join(" · "), style::mono(10.5), p.text_faint);
                        job2.wrap = egui::text::TextWrapping::truncate_at_width(max_w);
                        let g2 = ui.painter().layout_job(job2);
                        ui.painter().galley(egui::pos2(x, r.bottom() - 9.0 - g2.size().y), g2, p.text_faint);
                    });
                    let (cancel, ok) = footer(ui, None, t("Close Pane"), Confirm::Danger, true);
                    if cancel {
                        keep = false;
                    }
                    if ok {
                        self.actions.push(Action::ClosePaneId(*id));
                        keep = false;
                    }
                }
                Dialog::ConfirmCloseTab(panes, busy) => {
                    ui.set_width(460.0);
                    if header(
                        ui,
                        icons::X_CIRCLE,
                        p.warn,
                        t("Close this tab?"),
                        Some(t("These programs will be stopped:")),
                    ) {
                        keep = false;
                    }
                    body(ui, |ui| busy_list(ui, busy));
                    let (cancel, ok) = footer(ui, None, t("Close Tab"), Confirm::Danger, true);
                    if cancel {
                        keep = false;
                    }
                    if ok {
                        self.actions.push(Action::ClosePanesNow(panes.clone()));
                        keep = false;
                    }
                }
                Dialog::Rename(id, name) => {
                    ui.set_width(420.0);
                    if header(ui, icons::PENCIL_SIMPLE, p.accent, t("Rename Project"), None) {
                        keep = false;
                    }
                    body(ui, |ui| {
                        field_label(ui, t("Name"));
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
                    ui.set_width(420.0);
                    if header(
                        ui,
                        icons::PENCIL_SIMPLE,
                        p.accent,
                        t("Rename Pane"),
                        Some(t("Leave it empty to name it after what runs in it.")),
                    ) {
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
                    let max_w = (ui.ctx().content_rect().width() - 48.0).clamp(360.0, 760.0);
                    ui.set_width(max_w);
                    if header(
                        ui,
                        icons::PLUGS_CONNECTED,
                        p.accent,
                        &tf("Install hooks for {}?", &[plan.agent.label()]),
                        Some(&tf("TernaryCode adds its hooks to your {} settings, so agents you start by hand report Needs input, Done and Failed too.", &[plan.agent.label()])),
                    ) {
                        keep = false;
                        self.actions.push(Action::Integrations);
                    }
                    body(ui, |ui| install_preview(ui, &plan.path, plan.before.as_deref(), diff));
                    // Where the original is kept.
                    let backup = crate::paths::contract_tilde(&crate::paths::integrations_dir().join("backup").join(format!("{}-original", plan.agent.slug())));
                    let mut cancel = false;
                    let mut ok = false;
                    footer_left(
                        ui,
                        |ui| {
                            ui.spacing_mut().item_spacing.x = 6.0;
                            ui.label(RichText::new(icons::INFO).font(style::icon_font(12.0)).color(p.text_faint));
                            ui.label(RichText::new(t("Backup in")).size(style::LABEL).color(p.text_faint));
                            ui.add(egui::Label::new(RichText::new(backup).font(style::mono(10.5)).color(p.text_faint)).truncate());
                        },
                        |ui| {
                            let okr = kit::button_keys(ui, kit::Kind::Primary, None, t("Install"), Some("↵"), false);
                            #[cfg(debug_assertions)]
                            crate::devtest::record("dialog-ok", okr.rect);
                            ok = okr.clicked() || enter;
                            cancel = kit::button_ex(ui, kit::Kind::Ghost, None, t("Cancel"), false).clicked();
                        },
                    );
                    if cancel {
                        keep = false;
                        self.actions.push(Action::Integrations);
                    }
                    if ok {
                        self.actions.push(Action::ConfirmInstall);
                    }
                }
                Dialog::Message(head, text) => {
                    ui.set_width(440.0);
                    if header(ui, icons::INFO, p.accent, head, None) {
                        keep = false;
                    }
                    body(ui, |ui| {
                        ui.add(egui::Label::new(RichText::new(text.as_str()).color(p.text)).wrap());
                    });
                    let full = ui.max_rect();
                    ui.painter().hline(full.x_range(), ui.cursor().top(), Stroke::new(1.0, p.border));
                    egui::Frame::NONE
                        .inner_margin(Margin { left: PAD, right: PAD - 6, top: 12, bottom: 12 })
                        .show(ui, |ui| {
                            ui.set_width(ui.available_width());
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                if kit::primary(ui, t("OK")).clicked() || enter {
                                    keep = false;
                                }
                            });
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
                Dialog::MergeWorktree(id) => ctx.data_mut(|d| d.remove::<(usize, u32, u32)>(egui::Id::new(("merge-stat", *id)))),
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

/// What is at a folder typed into Add Project: its kind and its git branch ("Rust · git,
/// branch main"); nothing when it is not a folder.
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

/// The change Install makes: the file, how many lines it adds and removes, and the lines
/// themselves with their numbers (long runs of unchanged lines folded).
fn install_preview(ui: &mut Ui, path: &std::path::Path, before: Option<&str>, diff: &str) {
    let p = style::pal(ui);
    let dark = ui.visuals().dark_mode;
    let lines: Vec<&str> = diff.lines().collect();
    let added = lines.iter().filter(|l| l.starts_with("+ ")).count();
    let removed = lines.iter().filter(|l| l.starts_with("- ")).count();
    let w = ui.available_width();
    // The file: where, whether it is there, and how much changes.
    let (head, _) = ui.allocate_exact_size(egui::vec2(w, 38.0), egui::Sense::hover());
    let painter = ui.painter();
    painter.rect(head, CornerRadius { nw: 8, ne: 8, sw: 0, se: 0 }, p.surface, Stroke::new(1.0, p.border_strong), egui::StrokeKind::Inside);
    painter.text(egui::pos2(head.left() + 12.0 + 7.5, head.center().y), egui::Align2::CENTER_CENTER, icons::FILE_CODE, style::icon_font(15.0), p.text_faint);
    let mut right = head.right() - 12.0 - super::review::SHARE_W;
    super::review::paint_share(painter, egui::pos2(right, head.center().y), added as u32, removed as u32, &p);
    let mut cjob = egui::text::LayoutJob::default();
    cjob.append(&format!("+{added}"), 0.0, egui::TextFormat::simple(style::mono(11.5), p.ok));
    cjob.append(&format!("−{removed}"), 6.0, egui::TextFormat::simple(style::mono(11.5), p.err));
    let cg = painter.layout_job(cjob);
    right -= 10.0 + cg.size().x;
    painter.galley(egui::pos2(right, head.center().y - cg.size().y / 2.0), cg, p.text);
    let state = match before {
        Some(text) => tf("exists, {} lines", &[&text.lines().count().to_string()]),
        None => t("a new file").to_string(),
    };
    let mut job = egui::text::LayoutJob::default();
    job.append(&crate::paths::contract_tilde(path), 0.0, egui::TextFormat::simple(style::mono(11.5), p.text));
    job.append("·", 8.0, egui::TextFormat::simple(egui::FontId::proportional(12.0), p.text_ghost));
    job.append(&state, 8.0, egui::TextFormat::simple(egui::FontId::proportional(12.0), p.text_faint));
    let x = head.left() + 12.0 + 15.0 + 8.0;
    job.wrap = egui::text::TextWrapping::truncate_at_width((right - 12.0 - x).max(40.0));
    let g = painter.layout_job(job);
    painter.galley(egui::pos2(x, head.center().y - g.size().y / 2.0), g, p.text);
    // What it does, in three.
    let (sum, _) = ui.allocate_exact_size(egui::vec2(w, 52.0), egui::Sense::hover());
    let painter = ui.painter();
    painter.rect(sum, CornerRadius::ZERO, if dark { style::over(p.canvas, p.chrome, 0.3) } else { Color32::from_rgb(0xfa, 0xfa, 0xf9) }, Stroke::new(1.0, p.border_strong), egui::StrokeKind::Inside);
    let items = [
        (icons::PLUS_CIRCLE, p.ok, crate::i18n::count(added, t("Adds 1 line"), t("Adds {} lines")), t("Its hooks, after yours").to_string()),
        (icons::CHECK_CIRCLE, p.ok, t("Keeps what is there").to_string(), if removed == 0 { t("Nothing is removed").to_string() } else { crate::i18n::count(removed, t("1 line is rewritten"), t("{} lines are rewritten")) }),
        (icons::CLOCK_COUNTER_CLOCKWISE, p.text_muted, t("Keeps a backup").to_string(), t("Uninstall restores it byte for byte").to_string()),
    ];
    let cw = sum.width() / 3.0;
    for (i, (icon, c, title, sub)) in items.iter().enumerate() {
        let cell = egui::Rect::from_min_size(egui::pos2(sum.left() + i as f32 * cw, sum.top()), egui::vec2(cw, sum.height()));
        if i > 0 {
            painter.vline(cell.left(), (cell.top() + 10.0)..=(cell.bottom() - 10.0), Stroke::new(1.0, p.border));
        }
        painter.text(egui::pos2(cell.left() + 14.0 + 7.0, cell.center().y - 7.0), egui::Align2::CENTER_CENTER, *icon, style::icon_fill(14.0), *c);
        let tx = cell.left() + 14.0 + 14.0 + 9.0;
        let mut job = egui::text::LayoutJob::simple_singleline(title.clone(), style::medium(12.0), p.text);
        job.wrap = egui::text::TextWrapping::truncate_at_width((cell.right() - 10.0 - tx).max(20.0));
        let g = painter.layout_job(job);
        painter.galley(egui::pos2(tx, cell.center().y - 15.0), g, p.text);
        let mut job = egui::text::LayoutJob::simple_singleline(sub.clone(), egui::FontId::proportional(11.0), p.text_faint);
        job.wrap = egui::text::TextWrapping::truncate_at_width((cell.right() - 10.0 - tx).max(20.0));
        let g = painter.layout_job(job);
        painter.galley(egui::pos2(tx, cell.center().y + 2.0), g, p.text_faint);
    }
    // The change, unchanged lines more than three away from it folded.
    let syntax = crate::editor::syntax_for(path);
    let near = |i: usize| lines[i.saturating_sub(3)..(i + 4).min(lines.len())].iter().any(|l| l.starts_with("+ ") || l.starts_with("- "));
    egui::Frame::NONE
        .fill(p.canvas)
        .stroke(Stroke::new(1.0, p.border_strong))
        .corner_radius(CornerRadius { nw: 0, ne: 0, sw: 8, se: 8 })
        .inner_margin(Margin { left: 0, right: 0, top: 4, bottom: 4 })
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            egui::ScrollArea::vertical().max_height(320.0).show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 0.0;
                let font = style::mono(11.0);
                let mut number = 0;
                let mut folded = 0;
                let fold_row = |ui: &mut Ui, n: usize| {
                    let (r, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 26.0), egui::Sense::hover());
                    ui.painter().rect_filled(r, CornerRadius::ZERO, style::over(p.canvas, p.accent, 0.06));
                    ui.painter().text(egui::pos2(r.left() + 44.0 + 9.0, r.center().y), egui::Align2::CENTER_CENTER, icons::CARET_UP_DOWN, style::icon_font(12.0), p.text_faint);
                    ui.painter().text(egui::pos2(r.left() + 62.0, r.center().y), egui::Align2::LEFT_CENTER, tf("{} unchanged lines", &[&n.to_string()]), style::mono(10.5), p.text_faint);
                };
                for (i, line) in lines.iter().enumerate() {
                    let (sign, text) = line.split_at(line.len().min(2));
                    let add = sign == "+ ";
                    let del = sign == "- ";
                    if !del {
                        number += 1;
                    }
                    if !add && !del && !near(i) {
                        folded += 1;
                        continue;
                    }
                    if folded > 0 {
                        fold_row(ui, folded);
                        folded = 0;
                    }
                    let (r, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 18.0), egui::Sense::hover());
                    let (bg, sc, nc) = if add {
                        (style::soft(p.ok, if dark { 0.10 } else { 0.08 }), p.ok, style::soft(p.ok, 0.6))
                    } else if del {
                        (style::soft(p.err, if dark { 0.10 } else { 0.07 }), p.err, style::soft(p.err, 0.6))
                    } else {
                        (Color32::TRANSPARENT, p.text_ghost, p.text_ghost)
                    };
                    ui.painter().rect_filled(r, CornerRadius::ZERO, bg);
                    if !del {
                        ui.painter().text(egui::pos2(r.left() + 36.0, r.center().y), egui::Align2::RIGHT_CENTER, number.to_string(), style::mono(10.5), nc);
                    }
                    ui.painter().text(egui::pos2(r.left() + 44.0 + 9.0, r.center().y), egui::Align2::CENTER_CENTER, if add { "+" } else if del { "−" } else { "" }, font.clone(), sc);
                    let mut job = egui::text::LayoutJob::default();
                    for (c, piece) in crate::editor::line_colors(text, &syntax, dark, if add || del { p.text } else { p.text_muted }) {
                        job.append(&piece, 0.0, egui::TextFormat::simple(font.clone(), c));
                    }
                    job.wrap = egui::text::TextWrapping::truncate_at_width((r.width() - 70.0).max(40.0));
                    let g = ui.painter().layout_job(job);
                    ui.painter().galley(egui::pos2(r.left() + 62.0, r.center().y - g.size().y / 2.0), g, p.text);
                }
                if folded > 0 {
                    fold_row(ui, folded);
                }
            });
        });
}

impl App {
    /// The Run Agent dialog: which agent (with how it tells TernaryCode that it needs you),
    /// whether to continue its last session, and where it runs. Returns whether it stays open.
    #[allow(clippy::too_many_arguments)]
    fn run_agent_dialog(&mut self, ui: &mut Ui, selected: &mut usize, session: &mut usize, place: &mut usize, wt: &mut crate::worktree::Form, enter: bool) -> bool {
        use crate::actions::Launch;
        use tc_core::agent::AgentKind;
        let p = style::pal(ui);
        let presets = self.presets();
        let here = self.idle_command_pane().is_some();
        let project = self.active_project().map(|x| (x.name.clone(), crate::paths::contract_tilde(&x.path), x.path.clone(), x.letters(), x.color32()));
        let dlg_w = RUN_AGENT_W.min((ui.ctx().content_rect().width() - 32.0).max(520.0));
        let list_w = RUN_AGENT_LIST_W.min((dlg_w * 0.34).round());
        ui.set_width(dlg_w);
        ui.set_max_width(dlg_w);
        let mut keep = true;
        if run_agent_header(ui, project.as_ref().map(|x| (x.0.as_str(), x.1.as_str(), x.3.as_str(), x.4))) {
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
            && (!wt.typed || wt.name.is_empty())
        {
            let stem = chosen.as_ref().map(|x| x.agent.map_or(x.name.clone(), |a| a.slug().to_string())).unwrap_or_default();
            wt.name = self.suggest_worktree_name(r.0, &stem);
            wt.typed = false;
        }
        let base = wt.base.clone().or_else(|| facts.as_ref().and_then(|f| f.current.clone())).unwrap_or_default();
        let problem = root.as_ref().and_then(|r| self.worktree_name_problem(r.0, &wt.name));
        let wt_ready = facts.is_some() && problem.is_none() && !base.is_empty();
        let mut edit_setup = false;

        let mut run = enter;
        // Two columns under a line: the agents at the left, what to run at the right.
        let top = ui.cursor().top();
        ui.painter().hline(ui.max_rect().x_range(), top, Stroke::new(1.0, p.border));
        let left_bg = ui.painter().add(egui::Shape::Noop);
        let left_line = ui.painter().add(egui::Shape::Noop);
        let columns = ui.horizontal_top(|ui| {
            ui.spacing_mut().item_spacing = egui::vec2(0.0, 0.0);
            // ----- the agents ---------------------------------------------------------------
            egui::Frame::NONE.inner_margin(Margin::same(10)).show(ui, |ui| {
                ui.set_width(list_w - 20.0);
                egui::ScrollArea::vertical().id_salt("run-agent-presets").max_height(380.0).auto_shrink([false, true]).show(ui, |ui| {
                    ui.set_width(list_w - 20.0);
                    ui.spacing_mut().item_spacing.y = 2.0;
                    for (i, preset) in presets.iter().enumerate() {
                        let (rect, resp) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 50.0), egui::Sense::click());
                        #[cfg(debug_assertions)]
                        crate::devtest::record(format!("agent:{}", preset.id), rect);
                        let on = i == *selected;
                        let painter = ui.painter();
                        if on {
                            painter.rect(rect, CornerRadius::same(9), p.surface, Stroke::new(1.0, p.border_strong), egui::StrokeKind::Inside);
                            kit::paint_highlight(painter, rect, 9, &p);
                        } else if resp.hovered() {
                            painter.rect_filled(rect, CornerRadius::same(9), p.hover);
                        }
                        let cy = rect.center().y;
                        let tile = egui::Rect::from_center_size(egui::pos2(rect.left() + 8.0 + 16.0, cy), egui::vec2(32.0, 32.0));
                        painter.rect(tile, CornerRadius::same(9), p.surface, Stroke::new(1.0, p.border_strong), egui::StrokeKind::Inside);
                        let logo = egui::Rect::from_center_size(tile.center(), egui::vec2(18.0, 18.0));
                        if !preset.agent.is_some_and(|a| super::agent_logo::paint(painter, logo, a, p.text, false)) {
                            painter.text(tile.center(), egui::Align2::CENTER_CENTER, initials(&preset.name), style::mono_semibold(11.0), p.text_muted);
                        }
                        let x = tile.right() + 11.0;
                        let key = (i + 1).to_string();
                        let kw = if i < 9 { kit::kbd_width(painter, &key) } else { 0.0 };
                        let mut job = egui::text::LayoutJob::simple_singleline(preset.name.clone(), style::medium(13.0), p.text);
                        job.wrap = egui::text::TextWrapping::truncate_at_width((rect.right() - 10.0 - kw - 8.0 - x).max(20.0));
                        let g = painter.layout_job(job);
                        painter.galley(egui::pos2(x, cy - 8.0 - g.size().y / 2.0), g, p.text);
                        let mut job = egui::text::LayoutJob::simple_singleline(preset.command.clone(), style::mono(10.5), p.text_faint);
                        job.wrap = egui::text::TextWrapping::truncate_at_width((rect.right() - 10.0 - kw - 8.0 - x).max(20.0));
                        let g = painter.layout_job(job);
                        painter.galley(egui::pos2(x, cy + 9.0 - g.size().y / 2.0), g, p.text_faint);
                        if i < 9 {
                            kit::paint_kbd_text(painter, egui::pos2(rect.right() - 10.0 - kw, cy), &key, &p);
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
                    let (rect, resp) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 38.0), egui::Sense::click());
                    if resp.hovered() {
                        ui.painter().rect_filled(rect, CornerRadius::same(9), p.hover);
                    }
                    let c = if resp.hovered() { p.text_muted } else { p.text_faint };
                    ui.painter().text(egui::pos2(rect.left() + 14.0 + 7.0, rect.center().y), egui::Align2::CENTER_CENTER, icons::PLUS, style::icon_font(13.0), c);
                    ui.painter().text(egui::pos2(rect.left() + 14.0 + 14.0 + 9.0, rect.center().y), egui::Align2::LEFT_CENTER, t("Add preset…"), egui::FontId::proportional(12.5), c);
                    if resp.clicked() {
                        self.actions.push(Action::Integrations);
                        keep = false;
                    }
                });
            });
            // ----- what to run ----------------------------------------------------------------
            egui::Frame::NONE.inner_margin(Margin { left: 20, right: 20, top: 8, bottom: 10 }).show(ui, |ui| ui.vertical(|ui| {
                let w = dlg_w - list_w - 40.0;
                ui.set_width(w);
                ui.spacing_mut().item_spacing.y = 0.0;
                let mut first_row = true;
                let mut option_row = |ui: &mut Ui, label: &str, top_aligned: bool, add: &mut dyn FnMut(&mut Ui)| {
                    if !first_row {
                        let y = ui.cursor().top();
                        ui.painter().hline(ui.max_rect().x_range(), y, Stroke::new(1.0, p.border));
                    }
                    first_row = false;
                    egui::Frame::NONE.inner_margin(Margin::symmetric(0, 8)).show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        let row = |ui: &mut Ui| {
                            ui.spacing_mut().item_spacing.x = 14.0;
                            let (r, _) = ui.allocate_exact_size(egui::vec2(92.0, if top_aligned { 32.0 } else { 28.0 }), egui::Sense::hover());
                            ui.painter().text(egui::pos2(r.left(), r.center().y), egui::Align2::LEFT_CENTER, label, egui::FontId::proportional(12.5), p.text_muted);
                            ui.vertical(|ui| add(ui));
                        };
                        if top_aligned {
                            ui.horizontal_top(row);
                        } else {
                            ui.horizontal(row);
                        }
                    });
                };
                // Where it works: the project's folder, or a new worktree (git projects).
                if let Some(r) = &root {
                    option_row(ui, t("Work in"), false, &mut |ui: &mut Ui| {
                        let options = [
                            kit::Seg::new(t("This folder"), "").icon(icons::FOLDER_SIMPLE),
                            kit::Seg::new(t("New worktree"), t("A folder and a branch of its own, so agents in this repository don’t overwrite each other")).icon(icons::GIT_FORK),
                        ];
                        if let Some(i) = kit::segmented(ui, &options, wt.on as usize) {
                            wt.on = i == 1;
                            if let Some(project) = self.project_mut(r.0) {
                                project.new_worktree = wt.on;
                            }
                            self.mark_dirty();
                        }
                    });
                }
                if wt.on
                    && let Some(r) = &root
                {
                    ui.add_space(4.0);
                    edit_setup = self.worktree_box(ui, r.0, &r.1, r.3, wt, &base, facts.as_ref(), problem.as_deref(), name_id);
                    ui.add_space(10.0);
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(icons::INFO).font(style::icon_font(12.0)).color(p.text_faint));
                        ui.add(
                            egui::Label::new(
                                RichText::new(tf("A new worktree starts a new session. It opens under {} in the sidebar, in its own tab.", &[&r.1]))
                                    .size(style::SMALL)
                                    .color(p.text_faint),
                            )
                            .wrap(),
                        );
                    });
                    ui.add_space(8.0);
                } else {
                    // The session: a new one, or one to continue.
                    let command = chosen.as_ref().map(|x| x.command.clone()).unwrap_or_default();
                    option_row(ui, t("Session"), true, &mut |ui: &mut Ui| {
                        egui::ScrollArea::vertical().id_salt("run-agent-sessions").max_height(212.0).auto_shrink([false, true]).show(ui, |ui| {
                            ui.spacing_mut().item_spacing.y = 1.0;
                            // One row: a radio, two lines, and a pill at the right.
                            let mut row = |ui: &mut Ui, index: usize, title: &str, sub: &str, mono: bool, pill: Option<String>| {
                                let h = if sub.is_empty() { 32.0 } else { 44.0 };
                                let (rect, resp) = ui.allocate_exact_size(egui::vec2(ui.available_width(), h), egui::Sense::click());
                                #[cfg(debug_assertions)]
                                crate::devtest::record(format!("session:{index}"), rect);
                                let on = index == *session;
                                if on && moved {
                                    ui.scroll_to_rect(rect, None);
                                }
                                let painter = ui.painter_at(rect);
                                if on {
                                    painter.rect(rect, CornerRadius::same(8), p.accent_tint, Stroke::new(1.0, style::soft(p.accent, 0.3)), egui::StrokeKind::Inside);
                                } else if resp.hovered() {
                                    painter.rect_filled(rect, CornerRadius::same(8), p.hover);
                                }
                                let cy = rect.center().y;
                                // The radio.
                                let rc = egui::pos2(rect.left() + 8.0 + 8.0, cy);
                                if on {
                                    painter.circle_filled(rc, 8.0, p.accent);
                                    painter.circle_filled(rc, 3.0, Color32::WHITE);
                                } else {
                                    painter.circle_stroke(rc, 7.5, Stroke::new(1.0, p.border_heavy));
                                }
                                let x = rect.left() + 8.0 + 16.0 + 10.0;
                                let mut right = rect.right() - 8.0;
                                if let Some(text) = &pill {
                                    let w = kit::pill_width(&painter, Some(icons::TERMINAL_WINDOW), text);
                                    kit::paint_pill(&painter, egui::pos2(right - w, cy), Some(icons::TERMINAL_WINDOW), text, p.accent_text, p.accent_tint);
                                    right -= w + 8.0;
                                }
                                let line1 = if sub.is_empty() { cy } else { cy - 8.0 };
                                let mut job = egui::text::LayoutJob::simple_singleline(title.to_string(), if on { style::medium(12.5) } else { egui::FontId::proportional(12.5) }, p.text);
                                job.wrap = egui::text::TextWrapping::truncate_at_width((right - x).max(20.0));
                                let g = painter.layout_job(job);
                                painter.galley(egui::pos2(x, line1 - g.size().y / 2.0), g, p.text);
                                if !sub.is_empty() {
                                    let font = if mono { style::mono(10.5) } else { egui::FontId::proportional(11.5) };
                                    let mut job = egui::text::LayoutJob::simple_singleline(sub.to_string(), font, p.text_faint);
                                    job.wrap = egui::text::TextWrapping::truncate_at_width((right - x).max(20.0));
                                    let g = painter.layout_job(job);
                                    painter.galley(egui::pos2(x, cy + 9.0 - g.size().y / 2.0), g, p.text_faint);
                                }
                                if resp.clicked() {
                                    *session = index;
                                }
                                if resp.double_clicked() {
                                    run = true;
                                }
                            };
                            row(ui, 0, t("New session"), &command, true, None);
                            let note = |ui: &mut Ui, text: &str| {
                                let (r, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 28.0), egui::Sense::hover());
                                ui.painter().text(egui::pos2(r.left() + 8.0 + 16.0 + 10.0, r.center().y), egui::Align2::LEFT_CENTER, text, egui::FontId::proportional(12.0), p.text_faint);
                            };
                            match &rows {
                                Rows::Listing => note(ui, t("Looking for sessions…")),
                                Rows::Sessions(v) if v.is_empty() => note(ui, t("No earlier sessions in this project.")),
                                Rows::Sessions(v) => {
                                    for (k, s) in v.iter().enumerate() {
                                        let mut sub = vec![tf("{} ago", &[&ago(s.updated)])];
                                        if let Some(b) = &s.branch {
                                            sub.push(b.clone());
                                        }
                                        sub.push(s.id.chars().take(8).collect());
                                        // Already open in a pane: choosing it goes there.
                                        let open = self.panes.values().find(|x| x.agent_session.as_deref() == Some(s.id.as_str())).map(|x| tf("Open in {}", &[&x.label()]));
                                        row(ui, k + 1, &s.title, &sub.join(" · "), false, open);
                                    }
                                }
                                Rows::Last(cmd) => row(ui, 1, t("Last session"), cmd, true, None),
                                Rows::Nothing => note(ui, t("This agent has no command that resumes a session.")),
                            }
                        });
                    });
                    option_row(ui, t("Open in"), false, &mut |ui: &mut Ui| {
                        // A shell at its prompt with nothing typed takes the agent.
                        let first = if here { t("This Tab") } else { t("New Tab") };
                        let options = [kit::Seg::new(first, ""), kit::Seg::new(t("Split Right"), ""), kit::Seg::new(t("Split Down"), "")];
                        if let Some(i) = kit::segmented(ui, &options, *place) {
                            *place = i;
                        }
                    });
                }
                option_row(ui, t("Permissions"), false, &mut |ui: &mut Ui| {
                    self.skip_prompts_row(ui, agent);
                });
                // How its status reaches TernaryCode.
                let kind = agent.unwrap_or(AgentKind::Custom);
                let note = self.hooks_note(kind);
                let hooked = self.installed(kind).is_some()
                    || (matches!(kind, AgentKind::Claude | AgentKind::Codex)
                        && self.settings.integrations.get(kind.slug()).copied().unwrap_or_default() == tc_core::model::Integration::PerLaunch);
                option_row(ui, t("Status"), false, &mut |ui: &mut Ui| {
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 8.0;
                        let name = chosen.as_ref().map(|x| x.name.clone()).unwrap_or_default();
                        if hooked {
                            kit::pill(ui, Some(icons::CHECK_CIRCLE), &tf("Hooks · {}", &[note.trim_start_matches("hooks · ")]), p.ok, style::tint(p.ok, ui.visuals().dark_mode));
                            ui.add(egui::Label::new(RichText::new(tf("Needs input, Done and Failed come from {} itself.", &[&name])).size(11.5).color(p.text_faint)).wrap());
                        } else if kind == AgentKind::Custom {
                            kit::pill(ui, None, t("Custom"), p.text_muted, p.hover);
                            ui.add(egui::Label::new(RichText::new(t("The status comes from the terminal.")).size(11.5).color(p.text_faint)).wrap());
                        } else {
                            kit::pill(ui, Some(icons::CIRCLE_HALF), t("No hooks · shows “Active”"), p.text_muted, p.hover);
                            ui.add(egui::Label::new(RichText::new(t("Settings → Agents connects it.")).size(11.5).color(p.text_faint)).wrap());
                        }
                    });
                });
            }));
        });
        // The agents' column: a shade of the base behind it, a line at its right.
        let cols = columns.response.rect;
        let left = egui::Rect::from_min_max(cols.min, egui::pos2(cols.left() + list_w, cols.bottom()));
        ui.painter().set(left_bg, egui::Shape::rect_filled(left, CornerRadius::ZERO, style::over(p.overlay, p.chrome, 0.35)));
        ui.painter().set(left_line, egui::Shape::vline(left.right(), left.y_range(), Stroke::new(1.0, p.border)));
        // What will be typed, exactly, and where.
        if let Some(preset) = &chosen {
            let base_cmd = match (&rows, *session, wt.on) {
                (Rows::Sessions(v), k, false) if k > 0 => v
                    .get(k - 1)
                    .and_then(|s| agent.and_then(|a| tc_agent::sessions::resume_command(a, &preset.command, &s.id)))
                    .unwrap_or_else(|| preset.command.clone()),
                (Rows::Last(c), 1, false) => c.clone(),
                _ => preset.command.clone(),
            };
            let mut line = self.launch_command(preset, &base_cmd);
            if wt.on && let Some(r) = &root {
                let setup_run = self.worktree_setup_of(r.0).run;
                let setup_run = setup_run.trim();
                if !setup_run.is_empty() {
                    line = format!("{setup_run} && {line}");
                }
            }
            let whereto = if wt.on {
                t("a new tab of the new worktree").to_string()
            } else {
                match (*place, here) {
                    (0, true) => t("this tab, its shell is idle").to_string(),
                    (0, false) => t("a new tab").to_string(),
                    (1, _) => t("a split at the right").to_string(),
                    _ => t("a split below").to_string(),
                }
            };
            run_command_bar(ui, &line, &whereto);
        }
        if edit_setup && let Some(r) = &root {
            self.pending_setup = Some(r.0);
        }
        let name = chosen.as_ref().map(|x| x.name.clone()).unwrap_or_default();
        let (hint, label) = if wt.on {
            (t("↵ create and run · Esc cancel").to_string(), t("Create Worktree and Run").to_string())
        } else {
            let hint = tf("1–{} agent · ↑↓ session · ↵ run · Esc cancel", &[&n.min(9).to_string()]);
            (hint, if *session > 0 { tf("Resume {}", &[&name]) } else { tf("Run {}", &[&name]) })
        };
        let enabled = !wt.on || wt_ready;
        footer_with(ui, Some(&hint), |ui| {
            let r = ui.add_enabled_ui(enabled, |ui| kit::button_keys(ui, kit::Kind::Primary, None, &label, Some("↵"), false)).inner;
            #[cfg(debug_assertions)]
            crate::devtest::record("dialog-ok", r.rect);
            run |= r.clicked();
            if kit::button_ex(ui, kit::Kind::Ghost, None, t("Cancel"), false).clicked() {
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
            self.actions.push(Action::RunAgent(preset.id.clone(), launch, *place));
            keep = false;
        }
        keep
    }
}

/// Run Agent's width, and its agents' column.
const RUN_AGENT_W: f32 = 780.0;
const RUN_AGENT_LIST_W: f32 = 250.0;

/// Run Agent's header: the play icon, "Run an agent", and "in ‹tile› project ~/path".
fn run_agent_header(ui: &mut Ui, project: Option<(&str, &str, &str, Color32)>) -> bool {
    let p = style::pal(ui);
    let top = ui.max_rect();
    kit::paint_highlight(ui.painter(), egui::Rect::from_min_size(top.min, egui::vec2(top.width(), 1.0)), style::RADIUS_DIALOG, &p);
    let mut close = false;
    egui::Frame::NONE.inner_margin(Margin { left: PAD, right: 14, top: 18, bottom: 14 }).show(ui, |ui| {
        ui.horizontal_top(|ui| {
            paint_dialog_icon(ui, icons::PLAY, p.accent);
            ui.add_space(14.0);
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 4.0;
                let mut job = egui::text::LayoutJob::default();
                let mut f = egui::TextFormat::simple(style::semibold(15.0), p.text);
                f.extra_letter_spacing = -0.15;
                job.append(t("Run an agent"), 0.0, f);
                ui.label(job);
                if let Some((name, path, letters, color)) = project {
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 6.0;
                        ui.label(RichText::new(t("in")).size(12.5).color(p.text_muted));
                        let (r, _) = ui.allocate_exact_size(egui::vec2(16.0, 16.0), egui::Sense::hover());
                        kit::paint_project_tile(ui.painter(), r, letters, color, ui.visuals().dark_mode);
                        ui.label(RichText::new(name).font(style::medium(12.5)).color(p.text));
                        let mut job = egui::text::LayoutJob::simple_singleline(path.to_string(), style::mono(11.0), p.text_faint);
                        job.wrap = egui::text::TextWrapping::truncate_at_width((ui.available_width() - 40.0).max(40.0));
                        ui.label(job);
                    });
                }
            });
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Min), |ui| {
                close = kit::icon_button_ex(ui, icons::X, t("Close"), false, 22.0).clicked();
            });
        });
    });
    close
}

/// The line under Run Agent's columns: what will be typed (`$ claude --mcp-config …`) and where.
fn run_command_bar(ui: &mut Ui, line: &str, whereto: &str) {
    let p = style::pal(ui);
    let full = ui.max_rect();
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(full.width(), 34.0 + 14.0), egui::Sense::hover());
    let bar = egui::Rect::from_min_max(egui::pos2(rect.left() + 14.0, rect.top() + 2.0), egui::pos2(rect.right() - 14.0, rect.bottom() - 12.0));
    let painter = ui.painter_at(bar);
    painter.rect(bar, CornerRadius::same(8), p.inset, Stroke::new(1.0, p.border), egui::StrokeKind::Inside);
    let cy = bar.center().y;
    painter.text(egui::pos2(bar.left() + 12.0 + 6.5, cy), egui::Align2::CENTER_CENTER, icons::TERMINAL_WINDOW, style::icon_font(13.0), p.text_faint);
    let where_g = painter.layout_no_wrap(whereto.to_string(), style::mono(10.5), p.text_ghost);
    let ww = where_g.size().x;
    painter.galley(egui::pos2(bar.right() - 12.0 - ww, cy - where_g.size().y / 2.0), where_g, p.text_ghost);
    // The program in the text colour, its flags quieter.
    let mut job = egui::text::LayoutJob::default();
    job.append("$ ", 0.0, egui::TextFormat::simple(style::mono(11.5), p.text_faint));
    let (program, rest) = line.split_once(' ').unwrap_or((line, ""));
    job.append(program, 0.0, egui::TextFormat::simple(style::mono(11.5), p.text));
    if !rest.is_empty() {
        job.append(rest, 7.0, egui::TextFormat::simple(style::mono(11.5), p.text_faint));
    }
    job.wrap = egui::text::TextWrapping::truncate_at_width((bar.width() - 12.0 - 13.0 - 9.0 - ww - 24.0).max(40.0));
    let g = painter.layout_job(job);
    painter.galley(egui::pos2(bar.left() + 12.0 + 13.0 + 9.0, cy - g.size().y / 2.0), g, p.text);
    resp.on_hover_text(line);
}

/// Two letters for the tile of an agent with no logo ("My Agent" → "MA", "Aider" → "Ad").
fn initials(name: &str) -> String {
    let words: Vec<&str> = name.split_whitespace().collect();
    match words.as_slice() {
        [a, b, ..] => format!("{}{}", a.chars().next().unwrap_or(' '), b.chars().next().unwrap_or(' ')).to_uppercase(),
        [a] => {
            let mut c = a.chars();
            let first = c.next().map(|x| x.to_uppercase().to_string()).unwrap_or_default();
            let second = c.find(|x| !"aeiou".contains(*x)).map(|x| x.to_string()).unwrap_or_default();
            format!("{first}{second}")
        }
        [] => String::new(),
    }
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
    /// Run Agent found no program for a preset: how to install it (Install in Terminal types the
    /// command), or Run Anyway (an alias or a function the check can't see).
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
        ui.set_width(480.0);
        let mut keep = true;
        let found = self.presets().into_iter().find(|x| x.id == preset);
        let name = found.as_ref().map_or_else(|| program.to_string(), |x| x.name.clone());
        let install = found.as_ref().and_then(|x| x.agent).and_then(tc_core::agent::install_command);
        let brew = cfg!(target_os = "macos") && ["/opt/homebrew/bin/brew", "/usr/local/bin/brew"].iter().any(|b| std::path::Path::new(b).exists());
        let node = if npm { tc_core::agent::node_install_command(brew) } else { None };
        // What Install in Terminal types: Node.js first when npm is missing, else the agent.
        let now = if npm { node } else { install };
        let sub = tf("TernaryCode starts “{}” from your PATH, and it isn’t there.", &[program]);
        if header(ui, icons::DOWNLOAD_SIMPLE, p.warn, &tf("{} isn’t installed", &[&name]), Some(&sub)) {
            keep = false;
        }
        let mut copied = None;
        body(ui, |ui| {
            if npm {
                let text = if node.is_some() {
                    t("It is installed with npm, which comes with Node.js: install Node.js first.")
                } else {
                    t("It is installed with npm, which comes with Node.js: install Node.js first, from nodejs.org.")
                };
                ui.horizontal_top(|ui| {
                    ui.spacing_mut().item_spacing.x = 8.0;
                    ui.label(RichText::new(icons::WARNING).font(style::icon_font(14.0)).color(p.warn));
                    ui.add(egui::Label::new(RichText::new(text).size(12.5).color(p.text).line_height(Some(18.0))).wrap());
                });
                ui.add_space(6.0);
            }
            for command in [node, install].into_iter().flatten() {
                if command_box(ui, command) {
                    copied = Some(command);
                }
                ui.add_space(6.0);
            }
            ui.add(
                egui::Label::new(
                    RichText::new(t("Once it is installed, start the agent again: a new tab finds it without restarting TernaryCode."))
                        .size(style::SMALL)
                        .color(p.text_muted),
                )
                .wrap(),
            );
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
                let r = kit::button_ex(ui, kit::Kind::Ghost, None, t("Run Anyway"), false);
                #[cfg(debug_assertions)]
                crate::devtest::record("dialog-anyway", r.rect);
                anyway = r.clicked();
            },
            |ui| {
                if now.is_some() {
                    let r = kit::button_keys(ui, kit::Kind::Primary, Some(icons::TERMINAL_WINDOW), t("Install in Terminal"), Some("↵"), false);
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

/// A command in a quiet monospace box; true when it is clicked (to copy it).
fn command_box(ui: &mut Ui, command: &str) -> bool {
    let p = style::pal(ui);
    let frame = egui::Frame::NONE
        .fill(p.canvas)
        .stroke(Stroke::new(1.0, p.border))
        .corner_radius(CornerRadius::same(style::RADIUS + 1))
        .inner_margin(Margin::symmetric(12, 9))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.add(egui::Label::new(RichText::new(command).font(egui::FontId::monospace(12.5)).color(p.text)).selectable(false));
        });
    let r = ui.interact(frame.response.rect, ui.id().with(("command-box", command)), egui::Sense::click()).on_hover_text(t("Copy"));
    if r.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    r.clicked()
}
