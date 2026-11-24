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

pub const TAB_H: f32 = 40.0;
const CRUMB_H: f32 = 30.0;
const STATUS_H: f32 = 28.0;
/// Default width, drag bounds, and minimum drawer width for the resizable Files column.
pub const FILES_W: f32 = 230.0;
pub const FILES_MIN_W: f32 = 150.0;
pub const FILES_MAX_W: f32 = 520.0;
const EDITOR_MIN_W: f32 = 240.0;
pub const FILES_MIN_DRAWER: f32 = 520.0;

impl App {
    /// The Code drawer at the right of the panes: the resizable Files column (unless hidden, or
    /// the drawer is too narrow for it), a draggable divider between the tree and the editor, then
    /// the editor or document of the open file.
    pub fn code_drawer(&mut self, ui: &mut Ui, project: ProjectId, root: PathBuf) {
        let full = ui.max_rect();
        let files = !self.settings.files_hidden && full.width() >= FILES_MIN_DRAWER;
        let (col_opt, max_tree, editor) = if files {
            let max_tree = (full.width() - EDITOR_MIN_W).clamp(FILES_MIN_W, FILES_MAX_W);
            let tree_w = self.settings.files_width.clamp(FILES_MIN_W, max_tree).round();
            let col = Rect::from_min_size(full.min, vec2(tree_w, full.height()));
            #[cfg(debug_assertions)]
            crate::devtest::record("files-column", col);
            ui.scope_builder(egui::UiBuilder::new().max_rect(col), |ui| {
                ui.set_clip_rect(col);
                self.files_column(ui, col);
            });
            (Some(col), max_tree, Rect::from_min_max(pos2(col.right(), full.top()), full.max))
        } else {
            (None, FILES_MAX_W, full)
        };
        ui.scope_builder(egui::UiBuilder::new().max_rect(editor), |ui| {
            ui.set_clip_rect(editor);
            self.editor_panel(ui, project, root, !files);
        });
        if let Some(col) = col_opt {
            self.files_resize_edge(ui, project, col, full, max_tree);
        }
    }

    /// Draggable vertical divider between the file tree and the editor/document view.
    /// Double-clicking resets the file-tree width to its default (`FILES_W`).
    fn files_resize_edge(&mut self, ui: &mut Ui, project: ProjectId, col: Rect, full: Rect, max_tree: f32) {
        let p = style::pal(ui);
        let mid = col.right();
        let grab = Rect::from_min_max(
            pos2(mid - super::focus::DIVIDER_GRAB, full.top()),
            pos2(mid + super::focus::DIVIDER_GRAB, full.bottom()),
        );
        let r = ui.interact(grab, egui::Id::new(("files-edge", project)), Sense::click_and_drag());
        #[cfg(debug_assertions)]
        crate::devtest::record("files-edge", grab);
        if r.hovered() || r.dragged() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
            ui.painter().rect_filled(
                Rect::from_center_size(pos2(mid, full.center().y), vec2(2.0, (full.height() - 16.0).max(0.0))),
                CornerRadius::same(1),
                p.accent,
            );
        }
        if r.dragged()
            && let Some(pos) = r.interact_pointer_pos()
        {
            let next = (pos.x - full.left()).clamp(FILES_MIN_W, max_tree).round();
            if (next - self.settings.files_width).abs() > 0.5 {
                self.settings.files_width = next;
                ui.ctx().request_repaint();
            }
        }
        if r.double_clicked() {
            if (self.settings.files_width - FILES_W).abs() > 0.5 {
                self.settings.files_width = FILES_W;
                self.settings_changed();
                ui.ctx().request_repaint();
            }
        } else if r.drag_stopped() {
            self.settings_changed();
        }
    }

    pub fn editor_panel(&mut self, ui: &mut Ui, project: ProjectId, root: PathBuf, files_toggle: bool) {
        let full = ui.max_rect();
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
        // Path of the file and the status line at the bottom (Problems lives in the unified Secondary Drawer).
        let crumb = Rect::from_min_size(body.min, vec2(body.width(), CRUMB_H));
        let status = Rect::from_min_max(pos2(body.left(), body.bottom() - STATUS_H), body.max);
        let text_rect = Rect::from_min_max(pos2(body.left(), crumb.bottom()), pos2(body.right(), status.top()));
        self.want_line_marks(project);
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

    /// The open files as tabs (the one in front raised), and at the left, when the Files
    /// column is hidden, the button that brings it back.
    fn editor_tabs(&mut self, ui: &mut Ui, project: ProjectId, rect: Rect, files_toggle: bool) {
        let p = style::pal(ui);
        // The Files column hidden (or no room for it): a button at the left brings it back.
        let mut rect = rect;
        if files_toggle {
            let b = Rect::from_center_size(pos2(rect.left() + 8.0 + 11.0, rect.center().y), vec2(22.0, 22.0));
            let r = ui
                .scope_builder(egui::UiBuilder::new().max_rect(b), |ui| {
                    kit::icon_button_ex(ui, icons::SIDEBAR_SIMPLE, t("Show Files"), false, 22.0)
                })
                .inner;
            #[cfg(debug_assertions)]
            crate::devtest::record("show-files", r.rect);
            if r.clicked() {
                self.actions.push(Action::ToggleFiles);
            }
            rect.min.x = b.right() + 4.0;
        }

        let git = self.project(project).and_then(|x| x.git.clone());
        let Some(editor) = self.editors.get(&project) else { return };
        // Each file's git letter (M, A, D…) after its name.
        let files: Vec<(String, bool, PathBuf, Option<crate::project::Change>)> =
            editor.files.iter().map(|f| (f.name(), f.dirty(), f.path.clone(), git.as_ref().and_then(|g| g.changes.get(&f.path).copied()))).collect();
        let active = editor.active;
        let strip = Rect::from_min_max(pos2(rect.left() + 8.0, rect.top()), pos2(rect.right() - 8.0, rect.bottom()));
        let mut select = None;
        let mut close_file = None;
        let seen_id = egui::Id::new(("editor-tabs-active", project));
        let scroll_to_active = ui.data(|d| d.get_temp::<(usize, usize)>(seen_id)) != Some((active, files.len()));
        ui.data_mut(|d| d.insert_temp(seen_id, (active, files.len())));
        ui.scope_builder(egui::UiBuilder::new().max_rect(strip), |ui| {
            ui.set_clip_rect(strip.expand2(vec2(0.0, 1.0)));
            ui.style_mut().always_scroll_the_only_direction = true;
            ui.spacing_mut().scroll.fade.strength = 0.0;
            let out = egui::ScrollArea::horizontal()
                .id_salt(("editor-tabs", project))
                .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden)
                .show(ui, |ui| {
                    ui.horizontal_centered(|ui| {
                        ui.spacing_mut().item_spacing.x = 2.0;
                        for (i, (name, dirty, path, changed)) in files.iter().enumerate() {
                            let on = i == active;
                            // The one in front raised: its icon, name (Small Strong), letter, ×.
                            let font = if on { style::medium(style::SMALL) } else { egui::FontId::proportional(style::SMALL) };
                            let galley = ui.painter().layout_no_wrap(name.clone(), font, egui::Color32::PLACEHOLDER);
                            let letter = changed.map(|c| ui.painter().layout_no_wrap(c.letter().to_string(), style::mono_semibold(style::DATA), super::explorer_ui::change_color(c, &p)));
                            let mark_w = letter.as_ref().map_or(0.0, |g| g.size().x + 6.0);
                            let w = 9.0 + 13.0 + 6.0 + galley.size().x + mark_w + 6.0 + 12.0 + 9.0;
                            let (r, resp) = ui.allocate_exact_size(vec2(w, 28.0), Sense::click());
                            if on && scroll_to_active {
                                resp.scroll_to_me(None);
                            }
                            let close_r = Rect::from_center_size(pos2((r.right() - 9.0 - 6.0).round(), r.center().y.round()), vec2(16.0, 16.0));
                            let close_resp = ui.interact(close_r, egui::Id::new(("tab-close", project, i)), Sense::click());
                            let tab_hot = resp.hovered() || close_resp.hovered();
                            if tab_hot {
                                ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                            }
                            let painter = ui.painter();
                            if on {
                                painter.rect(r, CornerRadius::same(style::RADIUS_ROW), p.surface, Stroke::new(1.0, p.border), egui::StrokeKind::Inside);
                            } else if tab_hot {
                                painter.rect_filled(r, CornerRadius::same(style::RADIUS_ROW), p.hover);
                            }
                            let (icon, _) = icons::file_icon(name, false, false, &p);
                            painter.text(pos2(r.left() + 9.0 + 6.5, r.center().y), Align2::CENTER_CENTER, icon, style::icon_font(13.0), if on { p.text_muted } else { p.text_faint });
                            let text_color = if on || tab_hot { p.text } else { p.text_muted };
                            let gw = galley.size().x;
                            painter.galley(pos2(r.left() + 9.0 + 13.0 + 6.0, r.center().y - galley.size().y / 2.0), galley, text_color);
                            if let Some(g) = letter {
                                let c = changed.map(|c| super::explorer_ui::change_color(c, &p)).unwrap_or(p.text_faint);
                                painter.galley(pos2(r.left() + 9.0 + 13.0 + 6.0 + gw + 6.0, r.center().y - g.size().y / 2.0), g, c);
                            }
                            // × when hovered or in front; ● for unsaved edits otherwise.
                            let show_x = tab_hot || (on && !dirty);
                            if close_resp.hovered() {
                                painter.rect_filled(close_r, CornerRadius::same(4), p.subtle);
                            }
                            if *dirty && !close_resp.hovered() {
                                painter.circle_filled(close_r.center(), 3.0, p.text_muted);
                            } else if show_x {
                                kit::paint_close_x(painter, close_r.center(), 3.25, if close_resp.hovered() { p.text } else { p.text_faint });
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
            let shown = out.inner_rect;
            let offset = out.state.offset.x;
            let fade_w = 20.0_f32.min(shown.width() / 2.0);
            if offset > 0.5 {
                let r = Rect::from_min_max(shown.min, pos2(shown.left() + fade_w, shown.bottom()));
                kit::fade(ui.painter(), r, p.canvas, false);
            }
            if offset + shown.width() < out.content_size.x - 0.5 {
                let r = Rect::from_min_max(pos2(shown.right() - fade_w, shown.top()), shown.max);
                kit::fade(ui.painter(), r, p.canvas, true);
            }
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

    /// The open file's lines that differ from the agent's start (its run's snapshot) or else the
    /// last commit, worked out off the UI thread when the file on disk, the base or its git
    /// state changes: for the marks beside the line numbers.
    fn want_line_marks(&mut self, project: ProjectId) {
        let Some(pr) = self.project(project) else { return };
        let Some(git) = pr.git.clone() else { return };
        let folder = pr.path.clone();
        let base = pr
            .current_pane()
            .and_then(|id| self.panes.get(&id))
            .and_then(|x| x.run_snapshot.as_ref())
            .map_or_else(|| "HEAD".to_string(), |(tree, ..)| tree.clone());
        let Some(f) = self.editors.get_mut(&project).and_then(|e| e.file_mut()) else { return };
        if f.doc.is_some() {
            return;
        }
        let change = git.changes.get(&f.path).copied();
        let key = format!("{:?}|{base}|{change:?}", f.mtime);
        if f.marks_key.as_deref() == Some(key.as_str()) {
            return;
        }
        f.marks_key = Some(key.clone());
        let path = f.path.clone();
        // A file git does not know yet: every line is new.
        if matches!(change, Some(crate::project::Change::Untracked | crate::project::Change::Added)) && base == "HEAD" {
            f.marks = vec![tc_git::LineMark { line: 1, count: f.lines.max(1) as u32, kind: tc_git::MarkKind::Added }];
            return;
        }
        let tx = self.app_tx.clone();
        let ctx = self.ctx.clone();
        self.rt.spawn_blocking(move || {
            let marks = (|| {
                let top: PathBuf = tc_git::repo_root(&folder)?;
                let rel = path.strip_prefix(&top).ok()?.to_string_lossy().replace('\\', "/");
                tc_git::line_marks(&top, &base, &rel).ok()
            })()
            .unwrap_or_default();
            let _ = tx.send(crate::app::AppMsg::LineMarks(path, key, marks));
            ctx.request_repaint();
        });
    }

    fn editor_crumb(&mut self, ui: &mut Ui, project: ProjectId, root: &Path, rect: Rect) {
        let p = style::pal(ui);
        // The file's symbols, asked of its server (a moment after the text last changed).
        if let Some(f) = self.editors.get(&project).and_then(|e| e.file())
            && self.lsp.server_for(&f.path).is_some_and(|s| s.state == crate::lsp::ServerState::Ready)
            && self.lsp.outline.get(&f.path).is_none_or(|(rev, _)| *rev != f.rev)
        {
            let asked = self.lsp.outline_asked.get(&f.path).copied();
            match asked {
                Some((rev, _)) if rev == f.rev => {}
                Some((_, at)) if at.elapsed() < std::time::Duration::from_millis(800) => {
                    ui.ctx().request_repaint_after(std::time::Duration::from_millis(800) - at.elapsed());
                }
                _ => {
                    let ctx = ui.ctx().clone();
                    self.ide_cmd(&ctx, crate::lsp::IdeCmd::Outline);
                }
            }
        }
        let Some(file) = self.editors.get(&project).and_then(|e| e.file()) else { return };
        // The symbol the caret is in (the innermost function, type or module).
        let line = file.caret.0.saturating_sub(1) as u32;
        let symbol = self.lsp.outline.get(&file.path).and_then(|(_, items)| {
            use tc_lsp::types::SymbolKind as K;
            items
                .iter()
                .filter(|s| s.start <= line && line <= s.end)
                .filter(|s| !matches!(s.kind, K::VARIABLE | K::FIELD | K::CONSTANT | K::PROPERTY | K::ENUM_MEMBER | K::TYPE_PARAMETER | K::KEY | K::NULL | K::BOOLEAN | K::NUMBER | K::STRING | K::ARRAY | K::EVENT | K::OPERATOR))
                .min_by_key(|s| s.end - s.start)
                .map(|s| (s.name.clone(), matches!(s.kind, K::FUNCTION | K::METHOD | K::CONSTRUCTOR)))
        });
        let painter = ui.painter();
        painter.hline(rect.x_range(), rect.bottom() - 0.5, Stroke::new(1.0, p.border));
        // Right-side badges first so breadcrumb + symbol clip cleanly before them.
        let mut right = rect.right() - 12.0;
        if let Some((agent, at)) = file.changed_by.as_ref().filter(|(_, at)| at.elapsed().as_secs() < 600) {
            // "Changed by Claude Code · 2m", on the accent's tint.
            let text = crate::i18n::tf("Changed by {} · {}", &[agent, &super::status::short_age(*at)]);
            let kind = tc_core::agent::AgentKind::ALL.iter().copied().find(|k| k.label() == agent).filter(|k| super::agent_logo::has(*k));
            let g = painter.layout_no_wrap(text, egui::FontId::proportional(style::SMALL), p.accent_text);
            let w = 8.0 + kind.map_or(0.0, |_| 16.0 + 6.0) + g.size().x + 8.0;
            let r = Rect::from_min_size(pos2(right - w, rect.center().y - 11.0), vec2(w, 22.0));
            painter.rect_filled(r, CornerRadius::same(style::RADIUS), p.accent_tint);
            let mut x = r.left() + 8.0;
            if let Some(k) = kind {
                kit::paint_agent_badge(painter, Rect::from_min_size(pos2(x, r.center().y - 8.0), vec2(16.0, 16.0)), k, &p);
                x += 16.0 + 6.0;
            }
            painter.galley(pos2(x, r.center().y - g.size().y / 2.0), g, p.accent_text);
            right -= w + 6.0;
        }
        if file.read_only {
            let text = if file.truncated { t("Read-only · first 2 MB") } else { t("Read-only") };
            let w = kit::pill_width(painter, None, text);
            kit::paint_pill(painter, pos2(right - w, rect.center().y), None, text, p.text_muted, p.subtle);
            right -= w + 6.0;
        }
        let open_in_browser = crate::platform::browser_file(&file.path) && !file.use_virtual_viewer();
        let browser_path = open_in_browser.then(|| file.path.clone());
        if let Some(bp) = browser_path {
            let label = t("Open in Browser");
            let w = kit::button_width(ui, Some(icons::GLOBE), label, None, true);
            if right - w > rect.left() + 90.0 {
                let r = Rect::from_min_size(pos2(right - w, rect.center().y - 11.0), vec2(w, 22.0));
                let clicked = ui
                    .scope_builder(egui::UiBuilder::new().max_rect(r), |ui| {
                        kit::button_ex(ui, kit::Kind::Ghost, Some(icons::GLOBE), label, true).clicked()
                    })
                    .inner;
                #[cfg(debug_assertions)]
                crate::devtest::record("editor-open-browser", r);
                if clicked {
                    self.actions.push(Action::OpenInBrowser(bp));
                }
                right -= w + 6.0;
            }
        }
        let Some(file) = self.editors.get(&project).and_then(|e| e.file()) else { return };
        let crumb_clip = Rect::from_min_max(rect.min, pos2((right - 6.0).max(rect.left()), rect.max.y));
        let crumb_end = ui
            .scope_builder(egui::UiBuilder::new().max_rect(crumb_clip), |ui| {
                ui.set_clip_rect(crumb_clip.intersect(ui.clip_rect()));
                paint_crumb(ui, rect, root, &file.path, &p)
            })
            .inner;
        if let Some((name, func)) = symbol {
            let clip_painter = ui.painter_at(crumb_clip);
            let x = crumb_end;
            let g = clip_painter.layout_no_wrap(icons::CARET_RIGHT.to_string(), style::icon_font(9.0), p.text_ghost);
            let gw = g.size().x;
            if x + gw + 28.0 < crumb_clip.right() {
                clip_painter.galley(pos2(x, rect.center().y - g.size().y / 2.0), g, p.text_ghost);
                let x = x + gw + 6.0;
                clip_painter.text(pos2(x + 6.0, rect.center().y), Align2::CENTER_CENTER, if func { icons::FUNCTION } else { icons::CODE }, style::icon_font(12.0), p.info);
                let mut job = egui::text::LayoutJob::simple_singleline(name, style::mono(style::DATA), p.text_muted);
                job.wrap = egui::text::TextWrapping::truncate_at_width((crumb_clip.right() - (x + 18.0)).max(20.0));
                let g = clip_painter.layout_job(job);
                clip_painter.galley(pos2(x + 18.0, rect.center().y - g.size().y / 2.0), g, p.text_muted);
            }
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
        let text = parts.join(" · ");
        let left_painter = ui.painter_at(Rect::from_min_max(rect.min, pos2((rect.right() - 12.0).max(rect.left()), rect.max.y)));
        let text_w = left_painter.layout_no_wrap(text.clone(), style::mono(style::DATA), p.text_faint).size().x;
        left_painter.text(
            pos2(rect.left() + 12.0, rect.center().y),
            Align2::LEFT_CENTER,
            text,
            style::mono(style::DATA),
            p.text_faint,
        );
        let dirty = file.dirty() && !file.read_only;
        // The language server and the file's problems at the right; Save for unsaved edits.
        let path = file.path.clone();
        let min_x = (rect.left() + 12.0 + text_w + 20.0).min(rect.right() - 12.0);
        let inner = Rect::from_min_max(pos2(min_x, rect.top() + 2.0), pos2((rect.right() - 12.0).max(min_x), rect.bottom() - 2.0));
        if inner.width() > 40.0 {
            ui.scope_builder(egui::UiBuilder::new().max_rect(inner).layout(egui::Layout::right_to_left(egui::Align::Center)), |ui| {
                ui.set_clip_rect(ui.max_rect().intersect(ui.clip_rect()));
                ui.spacing_mut().item_spacing.x = 14.0;
                if dirty {
                    let save = kit::button_keys(ui, kit::Kind::Primary, Some(icons::FLOPPY_DISK), t("Save"), kit::keys(&Action::SaveFile), true);
                    if save.clicked() {
                        self.actions.push(Action::SaveFile);
                    }
                    ui.label(RichText::new(t("Unsaved")).size(style::SMALL).color(p.warn));
                }
                self.ide_status(ui, &path);
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
        let overlay = self.overlay_open();
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
            file.caret_rev = file.rev;
            ctx.memory_mut(|m| m.request_focus(text_id));
        }

        // ----- find bar (⌘F with the keyboard in the editor) ---------------------------------
        let mut find_focused = false;
        if let Some(f) = find.as_mut() {
            // Esc closes the bar only while the keyboard is in the editor (a terminal needs Esc).
            let find_id = egui::Id::new(("editor-find", project));
            let ours = ctx.memory(|m| m.has_focus(text_id) || m.has_focus(find_id) || m.had_focus_last_frame(find_id));
            let mut close = ours && !overlay && ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape));
            let mut go: Option<bool> = None;
            let mut enter = false;
            f.refresh(file);
            // A card in the menus' colour over the text: the query, which match, Aa, ↑ ↓ ×.
            egui::Frame::NONE.inner_margin(egui::Margin { left: 8, right: 8, top: 8, bottom: 4 }).show(ui, |ui| {
                let card = egui::Frame::NONE
                    .fill(p.menu)
                    .stroke(Stroke::new(1.0, p.border_strong))
                    .corner_radius(CornerRadius::same(9))
                    .shadow(ui.visuals().popup_shadow)
                    .inner_margin(egui::Margin { left: 10, right: 4, top: 0, bottom: 0 });
                card.show(ui, |ui| {
                    ui.set_height(36.0);
                    ui.horizontal_centered(|ui| {
                        ui.spacing_mut().item_spacing.x = 6.0;
                        kit::icon(ui, icons::MAGNIFYING_GLASS, 14.0, p.text_faint);
                        let field_w = (ui.available_width() - 190.0).clamp(80.0, 180.0);
                        let r = ui.add_sized(
                            vec2(field_w, 20.0),
                            egui::TextEdit::singleline(&mut f.query)
                                .id(find_id)
                                .char_limit(1_024)
                                .frame(egui::Frame::NONE)
                                .margin(egui::Margin::ZERO)
                                .font(egui::FontId::proportional(style::BODY))
                                .hint_text(RichText::new(t("Find in file, or :line")).color(p.text_faint))
                                .vertical_align(egui::Align::Center),
                        );
                        #[cfg(debug_assertions)]
                        crate::devtest::record("editor-find", r.rect);
                        if f.focus {
                            // Opened again: the old query selected, so typing replaces it.
                            r.request_focus();
                            kit::select_text(ui.ctx(), find_id, 0, f.query.chars().count());
                            f.focus = false;
                        }
                        find_focused = r.has_focus();
                        if find_focused {
                            ui.memory_mut(|m| m.set_focus_lock_filter(find_id, egui::EventFilter { escape: true, horizontal_arrows: true, vertical_arrows: true, ..Default::default() }));
                        }
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
                        if !info.0.is_empty() {
                            ui.label(RichText::new(info.0).font(style::mono(style::DATA)).color(info.1));
                        }
                        // Aa: match case.
                        let (ar, aresp) = ui.allocate_exact_size(vec2(24.0, 22.0), Sense::click());
                        if f.case {
                            ui.painter().rect_filled(ar, CornerRadius::same(5), p.accent_tint);
                        } else if aresp.hovered() {
                            ui.painter().rect_filled(ar, CornerRadius::same(5), p.hover);
                        }
                        ui.painter().text(ar.center(), Align2::CENTER_CENTER, "Aa", style::medium(style::SMALL), if f.case { p.accent_text } else { p.text_muted });
                        if aresp.on_hover_text(t("Match Case")).clicked() {
                            f.case = !f.case;
                            f.refresh(file);
                            go = Some(true);
                            f.current = None;
                        }
                        kit::vdivider(ui, 16.0);
                        ui.spacing_mut().item_spacing.x = 2.0;
                        if kit::icon_button_ex(ui, icons::CARET_UP, t("Previous"), false, 24.0).clicked() {
                            go = Some(false);
                        }
                        if kit::icon_button_ex(ui, icons::CARET_DOWN, t("Next"), false, 24.0).clicked() {
                            go = Some(true);
                        }
                        if kit::icon_button_ex(ui, icons::X, t("Close"), false, 24.0).clicked() {
                            close = true;
                        }
                    });
                });
            });
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
            // A notice over the text: who changed it, and the ways on.
            let who = file.changed_by.as_ref().map(|(a, _)| a.clone());
            let title = match &who {
                Some(agent) => tf("{} changed this file while you were editing it", &[agent]),
                None => t("This file changed on disk while you were editing it").to_string(),
            };
            let theirs = match &who {
                Some(agent) => tf("Use {}’s version", &[agent]),
                None => t("Reload from disk").to_string(),
            };
            let buttons = [
                kit::NoticeButton { label: t("Keep mine"), kind: kit::Kind::Ghost, keys: None },
                kit::NoticeButton { label: if file.compare { t("Hide differences") } else { t("Compare") }, kind: kit::Kind::Secondary, keys: None },
                kit::NoticeButton { label: &theirs, kind: kit::Kind::Primary, keys: None },
            ];
            let mut clicked = None;
            egui::Frame::NONE.inner_margin(egui::Margin { left: 8, right: 8, top: 8, bottom: 6 }).show(ui, |ui| {
                let w = ui.available_width();
                clicked = kit::notice_ex(ui, w, kit::Tone::Input, Some(icons::WARNING), &title, Some(t("Nothing was overwritten.")), &buttons).0;
            });
            match clicked {
                Some(0) => self.actions.push(Action::ForceSave),
                Some(1) => file.compare = !file.compare,
                Some(2) => self.actions.push(Action::ReloadFile),
                _ => {}
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
            type CompareRow<'a> = (Option<(char, &'a str)>, Option<(char, &'a str)>);
            let mut rows: Vec<CompareRow<'_>> = Vec::new();
            let mut minus: Vec<&str> = Vec::new();
            let mut plus: Vec<&str> = Vec::new();
            fn flush<'a>(rows: &mut Vec<CompareRow<'a>>, minus: &mut Vec<&'a str>, plus: &mut Vec<&'a str>) {
                let n = minus.len().max(plus.len());
                for i in 0..n {
                    rows.push((minus.get(i).map(|s| ('-', *s)), plus.get(i).map(|s| ('+', *s))));
                }
                minus.clear();
                plus.clear();
            }
            for line in diff.lines() {
                if let Some(rest) = line.strip_prefix("- ") {
                    minus.push(rest);
                } else if let Some(rest) = line.strip_prefix("+ ") {
                    plus.push(rest);
                } else {
                    flush(&mut rows, &mut minus, &mut plus);
                    let rest = line.strip_prefix("  ").unwrap_or(line);
                    rows.push((Some((' ', rest)), Some((' ', rest))));
                }
            }
            flush(&mut rows, &mut minus, &mut plus);
            egui::ScrollArea::vertical().id_salt(("compare", project)).auto_shrink([false, false]).show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 0.0;
                let (head, _) = ui.allocate_exact_size(vec2(area.width(), 24.0), Sense::hover());
                ui.painter().hline(head.x_range(), head.bottom() - 0.5, Stroke::new(1.0, p.border));
                ui.painter().vline(head.left() + half, head.y_range(), Stroke::new(1.0, p.border));
                ui.painter().text(pos2(head.left() + 10.0, head.center().y), Align2::LEFT_CENTER, disk_label, egui::FontId::proportional(style::SMALL), p.text_faint);
                ui.painter().text(pos2(head.left() + half + 10.0, head.center().y), Align2::LEFT_CENTER, t("Yours — not saved"), egui::FontId::proportional(style::SMALL), p.text_faint);
                for (left, right) in &rows {
                    let (row, _) = ui.allocate_exact_size(vec2(area.width(), 19.0), Sense::hover());
                    if !ui.is_rect_visible(row) {
                        continue;
                    }
                    for (side, x0) in [(left.as_ref(), row.left()), (right.as_ref(), row.left() + half)] {
                        let Some((mark, text)) = side else { continue };
                        let changed = *mark == '-' || *mark == '+';
                        let cell = Rect::from_min_size(pos2(x0, row.top()), vec2(half, row.height()));
                        let (sc, bg) = match mark {
                            '-' => (p.err, style::soft(p.err, 0.10)),
                            '+' => (p.ok, style::soft(p.ok, 0.10)),
                            _ => (p.text_faint, egui::Color32::TRANSPARENT),
                        };
                        ui.painter().rect_filled(cell, CornerRadius::ZERO, bg);
                        let sign = match mark { '-' => "−", '+' => "+", _ => "" };
                        ui.painter().text(pos2(cell.left() + 10.0, row.center().y), Align2::CENTER_CENTER, sign, mono.clone(), sc);
                        let painter = ui.painter().with_clip_rect(cell);
                        let shown = clip_chars(text, 2_000);
                        painter.text(pos2(cell.left() + 20.0, row.center().y), Align2::LEFT_CENTER, shown.as_ref(), mono.clone(), if changed { p.text } else { p.text_muted });
                    }
                    ui.painter().vline(row.left() + half, row.y_range(), Stroke::new(1.0, p.border));
                }
            });
            return;
        }

        let plain = p.text;
        let font = egui::FontId::monospace(font_size);
        let row_h = ui.fonts_mut(|f| f.row_height(&font));
        // Lines 20 high at the usual size (the text centred in them).
        let line_h = (font_size * 1.6).round().max(row_h.ceil());
        let char_w = ui.fonts_mut(|f| f.glyph_width(&font, '0'));
        let digits = file.lines.max(1).to_string().len().max(3) as f32;
        // 8, the numbers, 10, the change mark (3), 10, the code.
        let gutter = (digits * char_w + 8.0 + 10.0 + 3.0 + 10.0).round().min(127.0);
        let goto = file.goto_line.take();

        // Large files (> 350 KB) or files with giant minified lines (> 8 KB, e.g. Draw.io HTML
        // exports or minified JS bundles) use the virtualized read-only viewer so egui never
        // allocates > 256 MB GPU vertex buffers (which panics wgpu / force-closes the app).
        if file.use_virtual_viewer() {
            let mut open_browser = None;
            let virtual_seen = paint_virtual_text(
                ui,
                project,
                file,
                find.as_ref(),
                goto,
                font,
                font_size,
                line_h,
                gutter,
                dark,
                &p,
                &mut open_browser,
            );
            if virtual_seen {
                self.editor_seen = true;
            }
            if let Some(bp) = open_browser {
                self.actions.push(Action::OpenInBrowser(bp));
            }
            return;
        }

        let mut area = egui::ScrollArea::both()
            .id_salt(("editor", project, file.path.clone()))
            .auto_shrink([false, false]);
        if let Some(line) = goto {
            area = area.vertical_scroll_offset((line.saturating_sub(5) as f32) * line_h);
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
        let want_escape = find.is_some()
            || ide.completion.as_ref().is_some_and(|c| c.path == path && !c.shown.is_empty())
            || ide.actions.as_ref().is_some_and(|a| a.path == path)
            || ide.signature.as_ref().is_some_and(|s| s.path == path)
            || ide.hover.as_ref().is_some_and(|h| h.path == path);
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
                for section in &mut job.sections {
                    section.format.line_height = Some(line_h);
                    section.format.valign = egui::Align::Center;
                }
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
                .char_limit(crate::editor::MAX_TEXTEDIT_BYTES)
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
                if want_escape {
                    ui.memory_mut(|m| m.set_focus_lock_filter(text_id, egui::EventFilter { tab: true, horizontal_arrows: true, vertical_arrows: true, escape: true }));
                }
            }
            #[cfg(debug_assertions)]
            crate::devtest::record("editor-text", resp.rect.intersect(ui.clip_rect()));
            if resp.changed() && !read_only {
                file.rev += 1;
                file.lines = crate::editor::count_lines(&file.text);
                if file.text.len() > crate::editor::MAX_HIGHLIGHT_LINE_LEN {
                    file.max_line_bytes = crate::editor::max_line_len(&file.text);
                } else {
                    file.max_line_bytes = file.text.len();
                }
                file.refresh_dirty();
                file.compare_diff = None;
            }
            // Caret position (worked out only when it moved or the text changed).
            let caret_row = output.cursor_range.map(|c| {
                let index = c.primary.index.0;
                if file.caret_index != Some(index) || file.caret_rev != file.rev {
                    let lc = output.galley.layout_from_cursor(c.primary);
                    file.caret = (lc.row + 1, lc.column.0 + 1);
                    file.caret_index = Some(index);
                    file.caret_rev = file.rev;
                }
                file.caret.0 - 1
            });

            // Line numbers in the left margin, for the rows in view only.
            let clip = ui.clip_rect();
            let galley = &output.galley;
            let origin = output.galley_pos;
            let first = galley.rows.partition_point(|r| origin.y + r.rect().bottom() < clip.top());
            let numbers_right = resp.rect.left() + gutter - 23.0;
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
                    if current { p.text_muted } else { p.text_faint },
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
                    egui::Shape::rect_filled(band_rect, CornerRadius::ZERO, p.hover),
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
            // Beside the numbers: what changed since the agent started (or the last commit) —
            // a green bar for new lines, the accent for changed ones, a red notch where lines
            // were removed.
            let mark_x = resp.rect.left() + gutter - 11.5;
            let last_row = galley.rows.partition_point(|r| origin.y + r.rect().top() <= clip.bottom());
            for m in &file.marks {
                let (l0, l1) = (m.line.saturating_sub(1) as usize, (m.line.saturating_sub(1) + m.count.max(1)) as usize);
                // Out of view (a removal's notch at the edge of it still shows).
                if (l1 <= first || l0 >= last_row.max(first + 1)) && !(m.kind == tc_git::MarkKind::Deleted && l0 >= first && l0 <= last_row) {
                    continue;
                }
                match m.kind {
                    tc_git::MarkKind::Deleted => {
                        // At the top of the line after the removed ones (or under the last line).
                        let y = match galley.rows.get(l0) {
                            Some(row) => origin.y + row.pos.y,
                            None => galley.rows.last().map_or(origin.y, |r| origin.y + r.pos.y + r.rect().height()),
                        };
                        let pts = vec![pos2(mark_x - 1.5, y - 4.0), pos2(mark_x + 3.5, y), pos2(mark_x - 1.5, y + 4.0)];
                        ui.painter().add(egui::Shape::convex_polygon(pts, p.err, Stroke::NONE));
                    }
                    kind => {
                        let (Some(a), Some(b)) = (galley.rows.get(l0), galley.rows.get(l1.min(galley.rows.len()).saturating_sub(1))) else { continue };
                        let r = Rect::from_min_max(pos2(mark_x - 1.5, origin.y + a.pos.y), pos2(mark_x + 1.5, origin.y + b.pos.y + b.rect().height()));
                        let c = if kind == tc_git::MarkKind::Added { p.ok } else { p.accent };
                        ui.painter().rect_filled(r, CornerRadius::ZERO, c);
                    }
                }
            }
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

    /// No file open: high-craft wide-editor overview with Quick Open, changed git files, and
    /// recently closed files (Linear / Zed restraint — no decorative vanity cards).
    fn editor_empty(&mut self, ui: &mut Ui, project: ProjectId, root: &Path) {
        let p = style::pal(ui);
        let dark = ui.visuals().dark_mode;
        let recent: Vec<PathBuf> = self
            .editors
            .get(&project)
            .map(|e| e.recent_closed().take(6).cloned().collect())
            .unwrap_or_default();
        let mut changed: Vec<(PathBuf, crate::project::Change)> = self
            .project(project)
            .and_then(|pr| pr.git.as_ref())
            .map(|g| g.changes.iter().map(|(k, v)| (k.clone(), *v)).collect())
            .unwrap_or_default();
        changed.sort_by(|a, b| a.0.cmp(&b.0));
        changed.truncate(8);

        let area = ui.max_rect();
        let col_w = (area.width() - 32.0).clamp(160.0, 680.0);
        let top_pad = (area.height() * 0.10).clamp(20.0, 72.0);

        egui::ScrollArea::vertical()
            .id_salt(("editor-empty", project))
            .auto_shrink([false, false])
            .show(ui, |ui| {
                ui.add_space(top_pad);
                ui.horizontal(|ui| {
                    let side = ((ui.available_width() - col_w) / 2.0).max(16.0);
                    ui.add_space(side);
                    ui.vertical(|ui| {
                        ui.set_width(col_w);
                        ui.spacing_mut().item_spacing.y = 0.0;
                        ui.label(RichText::new(t("Code Editor")).font(style::semibold(15.0)).color(p.text));
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
                        ui.horizontal_wrapped(|ui| {
                            ui.spacing_mut().item_spacing = vec2(8.0, 6.0);
                            if kit::button_keys(ui, kit::Kind::Primary, Some(icons::MAGNIFYING_GLASS), t("Open File…"), kit::keys(&Action::QuickOpen), false).clicked() {
                                self.actions.push(Action::QuickOpen);
                            }
                            if !changed.is_empty()
                                && kit::button_keys(ui, kit::Kind::Secondary, Some(icons::GIT_DIFF), t("Review Changes…"), kit::keys(&Action::Review), false).clicked()
                            {
                                self.actions.push(Action::Review);
                            }
                        });

                        if !changed.is_empty() {
                            ui.add_space(20.0);
                            let g = kit::section_galley(ui.painter(), t("Working Tree Changes"), p.text_faint);
                            let (r, _) = ui.allocate_exact_size(vec2(col_w, g.size().y + 6.0), Sense::hover());
                            ui.painter().galley(r.min, g, p.text_faint);
                            egui::Frame::NONE
                                .fill(p.surface)
                                .stroke(Stroke::new(1.0, if dark { p.border } else { p.border_strong }))
                                .corner_radius(CornerRadius::same(8))
                                .show(ui, |ui| {
                                    ui.set_width(col_w);
                                    for (i, (path, ch)) in changed.iter().enumerate() {
                                        if i > 0 {
                                            let y = ui.cursor().top();
                                            ui.painter().hline(ui.max_rect().x_range(), y + 0.5, Stroke::new(1.0, p.border));
                                        }
                                        let rel = path.strip_prefix(root).unwrap_or(path).to_string_lossy().into_owned();
                                        let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                                        let (icon, color) = icons::file_icon(&name, false, false, &p);
                                        let (row, resp) = ui.allocate_exact_size(vec2(col_w, 28.0), Sense::click());
                                        if resp.hovered() {
                                            ui.painter().rect_filled(row, CornerRadius::same(6), p.hover);
                                            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                                        }
                                        ui.painter().text(pos2(row.left() + 14.0, row.center().y), Align2::CENTER_CENTER, icon, style::icon_font(13.5), color);
                                        let mut job = egui::text::LayoutJob::simple_singleline(rel, style::mono(11.5), p.text);
                                        job.wrap = egui::text::TextWrapping::truncate_at_width((row.width() - 54.0).max(20.0));
                                        let rg = ui.painter().layout_job(job);
                                        ui.painter().galley(pos2(row.left() + 28.0, row.center().y - rg.size().y / 2.0), rg, p.text);
                                        ui.painter().text(pos2(row.right() - 14.0, row.center().y), Align2::RIGHT_CENTER, ch.letter(), style::mono_semibold(10.5), super::explorer_ui::change_color(*ch, &p));
                                        if resp.clicked() {
                                            self.actions.push(Action::OpenFile(path.clone(), None));
                                        }
                                    }
                                });
                        }

                        if !recent.is_empty() {
                            ui.add_space(18.0);
                            let g = kit::section_galley(ui.painter(), t("Recently closed"), p.text_faint);
                            let (r, _) = ui.allocate_exact_size(vec2(col_w, g.size().y + 6.0), Sense::hover());
                            ui.painter().galley(r.min, g, p.text_faint);
                            for path in recent {
                                let rel = path.strip_prefix(root).unwrap_or(&path).to_string_lossy().into_owned();
                                let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                                let (icon, color) = icons::file_icon(&name, false, false, &p);
                                let (row, resp) = ui.allocate_exact_size(vec2(col_w, 26.0), Sense::click());
                                if resp.hovered() {
                                    ui.painter().rect_filled(row, CornerRadius::same(5), p.faint_row());
                                    ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                                }
                                ui.painter().text(pos2(row.left() + 8.0 + 7.0, row.center().y), Align2::CENTER_CENTER, icon, style::icon_font(13.5), color);
                                let mut job = egui::text::LayoutJob::simple_singleline(rel, style::mono(11.5), p.text_muted);
                                job.wrap = egui::text::TextWrapping::truncate_at_width((row.width() - 38.0).max(20.0));
                                let rg = ui.painter().layout_job(job);
                                ui.painter().galley(pos2(row.left() + 8.0 + 14.0 + 8.0, row.center().y - rg.size().y / 2.0), rg, p.text_muted);
                                if resp.clicked() {
                                    self.actions.push(Action::OpenFile(path, None));
                                }
                            }
                        }
                    });
                });
            });
    }
}

/// Clips `s` to at most `max_chars` characters on a UTF-8 boundary, returning a borrowed slice
/// when short enough or an owned truncated string with `"…"`.
fn clip_chars(s: &str, max_chars: usize) -> std::borrow::Cow<'_, str> {
    if s.len() <= max_chars {
        return std::borrow::Cow::Borrowed(s);
    }
    match s.char_indices().nth(max_chars) {
        Some((idx, _)) => std::borrow::Cow::Owned(format!("{}…", &s[..idx])),
        None => std::borrow::Cow::Borrowed(s),
    }
}

/// Byte offsets `(start, end)` of each line in `text` (excluding trailing `\r`/`\n`).
fn compute_line_ranges(text: &str) -> Vec<(usize, usize)> {
    let mut ranges = Vec::new();
    let mut start = 0usize;
    for (i, b) in text.bytes().enumerate() {
        if b == b'\n' {
            let end = if i > start && text.as_bytes()[i - 1] == b'\r' { i - 1 } else { i };
            ranges.push((start, end));
            start = i + 1;
        }
    }
    let end = if text.len() > start && text.as_bytes()[text.len() - 1] == b'\r' {
        text.len() - 1
    } else {
        text.len()
    };
    ranges.push((start, end));
    ranges
}

/// Virtualized fast read-only code viewer for large (> 350 KB), truncated (> 2 MB), or
/// long-line/minified files (such as Draw.io HTML exports or minified JS/JSON bundles).
/// Only paints visible rows and caps per-line glyph counts so GPU vertex buffers stay < 1 MB.
#[allow(clippy::too_many_arguments)]
fn paint_virtual_text(
    ui: &mut Ui,
    project: ProjectId,
    file: &mut crate::editor::OpenFile,
    find: Option<&crate::editor::Find>,
    goto: Option<usize>,
    font: egui::FontId,
    font_size: f32,
    line_h: f32,
    gutter: f32,
    dark: bool,
    p: &style::Palette,
    open_browser: &mut Option<PathBuf>,
) -> bool {
    let is_browser_file = crate::platform::browser_file(&file.path);
    let reason = if file.truncated {
        t("Large file (first 2 MB shown) — read-only fast viewer")
    } else if file.max_line_bytes > crate::editor::MAX_HIGHLIGHT_LINE_LEN {
        t("Long or minified lines — read-only fast viewer")
    } else {
        t("Large file — read-only fast viewer")
    };
    egui::Frame::NONE
        .inner_margin(egui::Margin { left: 10, right: 10, top: 6, bottom: 6 })
        .fill(p.surface)
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            let left = |ui: &mut Ui| {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 6.0;
                    kit::icon(ui, icons::INFO, 13.0, p.accent_text);
                    ui.add(egui::Label::new(RichText::new(reason).size(style::SMALL).color(p.text_muted)).truncate());
                });
            };
            if is_browser_file {
                egui::Sides::new().spacing(10.0).shrink_left().show(
                    ui,
                    left,
                    |ui| {
                        let btn = kit::button_ex(ui, kit::Kind::Primary, Some(icons::GLOBE), t("Open in Browser"), true);
                        #[cfg(debug_assertions)]
                        crate::devtest::record("editor-open-browser", btn.rect);
                        if btn.clicked() {
                            *open_browser = Some(file.path.clone());
                        }
                    },
                );
            } else {
                left(ui);
            }
        });
    let y_sep = ui.cursor().top();
    ui.painter().hline(ui.max_rect().x_range(), y_sep, Stroke::new(1.0, p.border));

    // Cache line byte ranges on the OpenFile so scrolling never re-scans a 2 MB buffer and
    // closing/reopening or freeing editor memory cannot leak or reuse stale byte offsets.
    let ranges: crate::editor::LineRanges = match &file.virtual_lines {
        Some((rev, r)) if *rev == file.rev => r.clone(),
        _ => {
            let r = std::sync::Arc::new(compute_line_ranges(&file.text));
            file.virtual_lines = Some((file.rev, r.clone()));
            r
        }
    };
    let total_lines = ranges.len().max(1);
    let mut area = egui::ScrollArea::both()
        .id_salt(("virtual-editor", project, file.path.clone()))
        .auto_shrink([false, false]);
    if let Some(line) = goto {
        area = area.vertical_scroll_offset((line.saturating_sub(5) as f32) * line_h);
    }
    let number_font = egui::FontId::monospace(font_size - 1.0);
    let view_id = egui::Id::new(("editor-text", file.path.clone()));
    #[cfg(debug_assertions)]
    crate::devtest::record("editor-text", ui.available_rect_before_wrap().intersect(ui.clip_rect()));
    let mut seen = ui.ctx().memory(|m| m.has_focus(view_id));
    let mut clicked_line = None;
    let syntax = file.syntax.clone();
    let plain = p.text;
    let max_chars = crate::editor::MAX_VIRTUAL_LINE_CHARS;
    area.show_rows(ui, line_h, total_lines, |ui, row_range| {
        ui.spacing_mut().item_spacing.y = 0.0;
        let view_w = ui.available_width().max(200.0);
        for line_idx in row_range {
            let (s, e) = ranges.get(line_idx).copied().unwrap_or((0, 0));
            let raw = file.text.get(s..e).unwrap_or("");
            let (shown, omitted_chars) = if raw.len() > max_chars {
                match raw.char_indices().nth(max_chars) {
                    Some((cut, _)) => (&raw[..cut], raw[cut..].chars().count()),
                    None => (raw, 0),
                }
            } else {
                (raw, 0)
            };
            let mut job = egui::text::LayoutJob::default();
            job.wrap.max_width = f32::INFINITY;
            if raw.len() <= crate::editor::MAX_HIGHLIGHT_LINE_LEN {
                for (c, piece) in crate::editor::line_colors(shown, &syntax, dark, plain) {
                    job.append(&piece, 0.0, egui::TextFormat::simple(font.clone(), c));
                }
            } else {
                job.append(shown, 0.0, egui::TextFormat::simple(font.clone(), plain));
            }
            if omitted_chars > 0 {
                job.append(
                    &format!("  … (+{} {})", omitted_chars, t("more chars")),
                    0.0,
                    egui::TextFormat::simple(font.clone(), p.text_faint),
                );
            }
            let galley = ui.painter().layout_job(job);
            let row_w = view_w.max(gutter + galley.size().x + 24.0);
            let (rect, resp) = ui.allocate_exact_size(vec2(row_w, line_h), Sense::click());
            let current = file.caret.0 == line_idx + 1;
            if current {
                ui.painter().rect_filled(rect, CornerRadius::ZERO, p.hover);
            }
            // Highlight any ⌘F find matches on this visible line.
            if let Some(f) = find.filter(|f| !f.matches.is_empty()) {
                let from = f.matches.partition_point(|m| m.line < line_idx);
                for (mi, m) in f.matches.iter().enumerate().skip(from) {
                    if m.line > line_idx {
                        break;
                    }
                    if m.col.0 >= max_chars {
                        continue;
                    }
                    let c0 = m.col.0;
                    let c1 = m.col.1.min(max_chars);
                    let row0 = galley.rows.first();
                    if let Some(r0) = row0 {
                        let x0 = rect.left() + gutter + r0.x_offset(egui::epaint::text::CharIndex(c0));
                        let x1 = rect.left() + gutter + r0.x_offset(egui::epaint::text::CharIndex(c1));
                        let hr = Rect::from_min_max(pos2(x0, rect.top() + 1.0), pos2(x1.max(x0 + 2.0), rect.bottom() - 1.0));
                        let is_cur = f.current == Some(mi);
                        ui.painter().rect_filled(
                            hr,
                            CornerRadius::same(2),
                            if is_cur { style::soft(p.warn, 0.55) } else { style::soft(p.warn, 0.22) },
                        );
                    }
                }
            }
            let numbers_right = rect.left() + gutter - 23.0;
            ui.painter().text(
                pos2(numbers_right, rect.center().y),
                Align2::RIGHT_CENTER,
                (line_idx + 1).to_string(),
                number_font.clone(),
                if current { p.text_muted } else { p.text_faint },
            );
            ui.painter().galley(pos2(rect.left() + gutter, rect.center().y - galley.size().y / 2.0), galley, plain);
            if resp.clicked() {
                clicked_line = Some(line_idx + 1);
                ui.ctx().memory_mut(|m| m.request_focus(view_id));
                seen = true;
            }
        }
    });
    if let Some(ln) = clicked_line {
        file.caret = (ln, 1);
    }
    if seen && ui.input(|i| i.events.iter().any(|e| matches!(e, egui::Event::Copy))) {
        let idx = file.caret.0.saturating_sub(1);
        if let Some((s, e)) = ranges.get(idx).copied()
            && let Some(line) = file.text.get(s..e)
        {
            ui.ctx().copy_text(line.to_string());
        }
    }
    seen
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
/// Paints the path of `path` under `root` as a breadcrumb; returns where it ends (for what
/// follows it, such as a symbol).
pub fn paint_crumb(ui: &Ui, rect: Rect, root: &Path, path: &Path, p: &style::Palette) -> f32 {
    let rel = path.strip_prefix(root).unwrap_or(path);
    let parts: Vec<String> = rel.components().map(|c| c.as_os_str().to_string_lossy().into_owned()).collect();
    let painter = ui.painter();
    painter.hline(rect.x_range(), rect.bottom() - 0.5, Stroke::new(1.0, p.border));
    let font = egui::FontId::proportional(style::SMALL);
    let mut x = rect.left() + 12.0;
    let cy = rect.center().y;
    for (i, part) in parts.iter().enumerate() {
        let last = i + 1 == parts.len();
        if i > 0 {
            let g = painter.layout_no_wrap(icons::CARET_RIGHT.to_string(), style::icon_font(9.0), p.text_ghost);
            painter.galley(pos2(x, cy - g.size().y / 2.0), g.clone(), p.text_ghost);
            x += g.size().x + 6.0;
        }
        let color = if last { p.text } else { p.text_faint };
        let g = painter.layout_no_wrap(part.clone(), font.clone(), color);
        let w = g.size().x;
        painter.galley(pos2(x, cy - g.size().y / 2.0), g, color);
        x += w + 6.0;
    }
    x
}
