//! Modal dialogs: add project, quit and remove confirmations (FR-L2, FR-P6), rename, the diff
//! shown before installing hooks, and messages.
//!
//! Every dialog has the same parts: a header (an icon in a soft circle, the title, a line
//! saying what happens, a close button), the body with its fields, and a footer with a hint at
//! the left and the buttons at the right (Esc and a click outside cancel, Enter confirms).

use std::path::PathBuf;

use eframe::egui::{self, Color32, CornerRadius, Key, Margin, RichText, Stroke, Ui};

use super::{icons, kit, style};
use crate::actions::Action;
use crate::app::{App, Dialog};
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
        .inner_margin(Margin { left: PAD, right: 12, top: 18, bottom: 4 })
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
                        ui.add_space(2.0);
                        ui.add(
                            egui::Label::new(RichText::new(sub).size(style::BODY - 0.5).color(p.text_muted)).wrap(),
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
        .inner_margin(Margin { left: PAD, right: PAD, top: 14, bottom: 18 })
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
    ui.label(RichText::new(text).font(style::medium(style::SMALL + 0.5)).color(p.text_muted));
}

/// A single-line text field the width of the dialog (minus `leave` for what follows it).
fn text_field(ui: &mut Ui, text: &mut String, hint: &str, leave: f32) -> egui::Response {
    ui.add(
        egui::TextEdit::singleline(text)
            .hint_text(hint)
            .desired_width((ui.available_width() - leave).max(80.0))
            .margin(Margin::symmetric(9, 6))
            .font(egui::FontId::proportional(style::BODY)),
    )
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
        ui.add_space(2.0);
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
        .fill(style::soft(p.chrome, if ui.visuals().dark_mode { 0.55 } else { 0.7 }))
        .corner_radius(CornerRadius {
            nw: 0,
            ne: 0,
            sw: style::RADIUS_LG + 2,
            se: style::RADIUS_LG + 2,
        })
        .inner_margin(Margin { left: PAD, right: PAD - 6, top: 12, bottom: 12 })
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                if let Some(hint) = hint {
                    ui.label(RichText::new(hint).size(style::SMALL).color(p.text_faint));
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    buttons(ui);
                });
            });
        });
}

/// Programs that will be stopped and files not saved (those of `unsaved`), in a quiet box.
fn busy_list(ui: &mut Ui, items: &[String], unsaved: &[String]) {
    let p = style::pal(ui);
    egui::Frame::NONE
        .fill(p.canvas)
        .stroke(Stroke::new(1.0, p.border))
        .corner_radius(CornerRadius::same(style::RADIUS + 1))
        .inner_margin(Margin::symmetric(12, 8))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            egui::ScrollArea::vertical().max_height(180.0).show(ui, |ui| {
                for b in items {
                    let file = unsaved.contains(b);
                    ui.horizontal(|ui| {
                        let icon = if file { icons::FILE_TEXT } else { icons::TERMINAL_WINDOW };
                        ui.label(RichText::new(icon).font(style::icon_font(13.0)).color(if file { p.warn } else { p.text_faint }));
                        ui.label(RichText::new(b).color(p.text));
                    });
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
                Dialog::AddProject { path, name } => {
                    ui.set_width(500.0);
                    if header(
                        ui,
                        icons::FOLDER_PLUS,
                        p.accent,
                        t("Add Project"),
                        Some(t("A project is a folder: its terminals start there, and its files and changes show beside them.")),
                    ) {
                        keep = false;
                    }
                    body(ui, |ui| {
                        field_label(ui, t("Folder"));
                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing.x = 8.0;
                            let choose_w = 96.0;
                            let r = text_field(ui, path, "~/Developments/app", choose_w + 8.0);
                            if path.is_empty() && !r.has_focus() {
                                r.request_focus();
                            }
                            if kit::button_ex(ui, kit::Kind::Secondary, Some(icons::FOLDER_OPEN), t("Choose…"), false)
                                .clicked()
                                && let Some(dir) = rfd::FileDialog::new().pick_folder()
                            {
                                *path = dir.display().to_string();
                            }
                        });
                        ui.add_space(8.0);
                        field_label(ui, t("Name (optional)"));
                        let folder_name = std::path::Path::new(path.trim())
                            .file_name()
                            .map(|n| n.to_string_lossy().into_owned())
                            .unwrap_or_default();
                        let hint = if folder_name.is_empty() { t("The folder's name").to_string() } else { folder_name };
                        text_field(ui, name, &hint, 0.0);
                    });
                    let ready = !path.trim().is_empty();
                    let (cancel, ok) = footer(ui, Some(&keys_hint(t("Add"))), t("Add"), Confirm::Primary, ready);
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
                    let unsaved = self.unsaved_lines();
                    body(ui, |ui| {
                        busy_list(ui, busy, &unsaved);
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
                    let unsaved = self.unsaved_lines();
                    body(ui, |ui| {
                        busy_list(ui, busy, &unsaved);
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
                    if header(
                        ui,
                        icons::TRASH,
                        p.err,
                        t("Remove project from the workspace?"),
                        Some(t("The folder is not deleted. Programs still running are stopped, and changes not saved are lost:")),
                    ) {
                        keep = false;
                    }
                    let unsaved = self.unsaved_lines();
                    body(ui, |ui| busy_list(ui, busy, &unsaved));
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
                    ui.add_space(14.0);
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
                        Some(&tf("{} is still running and will be stopped.", &[what])),
                    ) {
                        keep = false;
                    }
                    ui.add_space(14.0);
                    let (cancel, ok) = footer(ui, None, t("Close Pane"), Confirm::Danger, true);
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
                    body(ui, |ui| busy_list(ui, busy, &[]));
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
                        Some(t("Empty: name it after what runs in it")),
                    ) {
                        keep = false;
                    }
                    body(ui, |ui| {
                        field_label(ui, t("Name"));
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
                    ui.set_width(660.0);
                    if header(
                        ui,
                        icons::PLUGS_CONNECTED,
                        p.accent,
                        &tf("Install hooks for {}?", &[plan.agent.label()]),
                        Some(&tf(
                            "TernaryCode will change {}. The original is backed up and Uninstall puts it back exactly as it is now.",
                            &[&crate::paths::contract_tilde(&plan.path)],
                        )),
                    ) {
                        keep = false;
                        self.actions.push(Action::Integrations);
                    }
                    body(ui, |ui| {
                        egui::Frame::NONE
                            .fill(p.canvas)
                            .stroke(Stroke::new(1.0, p.border))
                            .corner_radius(CornerRadius::same(style::RADIUS + 1))
                            .inner_margin(Margin::same(10))
                            .show(ui, |ui| {
                                ui.set_width(ui.available_width());
                                egui::ScrollArea::vertical().max_height(320.0).show(ui, |ui| {
                                    for line in diff.lines() {
                                        let color = if line.starts_with("+ ") {
                                            p.ok
                                        } else if line.starts_with("- ") {
                                            p.err
                                        } else {
                                            p.text_muted
                                        };
                                        ui.label(RichText::new(line).monospace().size(11.5).color(color));
                                    }
                                });
                            });
                        ui.add_space(2.0);
                        ui.label(
                            RichText::new(t("The hooks do nothing when the agent runs outside TernaryCode."))
                                .size(style::SMALL)
                                .color(p.text_muted),
                        );
                    });
                    let (cancel, ok) = footer(ui, None, t("Install"), Confirm::Primary, true);
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

    /// The busy lists' lines for the editors' files with changes not saved (worded as in
    /// `App::unsaved_files`): a line is a file's by the files' state, not by its wording,
    /// which a language may put in any order.
    fn unsaved_lines(&self) -> Vec<String> {
        self.editors
            .values()
            .flat_map(|e| e.files.iter())
            .filter(|f| f.dirty() && !f.read_only)
            .map(|f| tf("{} · not saved", &[&f.name()]))
            .collect()
    }
}

/// A name given in the Add Project dialog, applied once the project exists.
pub type PendingRename = Option<(PathBuf, String)>;
