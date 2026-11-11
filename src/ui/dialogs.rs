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
const PAD: i8 = 22;

/// The card every dialog sits on (its parts add their own margins).
pub fn frame(ctx: &egui::Context) -> egui::Frame {
    let p = style::palette(ctx.global_style().visuals.dark_mode);
    egui::Frame::NONE
        .fill(p.surface)
        .stroke(Stroke::new(1.0, p.border_strong))
        .corner_radius(CornerRadius::same(style::RADIUS_LG + 2))
        .shadow(egui::epaint::Shadow {
            offset: [0, 16],
            blur: 48,
            spread: 0,
            color: Color32::from_black_alpha(if ctx.global_style().visuals.dark_mode { 140 } else { 50 }),
        })
}

/// A dialog's header. Returns whether its close button was clicked.
pub fn header(ui: &mut Ui, icon: &str, color: Color32, title: &str, sub: Option<&str>) -> bool {
    let p = style::pal(ui);
    let mut close = false;
    egui::Frame::NONE
        .inner_margin(Margin { left: PAD, right: 14, top: 18, bottom: 6 })
        .show(ui, |ui| {
            ui.horizontal_top(|ui| {
                kit::icon_badge(ui, icon, color, 36.0);
                ui.add_space(12.0);
                let text_w = (ui.available_width() - 40.0).max(120.0);
                ui.vertical(|ui| {
                    ui.set_width(text_w);
                    ui.add_space(if sub.is_some() { 0.0 } else { 8.0 });
                    ui.add(egui::Label::new(RichText::new(title).font(style::semibold(15.5)).color(p.text)).wrap());
                    if let Some(sub) = sub {
                        ui.add_space(3.0);
                        ui.spacing_mut().item_spacing.y = 2.0;
                        ui.add(
                            egui::Label::new(RichText::new(sub).size(13.0).color(p.text_muted).line_height(Some(18.5))).wrap(),
                        );
                    }
                });
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Min), |ui| {
                    close = kit::icon_button_ex(ui, icons::X, t("Close"), false, 26.0).clicked();
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
            // Fields sit a shade below the card, so they read as places to type.
            let p = style::pal(ui);
            ui.visuals_mut().text_edit_bg_color = Some(p.canvas);
            ui.visuals_mut().extreme_bg_color = p.canvas;
            add(ui)
        })
        .inner
}

/// A label over a field that takes the whole width.
fn field_label(ui: &mut Ui, text: &str) {
    let p = style::pal(ui);
    ui.label(RichText::new(text).font(style::medium(12.0)).color(p.text_muted));
}

/// A single-line text field the width of the dialog (minus `leave` for what follows it).
fn text_field(ui: &mut Ui, text: &mut String, hint: &str, leave: f32) -> egui::Response {
    let id = ui.next_auto_id();
    let w = (ui.available_width() - leave).max(80.0);
    kit::field(ui, id, text, None, hint, w, false)
}

/// The same with a folder icon.
fn folder_field(ui: &mut Ui, text: &mut String, hint: &str, leave: f32) -> egui::Response {
    let id = ui.next_auto_id();
    let w = (ui.available_width() - leave).max(80.0);
    kit::field(ui, id, text, Some(icons::FOLDER), hint, w, false)
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Confirm {
    Primary,
    Danger,
}

/// The footer: a line, `hint` at the left, Cancel and the main button at the right. Returns
/// (cancel, ok).
fn footer(ui: &mut Ui, hint: Option<&str>, ok: &str, kind: Confirm, enabled: bool) -> (bool, bool) {
    let mut out = (false, false);
    footer_with(ui, hint, |ui| {
        let okr = ui
            .add_enabled_ui(enabled, |ui| {
                let k = match kind {
                    Confirm::Primary => kit::Kind::Primary,
                    Confirm::Danger => kit::Kind::DangerFilled,
                };
                kit::button_ex(ui, k, None, ok, false)
            })
            .inner;
        #[cfg(debug_assertions)]
        crate::devtest::record("dialog-ok", okr.rect);
        out.1 = okr.clicked();
        out.0 = kit::secondary(ui, None, t("Cancel")).clicked();
    });
    out
}

/// A footer with its own buttons: a line, `hint` at the left, and what `buttons` adds at the
/// right (right to left: the main button first).
pub fn footer_with(ui: &mut Ui, hint: Option<&str>, buttons: impl FnOnce(&mut Ui)) {
    let p = style::pal(ui);
    let full = ui.max_rect();
    let y = ui.cursor().top();
    ui.painter().hline(full.x_range(), y, Stroke::new(1.0, p.border));
    egui::Frame::NONE
        .fill(style::over(p.surface, p.chrome, 0.6))
        .corner_radius(CornerRadius {
            nw: 0,
            ne: 0,
            sw: style::RADIUS_LG + 2,
            se: style::RADIUS_LG + 2,
        })
        .inner_margin(Margin { left: PAD, right: 16, top: 12, bottom: 12 })
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                if let Some(hint) = hint {
                    ui.label(RichText::new(hint).size(style::SMALL).color(p.text_faint));
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
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
                    ui.painter().text(egui::pos2(r.left() + 13.0 + 8.0, r.center().y), egui::Align2::LEFT_CENTER, text, egui::FontId::proportional(12.5), p.text);
                }
            });
        });
}

/// "↵ to confirm · Esc to cancel".
fn keys_hint(ok: &str) -> String {
    tf("↵ {} · Esc to cancel", &[ok])
}

impl App {
    pub fn dialogs(&mut self, ctx: &egui::Context) {
        let Some(mut dialog) = self.dialog.take() else { return };
        let mut keep = true;
        let enter = ctx.input(|i| i.key_pressed(Key::Enter));
        let p = style::palette(ctx.global_style().visuals.dark_mode);
        let modal = egui::Modal::new(egui::Id::new("dialog"))
            .frame(frame(ctx))
            .backdrop_color(Color32::from_black_alpha(if ctx.global_style().visuals.dark_mode { 130 } else { 70 }))
            .show(ctx, |ui| match &mut dialog {
                Dialog::AddProject { path, name, then, found } => {
                    ui.set_width(520.0);
                    if header(
                        ui,
                        icons::FOLDER_PLUS,
                        p.accent,
                        t("Add Project"),
                        Some(t("A project is a folder: its terminals start there, and its files and changes show beside it.")),
                    ) {
                        keep = false;
                    }
                    let presets = self.presets();
                    body(ui, |ui| {
                        field_label(ui, t("Folder"));
                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing.x = 8.0;
                            let choose_w = kit::button_width(ui, Some(icons::FOLDER_OPEN), t("Choose…"), None, false);
                            let r = folder_field(ui, path, "~/Developments/app", choose_w + 8.0);
                            if path.is_empty() && !r.has_focus() && !r.lost_focus() {
                                r.request_focus();
                            }
                            if kit::button_ex(ui, kit::Kind::Secondary, Some(icons::FOLDER_OPEN), t("Choose…"), false)
                                .clicked()
                                && let Some(dir) = rfd::FileDialog::new().pick_folder()
                            {
                                *path = dir.display().to_string();
                            }
                        });
                        // What is at that folder (looked at again when the path changes).
                        let typed = path.trim().to_string();
                        if found.as_ref().is_none_or(|(checked, _)| *checked != typed) {
                            *found = Some((typed.clone(), folder_summary(&typed)));
                        }
                        if let Some((_, Some(line))) = found {
                            ui.add_space(2.0);
                            ui.horizontal(|ui| {
                                ui.spacing_mut().item_spacing.x = 5.0;
                                ui.label(RichText::new(icons::CHECK_CIRCLE).font(style::icon_font(12.0)).color(p.ok));
                                ui.label(RichText::new(line.as_str()).size(style::SMALL).color(p.text_faint));
                            });
                        }
                        ui.add_space(8.0);
                        field_label(ui, t("Name"));
                        let folder_name = std::path::Path::new(path.trim())
                            .file_name()
                            .map(|n| n.to_string_lossy().into_owned())
                            .unwrap_or_default();
                        let hint = if folder_name.is_empty() { t("The folder's name").to_string() } else { folder_name };
                        text_field(ui, name, &hint, 0.0);
                        // An agent to start in it once added.
                        ui.add_space(8.0);
                        ui.horizontal(|ui| {
                            ui.label(RichText::new(t("Then run")).size(style::SMALL).color(p.text_muted));
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                let mut options = vec![kit::Seg::new(t("Nothing"), "")];
                                options.extend(presets.iter().take(3).map(|x| kit::Seg::new(x.name.as_str(), x.command.as_str())));
                                if let Some(i) = kit::segmented(ui, &options, *then) {
                                    *then = i;
                                }
                            });
                        });
                        ui.add_space(10.0);
                        ui.label(
                            RichText::new(tf("Tip: drop a folder on the window, or type {} in any terminal.", &["ternarycode ."]))
                                .size(style::SMALL)
                                .color(p.text_faint),
                        );
                    });
                    let ready = !path.trim().is_empty();
                    let (cancel, ok) = footer(ui, Some(&keys_hint(t("Add"))), t("Add Project"), Confirm::Primary, ready);
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
                            self.actions.push(Action::RunAgent(preset.id.clone(), false, 1));
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
                    let (cancel, ok) = footer(ui, Some(&keys_hint(t("Uninstall"))), t("Uninstall"), Confirm::Danger, true);
                    if cancel {
                        keep = false;
                        self.actions.push(Action::Integrations);
                    }
                    if ok || enter {
                        self.actions.push(Action::UninstallNow(*agent));
                        self.actions.push(Action::Integrations);
                        keep = false;
                    }
                }
                Dialog::RunAgent { selected, resume, place } => {
                    keep = self.run_agent_dialog(ui, selected, resume, place, enter);
                }
                Dialog::ConfirmCloseFile(i, name) => {
                    ui.set_width(460.0);
                    if header(ui, icons::FLOPPY_DISK, p.warn, &tf("Save changes to {}?", &[name]), Some(t("Your changes are lost if you don’t save them."))) {
                        keep = false;
                    }
                    ui.add_space(12.0);
                    let mut save = enter;
                    footer_with(ui, Some(&tf("↵ {}", &[t("Save")])), |ui| {
                        let okr = kit::button_ex(ui, kit::Kind::Primary, None, t("Save"), false);
                        #[cfg(debug_assertions)]
                        crate::devtest::record("dialog-ok", okr.rect);
                        save |= okr.clicked();
                        if kit::secondary(ui, None, t("Cancel")).clicked() {
                            keep = false;
                        }
                        if kit::button_ex(ui, kit::Kind::Ghost, None, t("Don’t Save"), false).clicked() {
                            self.actions.push(Action::DiscardAndCloseFile(*i));
                            keep = false;
                        }
                    });
                    if save {
                        self.actions.push(Action::SaveFile);
                        self.actions.push(Action::CloseFile(*i));
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
                    if header(
                        ui,
                        icons::ARROW_CIRCLE_UP,
                        p.accent,
                        t("Restart to update?"),
                        Some(t("Programs still running are stopped, and changes not saved are lost:")),
                    ) {
                        keep = false;
                    }
                    body(ui, |ui| {
                        busy_list(ui, busy);
                        ui.add_space(4.0);
                        ui.label(
                            RichText::new(t("TernaryCode opens again with your projects and tabs; agents that support it can be resumed."))
                                .size(style::SMALL)
                                .color(p.text_muted),
                        );
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
                Dialog::ConfirmClosePane(id, what) => {
                    ui.set_width(440.0);
                    if header(
                        ui,
                        icons::X_CIRCLE,
                        p.warn,
                        t("Close this pane?"),
                        Some(&tf("{} is still running in it and will be stopped.", &[what])),
                    ) {
                        keep = false;
                    }
                    ui.add_space(12.0);
                    let (cancel, ok) = footer(ui, Some(&keys_hint(t("Close Pane"))), t("Close Pane"), Confirm::Danger, true);
                    if cancel {
                        keep = false;
                    }
                    if ok || enter {
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
                        if !r.has_focus() && !r.lost_focus() {
                            r.request_focus();
                        }
                    });
                    let ready = !name.trim().is_empty();
                    let (cancel, ok) = footer(ui, Some(&keys_hint(t("Rename"))), t("Rename"), Confirm::Primary, ready);
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
                        if !r.has_focus() && !r.lost_focus() {
                            r.request_focus();
                        }
                    });
                    let (cancel, ok) = footer(ui, Some(&keys_hint(t("Rename"))), t("Rename"), Confirm::Primary, true);
                    if cancel {
                        keep = false;
                    }
                    if ok || enter {
                        self.actions.push(Action::RenamePane(*id, name.trim().to_string()));
                        keep = false;
                    }
                }
                Dialog::Install(plan, diff) => {
                    ui.set_width(680.0);
                    if header(
                        ui,
                        icons::PLUGS_CONNECTED,
                        p.accent,
                        &tf("Install hooks for {}?", &[plan.agent.label()]),
                        Some(&tf("TernaryCode will add its hooks to your {} settings.", &[plan.agent.label()])),
                    ) {
                        keep = false;
                        self.actions.push(Action::Integrations);
                    }
                    body(ui, |ui| install_preview(ui, &plan.path, diff));
                    let (cancel, ok) = footer(ui, Some(t("Review the change, then Install")), t("Install"), Confirm::Primary, true);
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
            if matches!(dialog, Dialog::Install(..)) {
                self.actions.push(Action::Integrations);
            }
        }
        // `ConfirmInstall` takes the dialog itself.
        let install_confirmed = self.actions.contains(&Action::ConfirmInstall);
        if keep || install_confirmed {
            self.dialog = Some(dialog);
        } else {
            self.focus_request = self.current_pane();
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
    let git = branch.map(|b| tf("git, branch {}", &[&b]));
    let parts: Vec<String> = [Some(kind.to_string()).filter(|k| !k.is_empty()), git].into_iter().flatten().collect();
    Some(if parts.is_empty() { t("A folder").to_string() } else { parts.join(" · ") })
}

/// The change Install makes: the file, how many lines it adds and removes, and the lines
/// themselves with their numbers (long runs of unchanged lines folded).
fn install_preview(ui: &mut Ui, path: &std::path::Path, diff: &str) {
    let p = style::pal(ui);
    let lines: Vec<&str> = diff.lines().collect();
    let added = lines.iter().filter(|l| l.starts_with("+ ")).count();
    let removed = lines.iter().filter(|l| l.starts_with("- ")).count();
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        ui.label(RichText::new(icons::FILE_CODE).font(style::icon_font(13.0)).color(p.text_muted));
        ui.label(RichText::new(crate::paths::contract_tilde(path)).size(style::SMALL).color(p.text_muted));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let what = if removed == 0 { t("nothing removed").to_string() } else { tf("−{} lines", &[&removed.to_string()]) };
            ui.label(RichText::new(format!("· {what}")).size(style::SMALL).color(p.text_faint));
            ui.label(RichText::new(tf("+{} lines", &[&added.to_string()])).size(style::SMALL).color(p.ok));
        });
    });
    ui.add_space(4.0);
    // Unchanged lines more than three away from a change are folded.
    let near = |i: usize| lines[i.saturating_sub(3)..(i + 4).min(lines.len())].iter().any(|l| l.starts_with("+ ") || l.starts_with("- "));
    egui::Frame::NONE
        .fill(p.canvas)
        .stroke(Stroke::new(1.0, p.border))
        .corner_radius(CornerRadius::same(7))
        .inner_margin(Margin::symmetric(0, 6))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            egui::ScrollArea::vertical().max_height(300.0).show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 0.0;
                let font = egui::FontId::monospace(12.0);
                let mut number = 0;
                let mut folded = 0;
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
                        let (r, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 19.0), egui::Sense::hover());
                        ui.painter().text(egui::pos2(r.left() + 62.0, r.center().y), egui::Align2::LEFT_CENTER, tf("⋯ {} unchanged lines", &[&folded.to_string()]), egui::FontId::proportional(11.5), p.text_faint);
                        folded = 0;
                    }
                    let (r, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 19.0), egui::Sense::hover());
                    let (bg, sc) = if add { (style::soft(p.ok, 0.10), p.ok) } else if del { (style::soft(p.err, 0.10), p.err) } else { (Color32::TRANSPARENT, p.text_faint) };
                    ui.painter().rect_filled(r, CornerRadius::ZERO, bg);
                    if !del {
                        ui.painter().text(egui::pos2(r.left() + 36.0, r.center().y), egui::Align2::RIGHT_CENTER, number.to_string(), font.clone(), p.text_faint);
                    }
                    ui.painter().text(egui::pos2(r.left() + 44.0 + 9.0, r.center().y), egui::Align2::CENTER_CENTER, if add { "+" } else if del { "−" } else { "" }, font.clone(), sc);
                    ui.painter().text(egui::pos2(r.left() + 62.0, r.center().y), egui::Align2::LEFT_CENTER, text, font.clone(), if add || del { p.text } else { p.text_muted });
                }
                if folded > 0 {
                    let (r, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 19.0), egui::Sense::hover());
                    ui.painter().text(egui::pos2(r.left() + 62.0, r.center().y), egui::Align2::LEFT_CENTER, tf("⋯ {} unchanged lines", &[&folded.to_string()]), egui::FontId::proportional(11.5), p.text_faint);
                }
            });
        });
    ui.add_space(8.0);
    ui.horizontal_top(|ui| {
        ui.spacing_mut().item_spacing.x = 10.0;
        ui.label(RichText::new(icons::INFO).font(style::icon_font(14.0)).color(p.text_faint));
        ui.add(
            egui::Label::new(
                RichText::new(t("A backup is kept, and Uninstall puts the file back exactly as it is now. The hooks do nothing when the agent runs outside TernaryCode."))
                    .size(12.5)
                    .color(p.text_muted),
            )
            .wrap(),
        );
    });
}

impl App {
    /// The Run Agent dialog: which agent (with how it tells TernaryCode that it needs you),
    /// whether to continue its last session, and where it runs. Returns whether it stays open.
    fn run_agent_dialog(&mut self, ui: &mut Ui, selected: &mut usize, resume: &mut bool, place: &mut usize, enter: bool) -> bool {
        let p = style::pal(ui);
        let presets = self.presets();
        let project = self.active_project().map(|x| (x.name.clone(), crate::paths::contract_tilde(&x.path))).unwrap_or_default();
        ui.set_width(560.0);
        let mut keep = true;
        if header(ui, icons::PLAY, p.accent, t("Run an agent"), Some(&tf("in {} · {}", &[&project.0, &project.1]))) {
            keep = false;
        }
        // Digits choose.
        let n = presets.len();
        ui.input(|i| {
            for (k, d) in [(Key::Num1, 0), (Key::Num2, 1), (Key::Num3, 2), (Key::Num4, 3), (Key::Num5, 4), (Key::Num6, 5), (Key::Num7, 6), (Key::Num8, 7), (Key::Num9, 8)] {
                if d < n && i.key_pressed(k) {
                    *selected = d;
                }
            }
            if i.key_pressed(Key::ArrowDown) && n > 0 {
                *selected = (*selected + 1) % n;
            }
            if i.key_pressed(Key::ArrowUp) && n > 0 {
                *selected = (*selected + n - 1) % n;
            }
        });
        *selected = (*selected).min(n.saturating_sub(1));
        let mut run = enter;
        body(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            for (i, preset) in presets.iter().enumerate() {
                let (rect, resp) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 52.0), egui::Sense::click());
                #[cfg(debug_assertions)]
                crate::devtest::record(format!("agent:{}", preset.id), rect);
                let on = i == *selected;
                let painter = ui.painter();
                if on {
                    painter.rect_filled(rect, CornerRadius::same(8), p.accent_soft);
                } else if resp.hovered() {
                    painter.rect_filled(rect, CornerRadius::same(8), p.faint_row());
                }
                let cy = rect.center().y;
                painter.text(egui::pos2(rect.left() + 12.0 + 5.0, cy), egui::Align2::CENTER_CENTER, (i + 1).to_string(), egui::FontId::proportional(11.0), p.text_faint);
                let tile = egui::Rect::from_center_size(egui::pos2(rect.left() + 12.0 + 10.0 + 12.0 + 15.0, cy), egui::vec2(30.0, 30.0));
                painter.rect_filled(tile, CornerRadius::same(8), p.subtle);
                let mark = match preset.agent {
                    Some(tc_core::agent::AgentKind::Claude) => "CC".to_string(),
                    Some(tc_core::agent::AgentKind::Codex) => "Cx".to_string(),
                    Some(tc_core::agent::AgentKind::Gemini) => "Gm".to_string(),
                    Some(tc_core::agent::AgentKind::OpenCode) => "OC".to_string(),
                    _ => initials(&preset.name),
                };
                painter.text(tile.center(), egui::Align2::CENTER_CENTER, mark, style::semibold(11.5), p.text_muted);
                let x = tile.right() + 12.0;
                painter.text(egui::pos2(x, cy - 8.0), egui::Align2::LEFT_CENTER, &preset.name, style::medium(13.5), p.text);
                painter.text(egui::pos2(x, cy + 9.0), egui::Align2::LEFT_CENTER, &preset.command, egui::FontId::monospace(11.0), p.text_faint);
                // How its status reaches TernaryCode.
                let agent = preset.agent.unwrap_or(tc_core::agent::AgentKind::Custom);
                let note = self.hooks_note(agent);
                let hooked = self.installed(agent).is_some()
                    || (matches!(agent, tc_core::agent::AgentKind::Claude | tc_core::agent::AgentKind::Codex)
                        && self.settings.integrations.get(agent.slug()).copied().unwrap_or_default() == tc_core::model::Integration::PerLaunch);
                let (icon, text, fg, bg) = if hooked {
                    (icons::PLUGS_CONNECTED, tf("Hooks · {}", &[note.trim_start_matches("hooks · ")]), p.ok, style::tint(p.ok, ui.visuals().dark_mode))
                } else if agent == tc_core::agent::AgentKind::Custom {
                    (icons::CIRCLE_HALF, t("Custom").to_string(), p.text_muted, p.subtle)
                } else {
                    (icons::CIRCLE_HALF, t("No hooks · shows “Active”").to_string(), p.text_muted, p.subtle)
                };
                let w = kit::pill_width(painter, Some(icon), &text);
                kit::paint_pill(painter, egui::pos2(rect.right() - 12.0 - w, cy), Some(icon), &text, fg, bg);
                let resp = if hooked { resp } else { resp.on_hover_text(t("Without hooks the status comes from the terminal: Settings → Agents")) };
                if resp.clicked() {
                    *selected = i;
                }
                if resp.double_clicked() {
                    run = true;
                }
            }
            let rect = ui.available_rect_before_wrap();
            ui.add_space(10.0);
            ui.painter().hline(rect.x_range(), ui.cursor().top(), Stroke::new(1.0, p.border));
            ui.add_space(12.0);
            let chosen = presets.get(*selected);
            let resume_cmd = chosen.and_then(|x| x.resume.clone()).filter(|r| !r.trim().is_empty());
            ui.horizontal(|ui| {
                ui.add_space(4.0);
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = 2.0;
                    ui.label(RichText::new(t("Continue the last session")).font(style::medium(13.0)).color(p.text));
                    let note = match &resume_cmd {
                        Some(cmd) => tf("Runs {}", &[cmd]),
                        None => t("This agent has no command that resumes a session.").to_string(),
                    };
                    ui.label(RichText::new(note).size(style::SMALL).color(p.text_faint));
                });
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.add_space(4.0);
                    ui.add_enabled_ui(resume_cmd.is_some(), |ui| kit::switch(ui, resume));
                });
            });
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                ui.add_space(4.0);
                ui.label(RichText::new(t("Open in")).font(style::medium(13.0)).color(p.text));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.add_space(4.0);
                    let options = [kit::Seg::new(t("New Tab"), ""), kit::Seg::new(t("Split Right"), ""), kit::Seg::new(t("Split Down"), "")];
                    if let Some(i) = kit::segmented(ui, &options, *place) {
                        *place = i;
                    }
                });
            });
            if resume_cmd.is_none() {
                *resume = false;
            }
        });
        let name = presets.get(*selected).map(|x| x.name.clone()).unwrap_or_default();
        let hint = tf("1–{} choose · ↵ run · Esc cancel", &[&n.min(9).to_string()]);
        footer_with(ui, Some(&hint), |ui| {
            let r = kit::button_keys(ui, kit::Kind::Primary, None, &tf("Run {}", &[&name]), Some("↵"), false);
            #[cfg(debug_assertions)]
            crate::devtest::record("dialog-ok", r.rect);
            run |= r.clicked();
            if kit::secondary(ui, None, t("Cancel")).clicked() {
                keep = false;
            }
        });
        if run && let Some(preset) = presets.get(*selected) {
            self.actions.push(Action::RunAgent(preset.id.clone(), *resume, *place));
            keep = false;
        }
        keep
    }
}

/// Two letters for an agent's tile ("Claude Code" → "CC", "Codex" → "Cx").
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
