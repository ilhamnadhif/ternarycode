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

const ROW_H: f32 = 25.0;
/// The Files | Changes header and the buttons at the bottom of the column.
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

/// The Files filter's matches: what they were found for (project, filter, file list) and the
/// best of them.
type FilterHits = Option<((tc_core::ProjectId, String, u64), std::rc::Rc<Vec<PathBuf>>)>;

thread_local! {
    static FILTER_HITS: std::cell::RefCell<FilterHits> = const { std::cell::RefCell::new(None) };
}

impl App {
    /// The Files column of the Code drawer: Files | Changes at the top, a filter, the tree (or
    /// the files that match the filter, or what git says changed), and the buttons that make
    /// files and folders at the bottom.
    pub fn files_column(&mut self, ui: &mut Ui, area: Rect) {
        let p = style::pal(ui);
        let Some(pid) = self.active else { return };
        ui.painter().vline(area.right() - 0.5, area.y_range(), egui::Stroke::new(1.0, p.border));
        let root = self.project(pid).map(|x| x.path.clone()).unwrap_or_default();
        let git = self.project(pid).and_then(|pr| pr.git.clone());
        let changed = git.as_ref().map_or(0, |g| g.changes.len());
        // (Files | Changes is the drawer's head.)
        let head = Rect::from_min_size(area.min, vec2(area.width(), 0.0));

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
        // At the left how many files changed; the buttons at the right.
        if changed > 0 {
            let painter = ui.painter();
            let g = painter.layout_no_wrap(crate::i18n::tf("{} changed", &[&changed.to_string()]), style::mono(10.5), p.text_faint);
            let mut x = foot.left() + 12.0 + g.size().x;
            painter.galley(pos2(foot.left() + 12.0, cy - g.size().y / 2.0), g, p.text_faint);
            // How many of them an agent changed (the one that changed most of them).
            let touched: Vec<tc_core::agent::AgentKind> = self
                .agent_touched
                .get(&pid)
                .map(|t| t.iter().filter(|(path, _)| git.as_ref().is_some_and(|g| g.changes.contains_key(*path))).map(|(_, k)| *k).collect())
                .unwrap_or_default();
            let by = touched.iter().copied().max_by_key(|k| touched.iter().filter(|x| *x == k).count());
            let room = foot.right() - 6.0 - 20.0 * 4.0 - 2.0 * 3.0 - 8.0;
            if let Some(k) = by {
                let n = touched.iter().filter(|x| **x == k).count();
                let text = crate::i18n::tf("{} by {}", &[&n.to_string(), k.label()]);
                let tg = painter.layout_no_wrap(text, style::mono(10.5), p.text_faint);
                if x + 18.0 + 15.0 + tg.size().x < room {
                    painter.text(pos2(x + 7.0, cy), Align2::CENTER_CENTER, "·", egui::FontId::proportional(11.0), p.text_ghost);
                    x += 14.0;
                    if super::agent_logo::has(k) {
                        super::agent_logo::paint(painter, Rect::from_min_size(pos2(x, cy - 5.0), vec2(10.0, 10.0)), k, p.text, false);
                        x += 10.0 + 5.0;
                    }
                    painter.galley(pos2(x, cy - tg.size().y / 2.0), tg, p.text_faint);
                }
            }
        }
        let mut left = foot.right() - 6.0 - 20.0 * 4.0 - 2.0 * 3.0;
        let mut right = foot.right() - 6.0 - 20.0 * 2.0 - 2.0;
        for (i, (icon, tip, action, at_right)) in buttons.into_iter().enumerate() {
            let x = if at_right { &mut right } else { &mut left };
            let r = Rect::from_min_size(pos2(*x, cy - 10.0), vec2(20.0, 20.0));
            *x += 22.0;
            let resp = ui
                .scope_builder(egui::UiBuilder::new().max_rect(r), |ui| kit::icon_button_ex(ui, icon, &tip, false, 20.0))
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
        // The filter.
        let frow = Rect::from_min_size(body.min, vec2(body.width(), 40.0));
        let field = Rect::from_min_size(pos2(frow.left() + 8.0, frow.top() + 8.0), vec2(frow.width() - 16.0, 26.0));
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
            ui.memory_mut(|m| m.set_focus_lock_filter(fid, egui::EventFilter { escape: true, horizontal_arrows: true, vertical_arrows: true, ..Default::default() }));
        }
        if fr.changed() {
            if let Some(ex) = self.explorer_mut(pid) {
                ex.filter = filter.clone();
            }
            self.want_project_files(pid);
        }
        if fr.has_focus() && !self.overlay_open() && ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape)) {
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

    /// The project's files whose path matches the filter, best first.
    fn filtered_files(&mut self, ui: &mut Ui, pid: tc_core::ProjectId, root: &Path, filter: &str, area: Rect) {
        let p = style::pal(ui);
        let Some(files) = self.project_files.get(&pid) else {
            ui.painter().text(pos2(area.left() + 14.0, area.top() + 12.0), Align2::LEFT_CENTER, t("Listing the project's files…"), egui::FontId::proportional(style::SMALL), p.text_faint);
            return;
        };
        // Scored again only when the filter or the file list changes (up to 20 000 paths), not
        // on every frame the column is drawn.
        let key = (pid, filter.trim().to_string(), self.files_rev);
        let kept = FILTER_HITS.with(|c| c.borrow().as_ref().filter(|(k, _)| *k == key).map(|(_, hits)| std::rc::Rc::clone(hits)));
        let hits = match kept {
            Some(hits) => hits,
            None => {
                let mut scored: Vec<(i32, &PathBuf)> = files
                    .iter()
                    .filter_map(|f| {
                        let rel = f.strip_prefix(root).unwrap_or(f).to_string_lossy();
                        super::palette::score(filter.trim(), &rel).map(|s| (s, f))
                    })
                    .collect();
                scored.sort_by_key(|h| std::cmp::Reverse(h.0));
                scored.truncate(200);
                let hits = std::rc::Rc::new(scored.into_iter().map(|(_, f)| f.clone()).collect::<Vec<PathBuf>>());
                FILTER_HITS.with(|c| *c.borrow_mut() = Some((key, std::rc::Rc::clone(&hits))));
                hits
            }
        };
        if hits.is_empty() {
            ui.painter().text(pos2(area.left() + 14.0, area.top() + 12.0), Align2::LEFT_CENTER, t("Nothing matches."), egui::FontId::proportional(style::SMALL), p.text_faint);
            return;
        }
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
                    // The list's paths are the project's (`project_files`): relative to its folder.
                    let folder = path.parent().map(|d| d.strip_prefix(root).unwrap_or(d).to_string_lossy().into_owned()).unwrap_or_default();
                    let mut job = egui::text::LayoutJob::default();
                    job.append(&name, 0.0, egui::TextFormat::simple(egui::FontId::proportional(13.0), style::tree_text(&p)));
                    if !folder.is_empty() {
                        job.append(&folder, 6.0, egui::TextFormat::simple(egui::FontId::proportional(11.0), p.text_faint));
                    }
                    job.wrap = egui::text::TextWrapping::truncate_at_width((rect.width() - 36.0).max(10.0));
                    let g = ui.painter().layout_job(job);
                    ui.painter().galley(pos2(rect.left() + 28.0, rect.center().y - g.size().y / 2.0), g, p.text);
                    if resp.clicked() {
                        open = Some(root.join(path));
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
        let touched = self.agent_touched.get(&pid).cloned().unwrap_or_default();
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
                let new_file = kit::empty_state_at(ui, tree, icons::FOLDER_SIMPLE, t("This folder is empty"), t("Create a file, or let an agent."), Some((Some(icons::PLUS), t("New file"), None)));
                if new_file.is_some_and(|r| r.clicked()) {
                    menu_actions.push(Action::NewFileIn(root.clone()));
                }
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
                                    painter.rect(rect, CornerRadius::same(6), p.accent_tint, egui::Stroke::new(1.0, style::soft(p.accent, 0.28)), egui::StrokeKind::Inside);
                                } else if resp.hovered() {
                                    painter.rect_filled(rect, CornerRadius::same(6), p.hover);
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
                                // Changed files in full ink; their letter at the right says how.
                                let mut name_color = if is_sel || change.is_some() { p.text } else { style::tree_text(&p) };
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
                                    painter.text(pos2(right - 6.0, rect.center().y), Align2::CENTER_CENTER, c.letter(), style::mono_semibold(10.0), change_color(c, &p));
                                    right -= 12.0 + 5.0;
                                    // Changed by an agent while it worked: its logo.
                                    if let Some(k) = touched.get(&entry.path).copied().filter(|k| super::agent_logo::has(*k)) {
                                        super::agent_logo::paint(&painter, Rect::from_min_size(pos2(right - 11.0, rect.center().y - 5.5), vec2(11.0, 11.0)), k, p.text, false);
                                        right -= 11.0 + 5.0;
                                    }
                                } else if dir_changed {
                                    painter.circle_filled(pos2(right - 6.0, rect.center().y), 2.5, style::soft(p.accent_text, 0.8));
                                    right -= 12.0 + 5.0;
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
                    kit::menu_body(ui);
                    new_items_menu(ui, &root, &mut menu_actions);
                    kit::menu_sep(ui);
                    if kit::menu_item(ui, Some(icons::TERMINAL_WINDOW), t("Open in Terminal"), None, false).clicked() {
                        menu_actions.push(Action::NewTabAt(root.clone()));
                        ui.close();
                    }
                    if kit::menu_item(ui, Some(icons::FOLDER_OPEN), reveal_label(), None, false).clicked() {
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
        let p = style::pal(ui);
        let dark = ui.visuals().dark_mode;
        let box_rect = Rect::from_min_max(pos2(rect.left(), rect.top() + 1.0), pos2(rect.right(), rect.bottom() - 1.0));
        let fill = if dark { p.inset } else { egui::Color32::WHITE };
        ui.painter().rect_stroke(
            box_rect.expand(1.5),
            CornerRadius::same(6),
            egui::Stroke::new(2.0, p.accent_tint),
            egui::StrokeKind::Inside,
        );
        ui.painter().rect(
            box_rect,
            CornerRadius::same(5),
            fill,
            egui::Stroke::new(1.0, p.accent),
            egui::StrokeKind::Inside,
        );
        let inner = box_rect.shrink2(vec2(6.0, 1.0));
        let mut child = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(inner)
                .layout(egui::Layout::left_to_right(egui::Align::Center)),
        );
        let r = child.add_sized(
            inner.size(),
            egui::TextEdit::singleline(&mut edit.text)
                .id(id)
                .frame(egui::Frame::NONE)
                .margin(egui::Margin::ZERO)
                .font(egui::FontId::proportional(12.5))
                .vertical_align(egui::Align::Center),
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
    if kit::menu_item(ui, Some(icons::PLUS), t("New File…"), None, false).clicked() {
        out.push(Action::NewFileIn(dir.to_path_buf()));
        ui.close();
    }
    if kit::menu_item(ui, Some(icons::FOLDER_PLUS), t("New Folder…"), None, false).clicked() {
        out.push(Action::NewFolderIn(dir.to_path_buf()));
        ui.close();
    }
}

fn tree_menu(ui: &mut Ui, entry: &Entry, root: &Path, out: &mut Vec<Action>) {
    kit::menu_body(ui);
    let mut item = |ui: &mut Ui, icon: Option<&str>, label: &str, action: Action, danger: bool| {
        if kit::menu_item(ui, icon, label, None, danger).clicked() {
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
        item(ui, Some(icons::PLUS), t("New File…"), Action::NewFileIn(dir.clone()), false);
        item(ui, Some(icons::FOLDER_PLUS), t("New Folder…"), Action::NewFolderIn(dir.clone()), false);
        item(ui, Some(icons::TERMINAL_WINDOW), t("Open in Terminal"), Action::NewTabAt(dir), false);
    } else {
        item(ui, Some(icons::CODE), t("Open in Editor"), Action::OpenFile(entry.path.clone(), None), false);
        if crate::platform::browser_file(&entry.path) {
            item(ui, Some(icons::GLOBE), t("Open in Browser"), Action::OpenInBrowser(entry.path.clone()), false);
            item(ui, Some(icons::ARROW_SQUARE_OUT), t("Open in Default Browser"), Action::OpenInDefaultBrowser(entry.path.clone()), false);
        }
        item(ui, Some(icons::PLUS), t("New File…"), Action::NewFileIn(dir.clone()), false);
        item(ui, Some(icons::FOLDER_PLUS), t("New Folder…"), Action::NewFolderIn(dir), false);
    }
    item(ui, Some(icons::TERMINAL), t("Insert Path in Terminal"), Action::InsertPath(entry.path.clone()), false);
    kit::menu_sep(ui);
    item(ui, Some(icons::PENCIL_SIMPLE), t("Rename…"), Action::RenamePath(entry.path.clone()), false);
    if !entry.is_dir {
        item(ui, Some(icons::COPY), t("Duplicate"), Action::DuplicatePath(entry.path.clone()), false);
    }
    item(ui, Some(icons::TRASH), t("Move to Trash"), Action::TrashPath(entry.path.clone()), true);
    kit::menu_sep(ui);
    if kit::menu_item(ui, Some(icons::COPY), t("Copy Path"), None, false).clicked() {
        ui.ctx().copy_text(entry.path.to_string_lossy().into_owned());
        ui.close();
    }
    if kit::menu_item(ui, None, t("Copy Relative Path"), None, false).clicked() {
        let rel = entry.path.strip_prefix(root).unwrap_or(&entry.path);
        ui.ctx().copy_text(rel.to_string_lossy().into_owned());
        ui.close();
    }
    item(ui, Some(icons::FOLDER_OPEN), reveal_label(), Action::RevealPath(entry.path.clone()), false);
}

/// The colour of a changed file's name and letter.
pub fn change_color(c: Change, p: &style::Palette) -> egui::Color32 {
    match c {
        // Modified in the accent: amber means "needs you".
        Change::Modified => p.accent_text,
        Change::Added | Change::Untracked => p.ok,
        Change::Deleted | Change::Conflict => p.err,
        Change::Renamed => p.info,
    }
}
