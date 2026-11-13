//! The Files section of the sidebar: the file tree of the active project, with what git says
//! changed. Clicking a file opens it in the editor beside the terminal. As in other editors,
//! files and folders are created and renamed in place, and the right-click menu has the rest
//! (duplicate, move to the Trash, open a terminal there, copy the path, ...).

use std::path::{Path, PathBuf};

use eframe::egui::{self, Align2, CornerRadius, Rect, Sense, Ui, pos2, vec2};

use super::{icons, kit, style};
use crate::actions::Action;
use crate::app::App;
use crate::explorer::{EditKind, Entry, Row};
use crate::i18n::t;
use crate::project::Change;

const ROW_H: f32 = 24.0;
/// The Files | Changes header and the buttons at the bottom of the column.
const HEAD_H: f32 = 36.0;
const FOOT_H: f32 = 34.0;
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
    /// The Files column of the Code drawer: Files | Changes at the top, a filter, the tree (or
    /// the files that match the filter, or what git says changed), and the buttons that make
    /// files and folders at the bottom.
    pub fn files_column(&mut self, ui: &mut Ui, area: Rect) {
        let p = style::pal(ui);
        let Some(pid) = self.active else { return };
        ui.painter().rect_filled(area, CornerRadius::ZERO, p.chrome);
        ui.painter().vline(area.right() - 0.5, area.y_range(), egui::Stroke::new(1.0, p.border));
        let root = self.project(pid).map(|x| x.path.clone()).unwrap_or_default();
        let git = self.project(pid).and_then(|pr| pr.git.clone());
        let changed = git.as_ref().map_or(0, |g| g.changes.len());

        // Header: Files | Changes.
        let head = Rect::from_min_size(area.min, vec2(area.width(), HEAD_H));
        ui.painter().hline(head.x_range(), head.bottom() - 0.5, egui::Stroke::new(1.0, p.border));
        let seg = Rect::from_min_max(pos2(head.left() + 8.0, head.center().y - kit::SEGMENTED_H / 2.0), pos2(head.right() - 7.0, head.center().y + kit::SEGMENTED_H / 2.0));
        let changes_mode = self.explorers.get(&pid).is_some_and(|e| e.changes_mode);
        let mut options = vec![kit::Seg::new(t("Files"), ""), kit::Seg::new(t("Changes"), "")];
        if changed > 0 {
            options[1] = options[1].clone().count(changed, p.warn);
        }
        #[cfg(debug_assertions)]
        for (r, name) in kit::segment_rects(seg, 2).into_iter().zip(["files:files", "files:changes"]) {
            crate::devtest::record(name, r);
        }
        if let Some(i) = kit::segmented_in(ui, seg, egui::Id::new(("files-mode", pid)), &options, changes_mode as usize)
            && let Some(ex) = self.explorer_mut(pid)
        {
            ex.changes_mode = i == 1;
        }

        // Footer: new file, new folder; collapse all, refresh.
        let foot = Rect::from_min_max(pos2(area.left(), area.bottom() - FOOT_H), area.max);
        ui.painter().hline(foot.x_range(), foot.top() + 0.5, egui::Stroke::new(1.0, p.border));
        // New items go into the selected folder, or beside the selected file, as in VS Code.
        let target_dir = self
            .explorers
            .get(&pid)
            .and_then(|e| e.selected.clone())
            .map(|s| if s.is_dir() { s } else { s.parent().map(Path::to_path_buf).unwrap_or_default() })
            .filter(|d| d.starts_with(&root))
            .unwrap_or_else(|| root.clone());
        let buttons: [(&str, String, Option<Action>, bool); 4] = [
            (icons::PLUS, t("New File…").to_string(), Some(Action::NewFileIn(target_dir.clone())), false),
            (icons::FOLDER_PLUS, t("New Folder…").to_string(), Some(Action::NewFolderIn(target_dir.clone())), false),
            (icons::CARET_UP, t("Collapse All Folders").to_string(), None, true),
            (icons::ARROWS_CLOCKWISE, t("Refresh").to_string(), None, true),
        ];
        let cy = foot.center().y + 0.5;
        let mut left = foot.left() + 8.0;
        let mut right = foot.right() - 8.0 - 22.0 * 2.0 - 2.0;
        for (i, (icon, tip, action, at_right)) in buttons.into_iter().enumerate() {
            let x = if at_right { &mut right } else { &mut left };
            let r = Rect::from_min_size(pos2(*x, cy - 11.0), vec2(22.0, 22.0));
            *x += 24.0;
            let resp = ui
                .scope_builder(egui::UiBuilder::new().max_rect(r), |ui| kit::icon_button_ex(ui, icon, &tip, false, 22.0))
                .inner;
            #[cfg(debug_assertions)]
            crate::devtest::record(format!("files-button:{i}"), resp.rect);
            if resp.clicked() {
                match (i, action) {
                    (_, Some(a)) => self.actions.push(a),
                    (3, None) => {
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

        let body = Rect::from_min_max(pos2(area.left(), head.bottom()), pos2(area.right(), foot.top()));
        if changes_mode {
            ui.scope_builder(egui::UiBuilder::new().max_rect(body), |ui| {
                ui.set_clip_rect(body);
                self.changes_list(ui, pid, &root, body);
            });
            return;
        }
        // The filter.
        let frow = Rect::from_min_size(body.min, vec2(body.width(), 38.0));
        let field = Rect::from_min_size(pos2(frow.left() + 8.0, frow.top() + 6.0), vec2(frow.width() - 16.0, 26.0));
        let mut filter = self.explorers.get(&pid).map(|e| e.filter.clone()).unwrap_or_default();
        let fid = egui::Id::new(("files-filter", pid));
        let fr = ui
            .scope_builder(egui::UiBuilder::new().max_rect(field), |ui| {
                kit::field(ui, fid, &mut filter, Some(icons::FUNNEL), t("Filter files"), field.width(), true)
            })
            .inner;
        #[cfg(debug_assertions)]
        crate::devtest::record("files-filter", fr.rect);
        // Esc is the field's (egui would otherwise take the keyboard away on it first).
        if fr.has_focus() {
            ui.memory_mut(|m| m.set_focus_lock_filter(fid, egui::EventFilter { escape: true, ..Default::default() }));
        }
        if fr.changed() {
            if let Some(ex) = self.explorer_mut(pid) {
                ex.filter = filter.clone();
            }
            self.want_project_files(pid);
        }
        if fr.has_focus() && ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape)) {
            if let Some(ex) = self.explorer_mut(pid) {
                ex.filter.clear();
            }
            fr.surrender_focus();
        }
        let tree = Rect::from_min_max(pos2(body.left(), frow.bottom()), body.max);
        if !filter.trim().is_empty() {
            ui.scope_builder(egui::UiBuilder::new().max_rect(tree), |ui| {
                ui.set_clip_rect(tree);
                self.filtered_files(ui, pid, &root, &filter, tree);
            });
            return;
        }
        self.tree_rows(ui, pid, &root, tree, git);
    }

    /// What git says changed in the project, one file a row: a click shows its changes.
    fn changes_list(&mut self, ui: &mut Ui, pid: tc_core::ProjectId, root: &Path, area: Rect) {
        let p = style::pal(ui);
        let mut changes: Vec<(PathBuf, Change)> = self
            .project(pid)
            .and_then(|pr| pr.git.as_ref())
            .map(|g| g.changes.iter().map(|(k, v)| (k.clone(), *v)).collect())
            .unwrap_or_default();
        changes.sort_by(|a, b| a.0.cmp(&b.0));
        if changes.is_empty() {
            ui.painter().text(
                pos2(area.left() + 14.0, area.top() + 18.0),
                Align2::LEFT_CENTER,
                t("Nothing changed."),
                egui::FontId::proportional(style::SMALL),
                p.text_faint,
            );
            return;
        }
        ui.scope_builder(egui::UiBuilder::new().max_rect(area.shrink2(vec2(4.0, 0.0)).with_min_y(area.top() + 6.0)), |ui| {
            let area = egui::ScrollArea::vertical().id_salt(("changes", pid)).auto_shrink([false, false]);
            kit::show_rows(area, ui, ROW_H, changes.len(), |ui, range| {
                for (path, c) in &changes[range] {
                    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), ROW_H), Sense::click());
                    if resp.hovered() {
                        ui.painter().rect_filled(rect, CornerRadius::same(4), p.faint_row());
                    }
                    let color = change_color(*c, &p);
                    ui.painter().text(pos2(rect.left() + 8.0, rect.center().y), Align2::LEFT_CENTER, c.letter(), style::medium(11.0), color);
                    let rel = path.strip_prefix(root).unwrap_or(path).to_string_lossy().into_owned();
                    let mut job = egui::text::LayoutJob::simple_singleline(rel.clone(), egui::FontId::proportional(12.5), style::tree_text(&p));
                    job.wrap = egui::text::TextWrapping::truncate_at_width((rect.width() - 30.0).max(10.0));
                    let g = ui.painter().layout_job(job);
                    ui.painter().galley(pos2(rect.left() + 24.0, rect.center().y - g.size().y / 2.0), g, p.text);
                    if resp.on_hover_text(kit::tip(t("Review Changes…"), Action::Review)).clicked() {
                        self.actions.push(Action::ReviewFile(path.clone()));
                    }
                }
            });
        });
    }

    /// The project's files whose path matches the filter, best first.
    fn filtered_files(&mut self, ui: &mut Ui, pid: tc_core::ProjectId, root: &Path, filter: &str, area: Rect) {
        let p = style::pal(ui);
        let Some(files) = self.project_files.get(&pid) else {
            ui.painter().text(pos2(area.left() + 14.0, area.top() + 12.0), Align2::LEFT_CENTER, t("Listing the project's files…"), egui::FontId::proportional(style::SMALL), p.text_faint);
            return;
        };
        let mut hits: Vec<(i32, &PathBuf)> = files
            .iter()
            .filter_map(|f| {
                let rel = f.strip_prefix(root).unwrap_or(f).to_string_lossy();
                super::palette::score(filter.trim(), &rel).map(|s| (s, f))
            })
            .collect();
        hits.sort_by_key(|h| std::cmp::Reverse(h.0));
        hits.truncate(200);
        if hits.is_empty() {
            ui.painter().text(pos2(area.left() + 14.0, area.top() + 12.0), Align2::LEFT_CENTER, t("Nothing matches."), egui::FontId::proportional(style::SMALL), p.text_faint);
            return;
        }
        let hits: Vec<PathBuf> = hits.into_iter().map(|(_, f)| f.clone()).collect();
        let mut open = None;
        ui.scope_builder(egui::UiBuilder::new().max_rect(area.shrink2(vec2(4.0, 0.0))), |ui| {
            let sa = egui::ScrollArea::vertical().id_salt(("filtered", pid)).auto_shrink([false, false]);
            kit::show_rows(sa, ui, ROW_H, hits.len(), |ui, range| {
                for path in &hits[range] {
                    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), ROW_H), Sense::click());
                    if resp.hovered() {
                        ui.painter().rect_filled(rect, CornerRadius::same(4), p.faint_row());
                    }
                    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                    let (icon, color) = icons::file_icon(&name, false, false, &p);
                    ui.painter().text(pos2(rect.left() + 8.0 + 7.0, rect.center().y), Align2::CENTER_CENTER, icon, style::icon_font(14.0), color);
                    let folder = path.parent().and_then(|d| d.strip_prefix(root).ok()).map(|d| d.to_string_lossy().into_owned()).unwrap_or_default();
                    let mut job = egui::text::LayoutJob::default();
                    job.append(&name, 0.0, egui::TextFormat::simple(egui::FontId::proportional(13.0), style::tree_text(&p)));
                    if !folder.is_empty() {
                        job.append(&folder, 6.0, egui::TextFormat::simple(egui::FontId::proportional(11.0), p.text_faint));
                    }
                    job.wrap = egui::text::TextWrapping::truncate_at_width((rect.width() - 36.0).max(10.0));
                    let g = ui.painter().layout_job(job);
                    ui.painter().galley(pos2(rect.left() + 28.0, rect.center().y - g.size().y / 2.0), g, p.text);
                    if resp.clicked() {
                        open = Some(path.clone());
                    }
                }
            });
        });
        if let Some(path) = open {
            self.actions.push(Action::OpenFile(path, None));
        }
    }

    /// The tree of the project's files.
    fn tree_rows(&mut self, ui: &mut Ui, pid: tc_core::ProjectId, root: &Path, tree: Rect, git: Option<std::sync::Arc<crate::project::GitInfo>>) {
        let p = style::pal(ui);
        let root = root.to_path_buf();
        let editing = self
            .editors
            .get(&pid)
            .filter(|e| e.visible)
            .and_then(|e| e.file())
            .map(|f| f.path.clone());
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
                                x += 16.0;
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
                                    egui::FontId::proportional(13.0),
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
