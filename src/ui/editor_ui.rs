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
const CRUMB_H: f32 = 26.0;
const STATUS_H: f32 = 26.0;

impl App {
    pub fn editor_panel(&mut self, ui: &mut Ui, project: ProjectId, root: PathBuf) {
        let p = style::pal(ui);
        let full = ui.max_rect();
        ui.painter().rect_filled(full, CornerRadius::ZERO, p.canvas);

        let tabs = Rect::from_min_size(full.min, vec2(full.width(), TAB_H));
        self.editor_tabs(ui, project, tabs);
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
        // Path of the file, and the status line at the bottom.
        let crumb = Rect::from_min_size(body.min, vec2(body.width(), CRUMB_H));
        let status = Rect::from_min_max(pos2(body.left(), body.bottom() - STATUS_H), body.max);
        let text_rect = Rect::from_min_max(pos2(body.left(), crumb.bottom()), pos2(body.right(), status.top()));
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

    fn editor_tabs(&mut self, ui: &mut Ui, project: ProjectId, rect: Rect) {
        let p = style::pal(ui);
        let painter = ui.painter();
        painter.rect_filled(rect, CornerRadius::ZERO, p.chrome);
        painter.hline(rect.x_range(), rect.bottom() - 0.5, Stroke::new(1.0, p.border));

        let close = Rect::from_center_size(pos2(rect.right() - 20.0, rect.center().y), vec2(26.0, 26.0));
        let r = ui
            .scope_builder(egui::UiBuilder::new().max_rect(close), |ui| {
                kit::icon_button_ex(ui, icons::X, &kit::tip(t("Close Editor"), Action::ToggleEditor), false, 26.0)
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
                            let font = style::medium(style::BODY - 0.5);
                            let galley = ui.painter().layout_no_wrap(name.clone(), font, egui::Color32::PLACEHOLDER);
                            let w = 12.0 + 16.0 + 6.0 + galley.size().x + 8.0 + 18.0 + 8.0;
                            let (r, resp) = ui.allocate_exact_size(vec2(w, TAB_H), Sense::click());
                            let close_r = Rect::from_center_size(pos2(r.right() - 17.0, r.center().y), vec2(18.0, 18.0));
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
                                pos2(r.left() + 12.0 + 8.0, r.center().y),
                                Align2::CENTER_CENTER,
                                icon,
                                style::icon_font(14.0),
                                if on { p.accent_text } else { color },
                            );
                            let text_color = if on { p.text } else { p.text_muted };
                            painter.galley(
                                pos2(r.left() + 34.0, r.center().y - galley.size().y / 2.0),
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
        let rel = file.path.strip_prefix(root).unwrap_or(&file.path);
        let parts: Vec<String> = rel.components().map(|c| c.as_os_str().to_string_lossy().into_owned()).collect();
        let mut job = egui::text::LayoutJob::default();
        let font = egui::FontId::proportional(style::SMALL + 0.5);
        for (i, part) in parts.iter().enumerate() {
            let last = i + 1 == parts.len();
            if i > 0 {
                job.append("  ›  ", 0.0, egui::TextFormat::simple(font.clone(), p.text_faint));
            }
            let color = if last { p.text_muted } else { p.text_faint };
            job.append(part, 0.0, egui::TextFormat::simple(font.clone(), color));
        }
        let mut right = rect.right() - 12.0;
        if file.read_only {
            let text = if file.truncated { t("Read-only · first 2 MB") } else { t("Read-only") };
            let g = ui.painter().layout_no_wrap(text.to_string(), style::medium(style::SMALL), p.warn);
            let pill = Rect::from_min_size(
                pos2(right - g.size().x - 14.0, rect.center().y - 9.0),
                vec2(g.size().x + 14.0, 18.0),
            );
            ui.painter().rect_filled(pill, CornerRadius::same(9), style::soft(p.warn, 0.14));
            ui.painter().galley(pill.center() - g.size() / 2.0, g, p.warn);
            right = pill.left() - 8.0;
        }
        job.wrap = egui::text::TextWrapping::truncate_at_width((right - rect.left() - 14.0).max(10.0));
        let g = ui.painter().layout_job(job);
        ui.painter().galley(pos2(rect.left() + 14.0, rect.center().y - g.size().y / 2.0), g, p.text_muted);
    }

    fn editor_status(&mut self, ui: &mut Ui, project: ProjectId, rect: Rect) {
        let p = style::pal(ui);
        ui.painter().hline(rect.x_range(), rect.top() + 0.5, Stroke::new(1.0, p.border));
        let Some(file) = self.editors.get(&project).and_then(|e| e.file()) else { return };
        let (line, col) = file.caret;
        let mut parts = vec![
            tf("Ln {}, Col {}", &[&line.to_string(), &col.to_string()]),
            crate::editor::language_name(&file.syntax),
        ];
        // Encoding and line ends only when there is room besides the language server's status.
        if rect.width() > 640.0 {
            parts.push(if file.lossy { t("Not UTF-8").to_string() } else { "UTF-8".to_string() });
            parts.push(if file.crlf { "CRLF".into() } else { "LF".into() });
        }
        let text = parts.join("   ·   ");
        let text_w = style::text_width(ui, &text);
        ui.painter().text(
            pos2(rect.left() + 14.0, rect.center().y),
            Align2::LEFT_CENTER,
            text,
            egui::FontId::proportional(style::SMALL),
            p.text_faint,
        );
        let dirty = file.dirty() && !file.read_only;
        // The language server and the file's problems, right of the caret's place.
        let path = file.path.clone();
        let right = rect.right() - if dirty { 158.0 } else { 8.0 };
        let left = rect.left() + 14.0 + text_w + 20.0;
        if right > left {
            ui.scope_builder(
                egui::UiBuilder::new()
                    .max_rect(Rect::from_min_max(pos2(left, rect.top() + 2.0), pos2(right, rect.bottom() - 2.0)))
                    .layout(egui::Layout::right_to_left(egui::Align::Center)),
                |ui| {
                    ui.set_clip_rect(ui.max_rect().intersect(ui.clip_rect()));
                    ui.spacing_mut().item_spacing.x = 6.0;
                    self.ide_status(ui, &path);
                },
            );
        }
        if dirty {
            let r = Rect::from_min_max(pos2(rect.right() - 150.0, rect.top() + 2.0), pos2(rect.right() - 8.0, rect.bottom() - 2.0));
            let resp = ui
                .scope_builder(
                    egui::UiBuilder::new()
                        .max_rect(r)
                        .layout(egui::Layout::right_to_left(egui::Align::Center)),
                    |ui| {
                        let save = kit::button_ex(ui, kit::Kind::Primary, Some(icons::FLOPPY_DISK), t("Save"), true);
                        ui.label(RichText::new(t("Unsaved")).size(style::SMALL).color(p.warn));
                        save
                    },
                )
                .inner;
            if resp.on_hover_text(kit::tip(t("Save"), Action::SaveFile)).clicked() {
                self.actions.push(Action::SaveFile);
            }
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
                .inner_margin(egui::Margin::symmetric(12, 5))
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 6.0;
                        let r = ui.add(
                            egui::TextEdit::singleline(&mut f.query)
                                .id(find_id)
                                .hint_text(t("Find in file, or :line"))
                                .desired_width(230.0),
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
                        ui.label(RichText::new(info.0).size(style::SMALL).color(info.1));
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if kit::icon_button_ex(ui, icons::X, t("Close"), false, 24.0).clicked() {
                                close = true;
                            }
                            if kit::icon_button_ex(ui, icons::ARROW_DOWN, t("Next"), false, 24.0).clicked() {
                                go = Some(true);
                            }
                            if kit::icon_button_ex(ui, icons::ARROW_UP, t("Previous"), false, 24.0).clicked() {
                                go = Some(false);
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
            let mut reload = false;
            let mut force = false;
            egui::Frame::NONE
                .fill(style::soft(p.warn, 0.12))
                .inner_margin(egui::Margin::symmetric(14, 8))
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.horizontal(|ui| {
                        kit::icon(ui, icons::WARNING, 15.0, p.warn);
                        ui.add(
                            egui::Label::new(
                                RichText::new(t("This file changed on disk while you were editing it (probably an agent). Nothing was overwritten."))
                                    .color(p.text),
                            )
                            .wrap(),
                        );
                    });
                    ui.add_space(4.0);
                    ui.horizontal(|ui| {
                        if kit::button_ex(ui, kit::Kind::Secondary, None, t("Reload from Disk"), true).clicked() {
                            reload = true;
                        }
                        let label = if file.compare { t("Hide Differences") } else { t("Compare") };
                        if kit::button_ex(ui, kit::Kind::Secondary, None, label, true).clicked() {
                            file.compare = !file.compare;
                        }
                        if kit::button_ex(ui, kit::Kind::Danger, None, t("Keep Mine and Save"), true).clicked() {
                            force = true;
                        }
                    });
                });
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
            egui::ScrollArea::both()
                .id_salt(("compare", project))
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    ui.add_space(6.0);
                    for line in diff.lines() {
                        let color = if line.starts_with("+ ") {
                            p.ok
                        } else if line.starts_with("- ") {
                            p.err
                        } else {
                            p.text_muted
                        };
                        ui.label(RichText::new(line).monospace().size(font_size - 1.0).color(color));
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
            let mut output = egui::TextEdit::multiline(buffer)
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
            let _ = &mut output;
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
            kit::icon(ui, icons::CODE, 34.0, p.text_faint);
            ui.add_space(8.0);
            ui.label(RichText::new(t("No file open")).font(style::semibold(15.0)).color(p.text));
            ui.add_space(4.0);
            ui.label(
                RichText::new(tf(
                    "Click a file in the sidebar, or {}-click a path in the terminal.",
                    &[if cfg!(target_os = "macos") { "⌘" } else { "Ctrl" }],
                ))
                    .size(style::SMALL + 0.5)
                    .color(p.text_muted),
            );
            ui.add_space(12.0);
            if kit::button_ex(ui, kit::Kind::Secondary, Some(icons::MAGNIFYING_GLASS), &kit::tip(t("Open File…"), Action::QuickOpen), false)
                .clicked()
            {
                self.actions.push(Action::QuickOpen);
            }
            if !recent.is_empty() {
                ui.add_space(20.0);
                ui.label(RichText::new(t("RECENTLY CLOSED")).font(style::semibold(style::SMALL - 0.5)).color(p.text_faint));
                ui.add_space(4.0);
                for path in recent {
                    let rel = path.strip_prefix(root).unwrap_or(&path).to_string_lossy().into_owned();
                    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                    // The icon in its file type's colour, as on the tabs and in the file tree.
                    let (icon, color) = icons::file_icon(&name, false, false, &p);
                    let font = egui::FontId::proportional(style::BODY - 0.5);
                    let mut job = egui::text::LayoutJob::default();
                    job.append(icon, 0.0, egui::TextFormat::simple(font.clone(), color));
                    job.append(&format!("  {rel}"), 0.0, egui::TextFormat::simple(font, p.text_muted));
                    let r = ui.add(egui::Label::new(job).sense(Sense::click()));
                    if r.hovered() {
                        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                    }
                    if r.clicked() {
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
