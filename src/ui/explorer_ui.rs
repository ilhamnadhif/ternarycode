//! The Files section of the sidebar: the file tree of the active project, with what git says
//! changed. Clicking a file opens it in the editor beside the terminal. As in other editors,
//! files and folders are created and renamed in place, and the right-click menu has the rest
//! (duplicate, move to the Trash, open a terminal there, copy the path, ...).

use std::path::{Path, PathBuf};

use eframe::egui::{self, Align2, CornerRadius, Rect, Sense, Ui, pos2, vec2};

use super::sidebar::{SECTION_H, section_title};
use super::{icons, kit, style};
use crate::actions::Action;
use crate::app::App;
use crate::explorer::{EditKind, Entry, Row};
use crate::i18n::t;
use crate::project::Change;

const ROW_H: f32 = 22.0;
const INDENT: f32 = 12.0;

/// What a click in the tree asks for (applied after drawing).
enum Click {
    Toggle(PathBuf),
    Open(PathBuf),
}

/// What happened in the name field this frame.
enum EditOutcome {
    Commit,
    Cancel,
}

impl App {
    pub fn files_section(&mut self, ui: &mut Ui, open: bool) {
        let p = style::pal(ui);
        let area = ui.max_rect();
        let title = Rect::from_min_size(area.min, vec2(area.width(), SECTION_H));
        let Some(pid) = self.active else {
            section_title(ui, title, t("FILES"));
            return;
        };

        // Title: click folds the section; buttons on the right.
        let fold = ui.interact(title, egui::Id::new("files-title"), Sense::click());
        if fold.hovered() {
            ui.painter().rect_filled(title, CornerRadius::ZERO, p.faint_row());
        }
        ui.painter().text(
            pos2(title.left() + 10.0, title.center().y),
            Align2::CENTER_CENTER,
            if open { icons::CARET_DOWN } else { icons::CARET_RIGHT },
            style::icon_font(10.0),
            p.text_faint,
        );
        let mut title_text = title;
        title_text.min.x += 8.0;
        section_title(ui, title_text, t("FILES"));
        if fold
            .on_hover_text(if open { t("Hide Files") } else { t("Show Files") })
            .clicked()
        {
            self.actions.push(Action::ToggleFiles);
        }
        if !open {
            return;
        }
        let root = self.project(pid).map(|x| x.path.clone()).unwrap_or_default();
        // New items go into the selected folder, or beside the selected file, as in VS Code.
        let target_dir = self
            .explorers
            .get(&pid)
            .and_then(|e| e.selected.clone())
            .map(|s| if s.is_dir() { s } else { s.parent().map(Path::to_path_buf).unwrap_or_default() })
            .filter(|d| d.starts_with(&root))
            .unwrap_or_else(|| root.clone());
        let buttons: [(&str, String, Option<Action>); 5] = [
            (icons::ARROWS_CLOCKWISE, t("Refresh").to_string(), None),
            (icons::CARET_UP, t("Collapse All Folders").to_string(), None),
            (icons::MAGNIFYING_GLASS, kit::tip(t("Open File…"), Action::QuickOpen), Some(Action::QuickOpen)),
            (icons::FOLDER_PLUS, t("New Folder…").to_string(), Some(Action::NewFolderIn(target_dir.clone()))),
            (icons::PLUS, t("New File…").to_string(), Some(Action::NewFileIn(target_dir.clone()))),
        ];
        let mut x = title.right() - 8.0;
        for (i, (icon, tip, action)) in buttons.into_iter().enumerate() {
            let r = Rect::from_center_size(pos2(x - 11.0, title.center().y), vec2(22.0, 22.0));
            x -= 23.0;
            let resp = ui
                .scope_builder(egui::UiBuilder::new().max_rect(r), |ui| kit::icon_button_ex(ui, icon, &tip, false, 22.0))
                .inner;
            #[cfg(debug_assertions)]
            crate::devtest::record(format!("files-button:{i}"), resp.rect);
            if resp.clicked() {
                match (i, action) {
                    (_, Some(a)) => self.actions.push(a),
                    (0, None) => {
                        if let Some(ex) = self.explorer_mut(pid) {
                            ex.refresh();
                        }
                        self.refresh_git(pid);
                    }
                    _ => {
                        if let Some(ex) = self.explorer_mut(pid) {
                            ex.collapse_all();
                        }
                    }
                }
            }
        }

        // The tree.
        let tree = Rect::from_min_max(pos2(area.left(), title.bottom()), area.max);
        let editing = self
            .editors
            .get(&pid)
            .filter(|e| e.visible)
            .and_then(|e| e.file())
            .map(|f| f.path.clone());
        let git = self.project(pid).and_then(|pr| pr.git.clone());
        let Some(explorer) = self.explorer_mut(pid) else { return };
        let count = explorer.rows().len();
        let selected = editing.or_else(|| explorer.selected.clone());
        // A name field that just opened is scrolled into view.
        let opening = explorer.edit.as_ref().filter(|e| e.focus).map(|e| e.kind.clone());
        let scroll_to = opening.and_then(|kind| {
            explorer
                .rows()
                .iter()
                .position(|r| match (&kind, r) {
                    (EditKind::Rename(path), Row::Item { entry, .. }) => entry.path == *path,
                    (EditKind::NewFile | EditKind::NewFolder, Row::New { .. }) => true,
                    _ => false,
                })
                .map(|i| (i as f32 - 3.0).max(0.0) * ROW_H)
        });
        let mut click = None;
        let mut menu_actions = Vec::new();
        let mut outcome = None;
        let edit_id = egui::Id::new(("tree-edit", pid));
        ui.scope_builder(egui::UiBuilder::new().max_rect(tree.shrink2(vec2(4.0, 0.0))), |ui| {
            // Where the rows end on screen (the blank below them has the folder's menu).
            let mut rows_end = None;
            if count == 0 {
                ui.add_space(8.0);
                ui.label(egui::RichText::new(t("This folder is empty.")).size(style::SMALL).color(p.text_faint));
            } else {
                let mut area = egui::ScrollArea::vertical().id_salt(("files", pid)).auto_shrink([false, false]);
                if let Some(y) = scroll_to {
                    area = area.vertical_scroll_offset(y);
                }
                // The area fills the tree, so the blank is inside it: below the last row, once
                // that is in view.
                rows_end = kit::show_rows(area, ui, ROW_H, count, |ui, range| {
                    let last = range.end == count;
                    let rows: Vec<Row> = self
                        .explorers
                        .get_mut(&pid)
                        .map(|ex| ex.rows()[range.clone()].to_vec())
                        .unwrap_or_default();
                    for row in rows {
                        let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), ROW_H), Sense::click());
                        let painter = ui.painter_at(rect);
                        match row {
                            Row::More { depth, count } => {
                                painter.text(
                                    pos2(rect.left() + 8.0 + depth as f32 * INDENT + 30.0, rect.center().y),
                                    Align2::LEFT_CENTER,
                                    crate::i18n::tf("… {} more", &[&count.to_string()]),
                                    egui::FontId::proportional(style::SMALL),
                                    p.text_faint,
                                );
                            }
                            Row::New { depth, folder } => {
                                let x = rect.left() + 8.0 + depth as f32 * INDENT + 14.0;
                                let icon = if folder { icons::FOLDER } else { icons::FILE };
                                painter.text(pos2(x + 7.0, rect.center().y), Align2::CENTER_CENTER, icon, style::icon_font(14.0), p.folder);
                                let field = Rect::from_min_max(pos2(x + 18.0, rect.top() + 1.0), pos2(rect.right() - 4.0, rect.bottom() - 1.0));
                                if let Some(o) = self.name_field(ui, pid, field, edit_id) {
                                    outcome = Some(o);
                                }
                            }
                            Row::Item { depth, entry, open } => {
                                #[cfg(debug_assertions)]
                                crate::devtest::record(
                                    format!("tree:{}", entry.path.strip_prefix(&root).unwrap_or(&entry.path).display()),
                                    rect,
                                );
                                let renaming = self
                                    .explorers
                                    .get(&pid)
                                    .and_then(|e| e.edit.as_ref())
                                    .is_some_and(|e| matches!(&e.kind, EditKind::Rename(path) if *path == entry.path));
                                let is_sel = selected.as_deref() == Some(entry.path.as_path());
                                if is_sel {
                                    painter.rect_filled(rect, CornerRadius::same(4), p.accent_soft);
                                } else if resp.hovered() {
                                    painter.rect_filled(rect, CornerRadius::same(4), p.faint_row());
                                }
                                let change = git.as_ref().and_then(|g| g.changes.get(&entry.path)).copied();
                                let dir_changed = entry.is_dir
                                    && git.as_ref().is_some_and(|g| g.changed_dirs.contains(&entry.path));
                                let mut x = rect.left() + 8.0 + depth as f32 * INDENT;
                                if entry.is_dir {
                                    painter.text(
                                        pos2(x + 5.0, rect.center().y),
                                        Align2::CENTER_CENTER,
                                        if open { icons::CARET_DOWN } else { icons::CARET_RIGHT },
                                        style::icon_font(10.0),
                                        p.text_faint,
                                    );
                                }
                                x += 14.0;
                                let (icon, mut icon_color) = if entry.is_dir {
                                    (if open { icons::FOLDER_OPEN } else { icons::FOLDER }, p.folder)
                                } else {
                                    icons::file_icon(&entry.name, false, entry.is_link, &p)
                                };
                                let mut name_color = if is_sel { p.text } else { style::tree_text(&p) };
                                if let Some(c) = change {
                                    name_color = change_color(c, &p);
                                }
                                if entry.dimmed {
                                    name_color = p.text_faint;
                                    icon_color = style::soft(icon_color, 0.55);
                                }
                                painter.text(
                                    pos2(x + 7.0, rect.center().y),
                                    Align2::CENTER_CENTER,
                                    icon,
                                    style::icon_font(14.0),
                                    icon_color,
                                );
                                x += 20.0;
                                if renaming {
                                    let field = Rect::from_min_max(pos2(x - 2.0, rect.top() + 1.0), pos2(rect.right() - 4.0, rect.bottom() - 1.0));
                                    if let Some(o) = self.name_field(ui, pid, field, edit_id) {
                                        outcome = Some(o);
                                    }
                                    continue;
                                }
                                // Right: the change letter, or a dot on folders holding changes.
                                let mut right = rect.right() - 8.0;
                                if let Some(c) = change {
                                    let g = painter.layout_no_wrap(
                                        c.letter().to_string(),
                                        style::medium(style::SMALL),
                                        change_color(c, &p),
                                    );
                                    right -= g.size().x;
                                    painter.galley(pos2(right, rect.center().y - g.size().y / 2.0), g, name_color);
                                    right -= 6.0;
                                } else if dir_changed {
                                    painter.circle_filled(pos2(right - 3.0, rect.center().y), 3.0, style::soft(p.warn, 0.8));
                                    right -= 12.0;
                                }
                                let mut job = egui::text::LayoutJob::simple_singleline(
                                    entry.name.clone(),
                                    egui::FontId::proportional(style::BODY - 0.5),
                                    name_color,
                                );
                                job.wrap = egui::text::TextWrapping::truncate_at_width((right - x).max(10.0));
                                let g = painter.layout_job(job);
                                painter.galley(pos2(x, rect.center().y - g.size().y / 2.0), g, name_color);

                                if resp.clicked() {
                                    click = Some(if entry.is_dir {
                                        Click::Toggle(entry.path.clone())
                                    } else {
                                        Click::Open(entry.path.clone())
                                    });
                                }
                                if resp.secondary_clicked()
                                    && let Some(ex) = self.explorers.get_mut(&pid)
                                {
                                    ex.selected = Some(entry.path.clone());
                                }
                                resp.context_menu(|ui| {
                                    tree_menu(ui, &entry, &root, &mut menu_actions);
                                });
                            }
                        }
                    }
                    last.then(|| ui.min_rect().bottom())
                })
                .inner;
            }
            // The empty space below the rows: a menu for the project folder itself.
            let top = rows_end.unwrap_or(ui.min_rect().bottom());
            let rest = Rect::from_min_max(pos2(tree.left(), top.min(tree.bottom())), tree.max);
            if rest.height() > 4.0 {
                let r = ui.interact(rest, egui::Id::new(("tree-blank", pid)), Sense::click());
                r.context_menu(|ui| {
                    new_items_menu(ui, &root, &mut menu_actions);
                    ui.separator();
                    if ui.button(t("Open in Terminal")).clicked() {
                        menu_actions.push(Action::NewTabAt(root.clone()));
                        ui.close();
                    }
                    if ui.button(reveal_label()).clicked() {
                        menu_actions.push(Action::RevealPath(root.clone()));
                        ui.close();
                    }
                });
            }
        });
        match click {
            Some(Click::Toggle(dir)) => {
                if let Some(ex) = self.explorer_mut(pid) {
                    ex.toggle(&dir);
                    ex.selected = Some(dir);
                }
            }
            Some(Click::Open(path)) => self.actions.push(Action::OpenFile(path, None)),
            None => {}
        }
        match outcome {
            Some(EditOutcome::Commit) => self.commit_tree_edit(pid),
            Some(EditOutcome::Cancel) => {
                if let Some(ex) = self.explorer_mut(pid) {
                    ex.cancel_edit();
                }
            }
            None => {}
        }
        self.actions.extend(menu_actions);
    }

    /// The name field of a new or renamed item. Enter (or clicking elsewhere) keeps the name,
    /// Esc gives up.
    fn name_field(&mut self, ui: &mut Ui, pid: tc_core::ProjectId, rect: Rect, id: egui::Id) -> Option<EditOutcome> {
        let edit = self.explorers.get_mut(&pid)?.edit.as_mut()?;
        let r = ui.put(
            rect,
            egui::TextEdit::singleline(&mut edit.text)
                .id(id)
                .font(egui::FontId::proportional(style::BODY - 0.5))
                .margin(egui::Margin::symmetric(4, 1)),
        );
        #[cfg(debug_assertions)]
        crate::devtest::record("tree-edit", r.rect);
        if edit.focus {
            r.request_focus();
            // Renaming: the name without its extension is selected, as in other editors.
            let len = match &edit.kind {
                EditKind::Rename(_) => edit
                    .text
                    .rfind('.')
                    .filter(|i| *i > 0)
                    .map_or(edit.text.chars().count(), |i| edit.text[..i].chars().count()),
                _ => 0,
            };
            kit::select_text(ui.ctx(), id, 0, len);
            edit.focus = false;
            return None;
        }
        if r.lost_focus() {
            let esc = ui.input(|i| i.key_pressed(egui::Key::Escape));
            return Some(if esc || edit.text.trim().is_empty() {
                EditOutcome::Cancel
            } else {
                EditOutcome::Commit
            });
        }
        None
    }

    fn commit_tree_edit(&mut self, pid: tc_core::ProjectId) {
        let Some(ex) = self.explorer_mut(pid) else { return };
        let kind = ex.edit.as_ref().map(|e| e.kind.clone());
        match ex.commit_edit() {
            Ok(Some((path, old))) => {
                if let Some(old) = old {
                    self.actions.push(Action::PathRenamed { old, new: path });
                } else if kind == Some(EditKind::NewFile) {
                    self.actions.push(Action::OpenFile(path, None));
                }
            }
            Ok(None) => {}
            Err(e) => {
                // The field stays, with the keyboard, so the name can be fixed.
                if let Some(edit) = self.explorer_mut(pid).and_then(|x| x.edit.as_mut()) {
                    edit.focus = true;
                }
                self.toast(e);
            }
        }
    }
}

fn reveal_label() -> &'static str {
    if cfg!(target_os = "macos") {
        t("Show in Finder")
    } else {
        t("Show in Folder")
    }
}

fn new_items_menu(ui: &mut Ui, dir: &Path, out: &mut Vec<Action>) {
    if ui.button(t("New File…")).clicked() {
        out.push(Action::NewFileIn(dir.to_path_buf()));
        ui.close();
    }
    if ui.button(t("New Folder…")).clicked() {
        out.push(Action::NewFolderIn(dir.to_path_buf()));
        ui.close();
    }
}

fn tree_menu(ui: &mut Ui, entry: &Entry, root: &Path, out: &mut Vec<Action>) {
    let mut item = |ui: &mut Ui, label: &str, action: Action| {
        if ui.button(label).clicked() {
            out.push(action);
            ui.close();
        }
    };
    let dir = if entry.is_dir {
        entry.path.clone()
    } else {
        entry.path.parent().map(Path::to_path_buf).unwrap_or_else(|| root.to_path_buf())
    };
    if entry.is_dir {
        item(ui, t("New File…"), Action::NewFileIn(dir.clone()));
        item(ui, t("New Folder…"), Action::NewFolderIn(dir.clone()));
        item(ui, t("Open in Terminal"), Action::NewTabAt(dir));
    } else {
        item(ui, t("Open in Editor"), Action::OpenFile(entry.path.clone(), None));
        item(ui, t("New File…"), Action::NewFileIn(dir.clone()));
        item(ui, t("New Folder…"), Action::NewFolderIn(dir));
    }
    item(ui, t("Insert Path in Terminal"), Action::InsertPath(entry.path.clone()));
    ui.separator();
    item(ui, t("Rename…"), Action::RenamePath(entry.path.clone()));
    if !entry.is_dir {
        item(ui, t("Duplicate"), Action::DuplicatePath(entry.path.clone()));
    }
    item(ui, t("Move to Trash"), Action::TrashPath(entry.path.clone()));
    ui.separator();
    if ui.button(t("Copy Path")).clicked() {
        ui.ctx().copy_text(entry.path.to_string_lossy().into_owned());
        ui.close();
    }
    if ui.button(t("Copy Relative Path")).clicked() {
        let rel = entry.path.strip_prefix(root).unwrap_or(&entry.path);
        ui.ctx().copy_text(rel.to_string_lossy().into_owned());
        ui.close();
    }
    let mut item = |ui: &mut Ui, label: &str, action: Action| {
        if ui.button(label).clicked() {
            out.push(action);
            ui.close();
        }
    };
    item(ui, reveal_label(), Action::RevealPath(entry.path.clone()));
}

/// The colour of a changed file's name and letter.
pub fn change_color(c: Change, p: &style::Palette) -> egui::Color32 {
    match c {
        Change::Modified => p.warn,
        Change::Added | Change::Untracked => p.ok,
        Change::Deleted | Change::Conflict => p.err,
        Change::Renamed => p.info,
    }
}
