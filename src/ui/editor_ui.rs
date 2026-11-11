//! The editor beside the panes (FR-E1–E3): a tab per open file, the file's path, the text with
//! line numbers and syntax colours, a status line, and the choice shown when a file changed on
//! disk under unsaved edits.

use std::path::{Path, PathBuf};

use eframe::egui::{self, Align2, CornerRadius, Rect, RichText, Sense, Stroke, Ui, pos2, vec2};
use tc_core::ProjectId;

use super::{icons, kit, style};
use crate::actions::Action;
use crate::app::App;
use crate::i18n::{t, tf};

pub const TAB_H: f32 = 36.0;
const CRUMB_H: f32 = 28.0;
const STATUS_H: f32 = 26.0;
/// The Files column of the Code drawer, and the narrowest drawer that shows it.
pub const FILES_W: f32 = 212.0;
pub const FILES_MIN_DRAWER: f32 = 520.0;

impl App {
    /// The Code drawer at the right of the panes: the Files column (unless hidden, or the
    /// drawer is too narrow for it), then the editor or document of the open file.
    pub fn code_drawer(&mut self, ui: &mut Ui, project: ProjectId, root: PathBuf) {
        let full = ui.max_rect();
        let files = !self.settings.files_hidden && full.width() >= FILES_MIN_DRAWER;
        let editor = if files {
            let col = Rect::from_min_size(full.min, vec2(FILES_W, full.height()));
            #[cfg(debug_assertions)]
            crate::devtest::record("files-column", col);
            ui.scope_builder(egui::UiBuilder::new().max_rect(col), |ui| {
                ui.set_clip_rect(col);
                self.files_column(ui, col);
            });
            Rect::from_min_max(pos2(col.right(), full.top()), full.max)
        } else {
            full
        };
        ui.scope_builder(egui::UiBuilder::new().max_rect(editor), |ui| {
            ui.set_clip_rect(editor);
            self.editor_panel(ui, project, root, !files);
        });
    }

    pub fn editor_panel(&mut self, ui: &mut Ui, project: ProjectId, root: PathBuf, files_toggle: bool) {
        let p = style::pal(ui);
        let full = ui.max_rect();
        ui.painter().rect_filled(full, CornerRadius::ZERO, p.canvas);

        let tabs = Rect::from_min_size(full.min, vec2(full.width(), TAB_H));
        self.editor_tabs(ui, project, tabs, files_toggle);
        let body = Rect::from_min_max(pos2(full.left(), tabs.bottom()), full.max);
        let empty = self.editors.get(&project).is_none_or(|e| e.files.is_empty());
        if empty {
            ui.scope_builder(egui::UiBuilder::new().max_rect(body), |ui| self.editor_empty(ui, project, &root));
            return;
        }

        // A document lets go of its pictures when its tab is left (made again when shown):
        // once, not on every frame, or its search starts over each time it is shown again.
        if let Some(e) = self.editors.get_mut(&project) {
            let shown_id = egui::Id::new(("editor-shown", project));
            let shown = e.file().map(|f| f.path.clone());
            let before = ui.data(|d| d.get_temp::<Option<PathBuf>>(shown_id)).flatten();
            if before != shown {
                if let Some(d) = before.and_then(|b| e.files.iter_mut().find(|f| f.path == b)).and_then(|f| f.doc.as_mut()) {
                    d.release();
                }
                ui.data_mut(|d| d.insert_temp(shown_id, shown));
            }
        }
        if self.editors.get(&project).and_then(|e| e.file()).is_some_and(|f| f.doc.is_some()) {
            self.doc_panel(ui, project, body);
            return;
        }
        // Path of the file, the problems (⇧⌘M) and the status line at the bottom.
        let crumb = Rect::from_min_size(body.min, vec2(body.width(), CRUMB_H));
        let status = Rect::from_min_max(pos2(body.left(), body.bottom() - STATUS_H), body.max);
        let problems_h = if self.problems_open { (30.0 + 6.0 * 26.0_f32).min(body.height() * 0.4) } else { 0.0 };
        let problems = Rect::from_min_max(pos2(body.left(), status.top() - problems_h), pos2(body.right(), status.top()));
        if self.problems_open {
            ui.scope_builder(egui::UiBuilder::new().max_rect(problems), |ui| {
                ui.set_clip_rect(problems);
                self.problems_panel(ui, project, problems);
            });
        }
        let text_rect = Rect::from_min_max(pos2(body.left(), crumb.bottom()), pos2(body.right(), problems.top()));
        self.editor_crumb(ui, project, &root, crumb);
        self.editor_status(ui, project, status);
        ui.scope_builder(
            egui::UiBuilder::new()
                .max_rect(text_rect)
                .layout(egui::Layout::top_down(egui::Align::Min)),
            |ui| {
                ui.set_clip_rect(text_rect);
                self.editor_text(ui, project);
            },
        );
    }

    fn editor_tabs(&mut self, ui: &mut Ui, project: ProjectId, rect: Rect, files_toggle: bool) {
        let p = style::pal(ui);
        let painter = ui.painter();
        painter.rect_filled(rect, CornerRadius::ZERO, p.chrome);
        painter.hline(rect.x_range(), rect.bottom() - 0.5, Stroke::new(1.0, p.border));

        // The Files column hidden (or no room for it): a button at the left brings it back.
        let mut rect = rect;
        if files_toggle {
            let cell = Rect::from_min_size(rect.min, vec2(36.0, rect.height()));
            ui.painter().vline(cell.right() - 0.5, cell.y_range(), Stroke::new(1.0, p.border));
            let b = Rect::from_center_size(pos2(cell.left() + 6.0 + 13.0, cell.center().y), vec2(26.0, 26.0));
            let r = ui
                .scope_builder(egui::UiBuilder::new().max_rect(b), |ui| {
                    kit::icon_button_ex(ui, icons::SIDEBAR_SIMPLE, t("Show Files"), false, 26.0)
                })
                .inner;
            #[cfg(debug_assertions)]
            crate::devtest::record("show-files", r.rect);
            if r.clicked() {
                self.actions.push(Action::ToggleFiles);
            }
            rect.min.x = cell.right();
        }

        let close = Rect::from_center_size(pos2(rect.right() - 19.0, rect.center().y), vec2(26.0, 26.0));
        let r = ui
            .scope_builder(egui::UiBuilder::new().max_rect(close), |ui| {
                kit::icon_button_ex(ui, icons::X, &kit::tip(t("Close Code"), Action::ToggleEditor), false, 26.0)
            })
            .inner;
        #[cfg(debug_assertions)]
        crate::devtest::record("editor-close", r.rect);
        if r.clicked() {
            self.actions.push(Action::ToggleEditor);
        }

        let Some(editor) = self.editors.get(&project) else { return };
        let files: Vec<(String, bool, PathBuf)> =
            editor.files.iter().map(|f| (f.name(), f.dirty(), f.path.clone())).collect();
        let active = editor.active;
        let strip = Rect::from_min_max(rect.min, pos2(close.left() - 4.0, rect.bottom()));
        let mut select = None;
        let mut close_file = None;
        ui.scope_builder(egui::UiBuilder::new().max_rect(strip), |ui| {
            ui.set_clip_rect(strip);
            egui::ScrollArea::horizontal()
                .id_salt(("editor-tabs", project))
                .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden)
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 0.0;
                        for (i, (name, dirty, path)) in files.iter().enumerate() {
                            let on = i == active;
                            let font = style::medium(13.0);
                            let galley = ui.painter().layout_no_wrap(name.clone(), font, egui::Color32::PLACEHOLDER);
                            let w = 12.0 + 14.0 + 7.0 + galley.size().x + 7.0 + 18.0 + 8.0;
                            let (r, resp) = ui.allocate_exact_size(vec2(w, TAB_H), Sense::click());
                            let close_r = Rect::from_center_size(pos2(r.right() - 8.0 - 9.0, r.center().y), vec2(18.0, 18.0));
                            let close_resp = ui.interact(close_r, egui::Id::new(("tab-close", project, i)), Sense::click());
                            let painter = ui.painter();
                            if on {
                                painter.rect_filled(r, CornerRadius::ZERO, p.canvas);
                                painter.hline(r.x_range(), r.top() + 1.0, Stroke::new(2.0, p.accent));
                            } else if resp.hovered() {
                                painter.rect_filled(r, CornerRadius::ZERO, style::soft(p.hover, 0.6));
                            }
                            painter.vline(r.right() - 0.5, r.y_range(), Stroke::new(1.0, p.border));
                            let (icon, color) = icons::file_icon(name, false, false, &p);
                            painter.text(
                                pos2(r.left() + 12.0 + 7.0, r.center().y),
                                Align2::CENTER_CENTER,
                                icon,
                                style::icon_font(14.0),
                                if on { p.accent_text } else { color },
                            );
                            let text_color = if on { p.text } else { p.text_muted };
                            painter.galley(
                                pos2(r.left() + 33.0, r.center().y - galley.size().y / 2.0),
                                galley,
                                text_color,
                            );
                            // × when hovered or active; ● for unsaved edits otherwise.
                            let show_x = close_resp.hovered() || resp.hovered() || (on && !dirty);
                            if close_resp.hovered() {
                                painter.rect_filled(close_r, CornerRadius::same(4), p.hover);
                            }
                            if *dirty && !close_resp.hovered() {
                                painter.circle_filled(close_r.center(), 3.5, p.text_muted);
                            } else if show_x {
                                painter.text(
                                    close_r.center(),
                                    Align2::CENTER_CENTER,
                                    icons::X,
                                    style::icon_font(11.0),
                                    if close_resp.hovered() { p.text } else { p.text_faint },
                                );
                            }
                            if close_resp.clicked() || resp.middle_clicked() {
                                close_file = Some(i);
                            } else if resp.clicked() {
                                select = Some(i);
                            }
                            resp.on_hover_text(crate::paths::contract_tilde(path));
                        }
                    });
                });
        });
        if let Some(i) = select
            && let Some(e) = self.editors.get_mut(&project)
        {
            e.active = i;
        }
        if let Some(i) = close_file {
            self.actions.push(Action::CloseFile(i));
        }
    }

    fn editor_crumb(&mut self, ui: &mut Ui, project: ProjectId, root: &Path, rect: Rect) {
        let p = style::pal(ui);
        let Some(file) = self.editors.get(&project).and_then(|e| e.file()) else { return };
        paint_crumb(ui, rect, root, &file.path, &p);
        let dark = ui.visuals().dark_mode;
        let painter = ui.painter();
        let mut right = rect.right() - 14.0;
        // What the file is: read-only, or just changed by an agent.
        let mut pills: Vec<(Option<&str>, String, egui::Color32, egui::Color32)> = Vec::new();
        if file.read_only {
            let text = if file.truncated { t("Read-only · first 2 MB") } else { t("Read-only") };
            pills.push((None, text.to_string(), p.text_muted, p.subtle));
        }
        if let Some((agent, at)) = file.changed_by.as_ref().filter(|(_, at)| at.elapsed().as_secs() < 600) {
            let text = crate::i18n::tf("Changed by {} · {} ago", &[agent, &super::status::short_age(*at)]);
            pills.push((Some(icons::SPARKLE), text, p.ok, style::tint(p.ok, dark)));
        }
        for (icon, text, fg, bg) in pills {
            let w = kit::pill_width(painter, icon, &text);
            kit::paint_pill(painter, pos2(right - w, rect.center().y), icon, &text, fg, bg);
            right -= w + 6.0;
        }
    }

    fn editor_status(&mut self, ui: &mut Ui, project: ProjectId, rect: Rect) {
        let p = style::pal(ui);
        ui.painter().hline(rect.x_range(), rect.top() + 0.5, Stroke::new(1.0, p.border));
        let Some(file) = self.editors.get(&project).and_then(|e| e.file()) else { return };
        let (line, col) = file.caret;
        let mut parts = vec![
            tf("Ln {}, Col {}", &[&line.to_string(), &col.to_string()]),
            crate::editor::language_for(&file.path, &file.syntax),
        ];
        // Encoding and line ends only when there is room besides the language server's status.
        if rect.width() > 640.0 {
            parts.push(if file.lossy { t("Not UTF-8").to_string() } else { "UTF-8".to_string() });
            parts.push(if file.crlf { "CRLF".into() } else { "LF".into() });
        }
        let text = parts.join("  ·  ");
        let text_w = ui.painter().layout_no_wrap(text.clone(), egui::FontId::proportional(style::SMALL), p.text_faint).size().x;
        ui.painter().text(
            pos2(rect.left() + 12.0, rect.center().y),
            Align2::LEFT_CENTER,
            text,
            egui::FontId::proportional(style::SMALL),
            p.text_faint,
        );
        let dirty = file.dirty() && !file.read_only;
        // The language server and the file's problems at the right; Save for unsaved edits.
        let path = file.path.clone();
        let inner = Rect::from_min_max(pos2(rect.left() + 12.0 + text_w + 20.0, rect.top() + 2.0), pos2(rect.right() - 12.0, rect.bottom() - 2.0));
        if inner.width() > 40.0 {
            ui.scope_builder(egui::UiBuilder::new().max_rect(inner).layout(egui::Layout::right_to_left(egui::Align::Center)), |ui| {
                ui.set_clip_rect(ui.max_rect().intersect(ui.clip_rect()));
                ui.spacing_mut().item_spacing.x = 10.0;
                self.ide_status(ui, &path);
                if dirty {
                    let save = kit::button_keys(ui, kit::Kind::Primary, Some(icons::FLOPPY_DISK), t("Save"), kit::keys(&Action::SaveFile), true);
                    if save.clicked() {
                        self.actions.push(Action::SaveFile);
                    }
                    ui.label(RichText::new(t("Unsaved")).size(style::SMALL).color(p.warn));
                }
            });
        }
    }

    fn editor_text(&mut self, ui: &mut Ui, project: ProjectId) {
        let p = style::pal(ui);
        let dark = ui.visuals().dark_mode;
        let font_size = (self.settings.font_size - 0.5).max(9.0);
        let ctx = ui.ctx().clone();
        // The IDE's keys first: the text must not see the ones it takes.
        let mut ide_cmds = Vec::new();
        self.ide_keys(&ctx, project, &mut ide_cmds);
        let features = (self.settings.lsp, self.settings.auto_complete);
        let Some(editor) = self.editors.get_mut(&project) else { return };
        let crate::editor::Editor { files, active, find, refocus, .. } = editor;
        let Some(file) = files.get_mut(*active) else { return };
        let text_id = egui::Id::new(("editor-text", file.path.clone()));
        if std::mem::take(refocus) {
            ctx.memory_mut(|m| m.request_focus(text_id));
        }
        // A jump to a place (a definition, a reference): the caret goes there.
        if let Some((a, b)) = file.select.take() {
            kit::select_text(&ctx, text_id, a, b);
            let ix = tc_lsp::pos::LineIndex::new(&file.text);
            let line = ix.line_of(a);
            file.caret = (line + 1, a - ix.line_start(line) + 1);
            file.caret_index = Some(a);
            ctx.memory_mut(|m| m.request_focus(text_id));
        }

        // ----- find bar (⌘F with the keyboard in the editor) ---------------------------------
        let mut find_focused = false;
        if let Some(f) = find.as_mut() {
            // Esc closes the bar only while the keyboard is in the editor (a terminal needs Esc).
            let find_id = egui::Id::new(("editor-find", project));
            let ours = ctx.memory(|m| m.has_focus(text_id) || m.has_focus(find_id));
            let mut close = ours && ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape));
            let mut go: Option<bool> = None;
            let mut enter = false;
            f.refresh(file);
            egui::Frame::NONE
                .fill(p.chrome)
                .inner_margin(egui::Margin { left: 12, right: 8, top: 5, bottom: 5 })
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 6.0;
                        let r = kit::field(ui, find_id, &mut f.query, None, t("Find in file, or :line"), 220.0, true);
                        #[cfg(debug_assertions)]
                        crate::devtest::record("editor-find", r.rect);
                        if f.focus {
                            // Opened again: the old query selected, so typing replaces it.
                            r.request_focus();
                            kit::select_text(ui.ctx(), find_id, 0, f.query.chars().count());
                            f.focus = false;
                        }
                        find_focused = r.has_focus();
                        if r.changed() {
                            // Typing searches from the caret on, for the query just typed (the
                            // matches above were found for the one before).
                            f.refresh(file);
                            go = Some(true);
                            f.current = None;
                        }
                        if r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                            enter = true;
                            go = Some(!ui.input(|i| i.modifiers.shift));
                            r.request_focus();
                            find_focused = true;
                        }
                        // Aa: match case.
                        let (ar, aresp) = ui.allocate_exact_size(vec2(22.0, 22.0), Sense::click());
                        if f.case {
                            ui.painter().rect_filled(ar, CornerRadius::same(style::RADIUS), p.accent_soft);
                        } else if aresp.hovered() {
                            ui.painter().rect_filled(ar, CornerRadius::same(style::RADIUS), p.hover);
                        }
                        ui.painter().text(ar.center(), Align2::CENTER_CENTER, "Aa", style::semibold(11.0), if f.case { p.accent_text } else { p.text_muted });
                        if aresp.on_hover_text(t("Match Case")).clicked() {
                            f.case = !f.case;
                            f.refresh(file);
                            go = Some(true);
                            f.current = None;
                        }
                        let info = if let Some(line) = f.goto_line() {
                            (tf("Enter: go to line {}", &[&(line + 1).to_string()]), p.text_muted)
                        } else if f.query.is_empty() {
                            (String::new(), p.text_muted)
                        } else if f.matches.is_empty() {
                            (t("No results").to_string(), p.err)
                        } else {
                            let n = f.matches.len();
                            let more = if n >= crate::editor::MAX_MATCHES { "+" } else { "" };
                            match f.current {
                                Some(c) => (tf("{} of {}", &[&(c + 1).to_string(), &format!("{n}{more}")]), p.text_muted),
                                None => (tf("{} results", &[&format!("{n}{more}")]), p.text_muted),
                            }
                        };
                        ui.add_space(2.0);
                        ui.label(RichText::new(info.0).size(style::SMALL).color(info.1));
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.spacing_mut().item_spacing.x = 4.0;
                            if kit::icon_button_ex(ui, icons::X, t("Close"), false, 22.0).clicked() {
                                close = true;
                            }
                            if kit::icon_button_ex(ui, icons::ARROW_DOWN, t("Next"), false, 22.0).clicked() {
                                go = Some(true);
                            }
                            if kit::icon_button_ex(ui, icons::ARROW_UP, t("Previous"), false, 22.0).clicked() {
                                go = Some(false);
                            }
                        });
                    });
                });
            let full = ui.min_rect();
            ui.painter().hline(full.x_range(), full.bottom() - 0.5, Stroke::new(1.0, p.border));
            if let Some(line) = f.goto_line() {
                // `:N` + Enter: the caret to line N, and back to the text.
                if enter {
                    file.goto_line = Some(line + 1);
                    let at = line_start_char(&file.text, line);
                    kit::select_text(&ctx, text_id, at, at);
                    close = true;
                }
            } else if let Some(forward) = go {
                let caret = file.caret_index.unwrap_or(0);
                if let Some(m) = f.step(forward, caret) {
                    file.goto_line = Some(m.line + 1);
                    kit::select_text(&ctx, text_id, m.chars.0, m.chars.1);
                }
            }
            if close {
                *find = None;
                *refocus = true;
                // Nothing has the keyboard until the text takes it next frame.
                ctx.memory_mut(|m| m.surrender_focus(find_id));
                ctx.request_repaint();
            }
        }
        if find_focused {
            self.editor_seen = true;
        }
        let Some(editor) = self.editors.get_mut(&project) else { return };
        let crate::editor::Editor { files, active, find, .. } = editor;
        let Some(file) = files.get_mut(*active) else { return };

        if file.conflict {
            let mut reload = false;
            let mut force = false;
            let who = file.changed_by.as_ref().map(|(a, _)| a.clone());
            let title = match &who {
                Some(agent) => tf("{} changed this file while you were editing it.", &[agent]),
                None => t("This file changed on disk while you were editing it.").to_string(),
            };
            egui::Frame::NONE
                .fill(style::soft(p.warn, 0.12))
                .inner_margin(egui::Margin::symmetric(14, 8))
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 10.0;
                        kit::icon(ui, icons::WARNING, 16.0, p.warn);
                        let mut job = egui::text::LayoutJob::default();
                        job.append(&title, 0.0, egui::TextFormat::simple(style::medium(12.5), p.warn));
                        job.append(t("Nothing was overwritten."), 5.0, egui::TextFormat::simple(egui::FontId::proportional(12.5), p.text_muted));
                        job.wrap.max_width = ui.available_width();
                        ui.label(job);
                    });
                });
            let line = ui.min_rect();
            ui.painter().hline(line.x_range(), ui.cursor().top() - 0.5, Stroke::new(1.0, style::soft(p.warn, 0.3)));
            egui::Frame::NONE.inner_margin(egui::Margin::symmetric(12, 6)).show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 6.0;
                    let label = if file.compare { t("Hide Differences") } else { t("Compare") };
                    if kit::button_ex(ui, kit::Kind::Secondary, Some(icons::COLUMNS), label, true).clicked() {
                        file.compare = !file.compare;
                    }
                    let use_theirs = match &who {
                        Some(agent) => tf("Use {}’s Version", &[agent]),
                        None => t("Reload from Disk").to_string(),
                    };
                    if kit::button_ex(ui, kit::Kind::Secondary, None, &use_theirs, true).clicked() {
                        reload = true;
                    }
                    if kit::button_ex(ui, kit::Kind::Danger, None, t("Keep Mine and Save"), true).clicked() {
                        force = true;
                    }
                });
            });
            let line = ui.cursor().top();
            ui.painter().hline(ui.max_rect().x_range(), line - 0.5, Stroke::new(1.0, p.border));
            if reload {
                self.actions.push(Action::ReloadFile);
            }
            if force {
                self.actions.push(Action::ForceSave);
            }
        }

        if file.compare {
            if file.compare_diff.is_none() {
                file.compare_diff = Some(crate::editor::compare_diff(file));
            }
            let diff = file.compare_diff.as_deref().unwrap_or_default();
            // Two columns: the disk (what the agent wrote) and yours.
            let disk_label = match &file.changed_by {
                Some((agent, at)) => tf("On disk — {}, {} ago", &[agent, &super::status::age(*at)]),
                None => t("On disk").to_string(),
            };
            let area = ui.available_rect_before_wrap();
            let half = (area.width() / 2.0).floor();
            let mono = egui::FontId::monospace(font_size - 1.0);
            let left: Vec<(char, &str)> = diff.lines().filter(|l| !l.starts_with("+ ")).map(|l| (l.chars().next().unwrap_or(' '), l.get(2..).unwrap_or(""))).collect();
            let right: Vec<(char, &str)> = diff.lines().filter(|l| !l.starts_with("- ")).map(|l| (l.chars().next().unwrap_or(' '), l.get(2..).unwrap_or(""))).collect();
            egui::ScrollArea::vertical().id_salt(("compare", project)).auto_shrink([false, false]).show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 0.0;
                let (head, _) = ui.allocate_exact_size(vec2(area.width(), 24.0), Sense::hover());
                ui.painter().hline(head.x_range(), head.bottom() - 0.5, Stroke::new(1.0, p.border));
                ui.painter().vline(head.left() + half, head.y_range(), Stroke::new(1.0, p.border));
                ui.painter().text(pos2(head.left() + 10.0, head.center().y), Align2::LEFT_CENTER, disk_label, egui::FontId::proportional(style::SMALL), p.text_faint);
                ui.painter().text(pos2(head.left() + half + 10.0, head.center().y), Align2::LEFT_CENTER, t("Yours — not saved"), egui::FontId::proportional(style::SMALL), p.text_faint);
                for i in 0..left.len().max(right.len()) {
                    let (row, _) = ui.allocate_exact_size(vec2(area.width(), 19.0), Sense::hover());
                    if !ui.is_rect_visible(row) {
                        continue;
                    }
                    for (side, x0, sign_c, bg_c) in [(left.get(i), row.left(), p.ok, p.ok), (right.get(i), row.left() + half, p.err, p.err)] {
                        let Some((mark, text)) = side else { continue };
                        let changed = *mark == '-' || *mark == '+';
                        let cell = Rect::from_min_size(pos2(x0, row.top()), vec2(half, row.height()));
                        let (sc, bg) = if changed { (if *mark == '-' { sign_c } else { bg_c }, style::soft(if *mark == '-' { p.ok } else { p.err }, 0.10)) } else { (p.text_faint, egui::Color32::TRANSPARENT) };
                        ui.painter().rect_filled(cell, CornerRadius::ZERO, bg);
                        let sign = match mark { '-' => "+", '+' => "−", _ => "" };
                        ui.painter().text(pos2(cell.left() + 10.0, row.center().y), Align2::CENTER_CENTER, sign, mono.clone(), sc);
                        let painter = ui.painter().with_clip_rect(cell);
                        painter.text(pos2(cell.left() + 20.0, row.center().y), Align2::LEFT_CENTER, *text, mono.clone(), if changed { p.text } else { p.text_muted });
                    }
                    ui.painter().vline(row.left() + half, row.y_range(), Stroke::new(1.0, p.border));
                }
            });
            return;
        }

        let plain = p.text;
        let font = egui::FontId::monospace(font_size);
        let row_h = ui.fonts_mut(|f| f.row_height(&font));
        let char_w = ui.fonts_mut(|f| f.glyph_width(&font, '0'));
        let digits = file.lines.max(1).to_string().len().max(3) as f32;
        let gutter = (digits * char_w + 28.0).round();
        let goto = file.goto_line.take();
        let mut area = egui::ScrollArea::both()
            .id_salt(("editor", project, file.path.clone()))
            .auto_shrink([false, false]);
        if let Some(line) = goto {
            area = area.vertical_scroll_offset((line.saturating_sub(5) as f32) * row_h);
        }
        let read_only = file.read_only;
        let path = file.path.clone();
        let mut editor_seen = false;
        let triggers: Vec<char> = self
            .lsp
            .server_for(&path)
            .and_then(|s| s.caps.completion_provider.as_ref())
            .and_then(|c| c.trigger_characters.as_ref())
            .map(|t| t.iter().filter_map(|s| s.chars().next()).collect())
            .unwrap_or_default();
        let diags = self.lsp.diagnostics.get(&path);
        let ide = &mut self.lsp.ide;
        let mut geom = None;
        area.show(ui, |ui| {
            // The caret line's band goes under the text: reserve its place before the text.
            let band = ui.painter().add(egui::Shape::Noop);
            let marks = ui.painter().add(egui::Shape::Noop);
            let syntax = file.syntax.clone();
            let cache = &mut file.highlight;
            let mut layouter = |ui: &Ui, buf: &dyn egui::TextBuffer, _wrap: f32| {
                let mut job = crate::editor::cached_layout(cache, buf.as_str(), &syntax, dark, font.clone(), plain);
                // No wrapping: one row per line, so the line numbers line up.
                job.wrap.max_width = f32::INFINITY;
                ui.fonts_mut(|f| f.layout_job(job))
            };
            // A read-only file is shown through a `&str`, which no key can change: it still
            // takes the keyboard, so it can be selected, copied, searched and navigated.
            let mut shown: &str;
            let buffer: &mut dyn egui::TextBuffer = if read_only {
                shown = file.text.as_str();
                &mut shown
            } else {
                &mut file.text
            };
            let output = egui::TextEdit::multiline(buffer)
                .id(egui::Id::new(("editor-text", path.clone())))
                .code_editor()
                .font(font.clone())
                .desired_width(f32::INFINITY)
                .desired_rows(40)
                .lock_focus(true)
                // With a frame given, egui takes the margin from the frame.
                .frame(egui::Frame::NONE.inner_margin(egui::Margin {
                    left: gutter.min(127.0) as i8,
                    right: 12,
                    top: 6,
                    bottom: 40,
                }))
                .layouter(&mut layouter)
                .show(ui);
            let resp = &output.response.response;
            if resp.has_focus() {
                editor_seen = true;
            }
            #[cfg(debug_assertions)]
            crate::devtest::record("editor-text", resp.rect);
            if resp.changed() && !read_only {
                file.rev += 1;
                file.lines = crate::editor::count_lines(&file.text);
                file.compare_diff = None;
            }
            // Caret position (worked out only when it moved).
            let caret_row = output.cursor_range.map(|c| {
                let index = c.primary.index.0;
                if file.caret_index != Some(index) {
                    let lc = output.galley.layout_from_cursor(c.primary);
                    file.caret = (lc.row + 1, lc.column.0 + 1);
                    file.caret_index = Some(index);
                }
                file.caret.0 - 1
            });

            // Line numbers in the left margin, for the rows in view only.
            let clip = ui.clip_rect();
            let galley = &output.galley;
            let origin = output.galley_pos;
            let first = galley.rows.partition_point(|r| origin.y + r.rect().bottom() < clip.top());
            let numbers_right = resp.rect.left() + gutter - 14.0;
            let number_font = egui::FontId::monospace(font_size - 1.0);
            for (i, row) in galley.rows.iter().enumerate().skip(first) {
                let y = origin.y + row.pos.y;
                if y > clip.bottom() {
                    break;
                }
                let current = caret_row == Some(i);
                ui.painter().text(
                    pos2(numbers_right, y + row.rect().height() / 2.0),
                    Align2::RIGHT_CENTER,
                    (i + 1).to_string(),
                    number_font.clone(),
                    if current { p.text_muted } else { style::soft(p.text_faint, 0.75) },
                );
            }
            if let Some(row) = caret_row.and_then(|r| galley.rows.get(r))
                && resp.has_focus()
            {
                let y = origin.y + row.pos.y;
                let band_rect = Rect::from_min_max(
                    pos2(clip.left(), y),
                    pos2(clip.right().max(resp.rect.right()), y + row.rect().height()),
                );
                ui.painter().set(
                    band,
                    egui::Shape::rect_filled(band_rect, CornerRadius::ZERO, style::soft(p.text, if dark { 0.045 } else { 0.05 })),
                );
            }
            // Find results in view, the current one stronger.
            if let Some(f) = find.as_ref().filter(|f| !f.matches.is_empty()) {
                let last = galley.rows.partition_point(|r| origin.y + r.rect().top() <= clip.bottom());
                let from = f.matches.partition_point(|m| m.line < first);
                let mut shapes = Vec::new();
                for (i, m) in f.matches.iter().enumerate().skip(from) {
                    if m.line >= last {
                        break;
                    }
                    let Some(row) = galley.rows.get(m.line) else { break };
                    let x0 = origin.x + row.pos.x + row.x_offset(egui::epaint::text::CharIndex(m.col.0));
                    let x1 = origin.x + row.pos.x + row.x_offset(egui::epaint::text::CharIndex(m.col.1));
                    let y = origin.y + row.pos.y;
                    let r = Rect::from_min_max(pos2(x0, y), pos2(x1.max(x0 + 2.0), y + row.rect().height()));
                    let current = f.current == Some(i);
                    shapes.push(egui::Shape::rect_filled(
                        r.expand2(vec2(1.0, 0.0)),
                        CornerRadius::same(2),
                        if current { style::soft(p.warn, 0.55) } else { style::soft(p.warn, 0.22) },
                    ));
                }
                ui.painter().set(marks, egui::Shape::Vec(shapes));
            }
            // A thin line between the numbers and the text.
            let sep_x = resp.rect.left() + gutter - 6.0;
            ui.painter().vline(sep_x, clip.y_range(), Stroke::new(1.0, style::soft(p.border, 0.6)));
            // Diagnostics, ⌘-click, hover and what typing asks of the language server.
            geom = Some(super::ide_ui::overlay(ui, &output, file, diags, ide, &p, features, &triggers, &mut ide_cmds));
        });
        if editor_seen {
            self.editor_seen = true;
        }
        if let Some(g) = geom {
            self.ide_popups(&ctx, &path, &g, &mut ide_cmds);
        }
        for cmd in ide_cmds {
            self.ide_cmd(&ctx, cmd);
        }
    }

    /// No file open: how to open one, and the files closed lately.
    fn editor_empty(&mut self, ui: &mut Ui, project: ProjectId, root: &Path) {
        let p = style::pal(ui);
        let recent: Vec<PathBuf> = self
            .editors
            .get(&project)
            .map(|e| e.recent_closed().take(5).cloned().collect())
            .unwrap_or_default();
        ui.add_space((ui.available_height() * 0.22).min(140.0));
        ui.vertical_centered(|ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            let (r, _) = ui.allocate_exact_size(vec2(52.0, 52.0), Sense::hover());
            ui.painter().circle_filled(r.center(), 26.0, p.subtle);
            ui.painter().text(r.center(), Align2::CENTER_CENTER, icons::CODE, style::icon_font(24.0), p.text_muted);
            ui.add_space(10.0);
            ui.label(RichText::new(t("No file open")).font(style::semibold(15.0)).color(p.text));
            ui.add_space(4.0);
            ui.label(
                RichText::new(tf(
                    "Pick a file in Files, press {}, or {}-click a path in a terminal.",
                    &[kit::keys(&Action::QuickOpen).unwrap_or(""), if cfg!(target_os = "macos") { "⌘" } else { "Ctrl" }],
                ))
                .size(style::SMALL)
                .color(p.text_muted),
            );
            ui.add_space(12.0);
            if kit::button_keys(ui, kit::Kind::Secondary, None, t("Open File…"), kit::keys(&Action::QuickOpen), true).clicked() {
                self.actions.push(Action::QuickOpen);
            }
            if !recent.is_empty() {
                ui.add_space(18.0);
                let g = kit::section_galley(ui.painter(), t("Recently closed"), p.text_faint);
                let (r, _) = ui.allocate_exact_size(vec2(260.0, g.size().y + 6.0), Sense::hover());
                ui.painter().galley(r.min, g, p.text_faint);
                for path in recent {
                    let rel = path.strip_prefix(root).unwrap_or(&path).to_string_lossy().into_owned();
                    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                    let (icon, color) = icons::file_icon(&name, false, false, &p);
                    let (row, resp) = ui.allocate_exact_size(vec2(260.0, 24.0), Sense::click());
                    if resp.hovered() {
                        ui.painter().rect_filled(row, CornerRadius::same(4), p.faint_row());
                        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                    }
                    ui.painter().text(pos2(row.left() + 4.0 + 7.0, row.center().y), Align2::CENTER_CENTER, icon, style::icon_font(14.0), color);
                    ui.painter().text(pos2(row.left() + 4.0 + 14.0 + 6.0, row.center().y), Align2::LEFT_CENTER, rel, egui::FontId::proportional(12.0), p.text_muted);
                    if resp.clicked() {
                        self.actions.push(Action::OpenFile(path, None));
                    }
                }
            }
        });
    }
}

/// The character index where line `line` (0-based) starts.
fn line_start_char(text: &str, line: usize) -> usize {
    if line == 0 {
        return 0;
    }
    let mut seen = 0;
    for (i, c) in text.chars().enumerate() {
        if c == '\n' {
            seen += 1;
            if seen == line {
                return i + 1;
            }
        }
    }
    text.chars().count()
}

/// A file's path from the project folder, its parts separated by a caret, the last brighter.
pub fn paint_crumb(ui: &Ui, rect: Rect, root: &Path, path: &Path, p: &style::Palette) {
    let rel = path.strip_prefix(root).unwrap_or(path);
    let parts: Vec<String> = rel.components().map(|c| c.as_os_str().to_string_lossy().into_owned()).collect();
    let painter = ui.painter();
    let font = egui::FontId::proportional(12.0);
    let mut x = rect.left() + 14.0;
    let cy = rect.center().y;
    for (i, part) in parts.iter().enumerate() {
        let last = i + 1 == parts.len();
        if i > 0 {
            let g = painter.layout_no_wrap(icons::CARET_RIGHT.to_string(), style::icon_font(10.0), p.text_faint);
            painter.galley(pos2(x, cy - g.size().y / 2.0), g.clone(), p.text_faint);
            x += g.size().x + 6.0;
        }
        let color = if last { p.text_muted } else { p.text_faint };
        let g = painter.layout_no_wrap(part.clone(), font.clone(), color);
        let w = g.size().x;
        painter.galley(pos2(x, cy - g.size().y / 2.0), g, color);
        x += w + 6.0;
    }
}
