//! The editor's IDE layer over the text: diagnostics as wavy underlines, ⌘-click on a symbol,
//! hover, completion, signature help, code actions and rename. Its keys are taken before the
//! text sees them (`ide_keys`); what needs the laid-out text is drawn inside the text's scroll
//! area (`overlay`); the popups go on top afterwards (`ide_popups`).

use std::sync::Arc;
use std::time::{Duration, Instant};

use eframe::egui::{self, Align2, Color32, CornerRadius, Key, Modifiers, Rect, Sense, Stroke, Ui, pos2, vec2};
use eframe::epaint::text::CharIndex;
use tc_lsp::pos::{Encoding, LineIndex};
use tc_lsp::types as lt;

use super::{icons, kit, style};
use crate::actions::Action;
use crate::app::App;
use crate::editor::OpenFile;
use crate::i18n::t;
use crate::lsp::{Goto, Ide, IdeCmd};

/// The pointer rests this long on a symbol before its hover is asked for.
const HOVER_DELAY: Duration = Duration::from_millis(450);

/// What the popups need from the text drawn this frame.
pub struct Geom {
    /// The caret's rectangle on screen.
    pub caret: Option<Rect>,
    /// The word under the pointer on screen.
    pub word: Option<Rect>,
    /// Messages of the diagnostics under the pointer.
    pub diags: Vec<(Color32, String)>,
}

fn is_ident(c: char) -> bool {
    c.is_alphanumeric() || c == '_' || c == '$'
}

/// The identifier around char column `col` of `line` (char range in the line).
fn word_in(line: &str, col: usize) -> Option<(usize, usize)> {
    let chars: Vec<char> = line.chars().collect();
    let col = col.min(chars.len());
    let mut a = col;
    while a > 0 && is_ident(chars[a - 1]) {
        a -= 1;
    }
    let mut b = col;
    while b < chars.len() && is_ident(chars[b]) {
        b += 1;
    }
    (a < b).then_some((a, b))
}

fn severity_color(s: Option<lt::DiagnosticSeverity>, p: &style::Palette) -> Color32 {
    match s {
        Some(lt::DiagnosticSeverity::ERROR) | None => p.err,
        Some(lt::DiagnosticSeverity::WARNING) => p.warn,
        _ => p.info,
    }
}

/// A wavy underline from `x0` to `x1` at `y`.
fn wave(painter: &egui::Painter, x0: f32, x1: f32, y: f32, color: Color32) {
    let mut pts = Vec::new();
    let mut x = x0;
    let mut up = true;
    while x < x1 {
        pts.push(pos2(x, if up { y - 1.2 } else { y + 1.2 }));
        x += 2.5;
        up = !up;
    }
    pts.push(pos2(x1, y));
    painter.add(egui::Shape::line(pts, Stroke::new(1.0, color)));
}

/// The diagnostics, ⌘-click and hover of the text just drawn, and what typing asks for.
#[allow(clippy::too_many_arguments)]
pub fn overlay(
    ui: &mut Ui,
    out: &egui::text_edit::TextEditOutput,
    file: &mut OpenFile,
    diags: Option<&(Encoding, Vec<lt::Diagnostic>)>,
    ide: &mut Ide,
    p: &style::Palette,
    features: (bool, bool),
    triggers: &[char],
    cmds: &mut Vec<IdeCmd>,
) -> Geom {
    let (lsp_on, auto) = features;
    let resp = &out.response.response;
    let galley: &Arc<egui::Galley> = &out.galley;
    let origin = out.galley_pos;
    let clip = ui.clip_rect();
    if file.index.as_ref().map(|(r, _)| *r) != Some(file.rev) {
        file.index = Some((file.rev, LineIndex::new(&file.text)));
    }
    let Some((_, ix)) = file.index.as_ref() else {
        return Geom { caret: None, word: None, diags: Vec::new() };
    };
    let text = file.text.as_str();
    let first = galley.rows.partition_point(|r| origin.y + r.rect().bottom() < clip.top());
    let last = galley.rows.partition_point(|r| origin.y + r.rect().top() <= clip.bottom());
    let char_x = |row: &egui::epaint::text::PlacedRow, col: usize| origin.x + row.pos.x + row.x_offset(CharIndex(col.min(row.char_count_excluding_newline().0)));

    // ----- diagnostics in view ---------------------------------------------------------------
    if let Some((enc, list)) = diags {
        let from = list.partition_point(|d| (d.range.start.line as usize) < first);
        // The gutter shows each line's most severe one.
        let mut marks: Vec<(usize, u32, Color32)> = Vec::new();
        for d in list[from..].iter().take_while(|d| (d.range.start.line as usize) < last) {
            let line = d.range.start.line as usize;
            let Some(row) = galley.rows.get(line) else { continue };
            let start = ix.char_index(text, d.range.start, *enc) - ix.line_start(line);
            let end = if d.range.end.line == d.range.start.line {
                ix.char_index(text, d.range.end, *enc) - ix.line_start(line)
            } else {
                row.char_count_excluding_newline().0
            };
            let c = severity_color(d.severity, p);
            let x0 = char_x(row, start);
            let mut x1 = char_x(row, end);
            if x1 - x0 < 6.0 {
                x1 = x0 + 6.0;
            }
            let y = origin.y + row.pos.y + row.rect().height() - 1.5;
            wave(ui.painter(), x0, x1, y, c);
            let rank = d.severity.map_or(1, |s| match s {
                lt::DiagnosticSeverity::ERROR => 1,
                lt::DiagnosticSeverity::WARNING => 2,
                _ => 3,
            });
            match marks.last_mut() {
                Some(m) if m.0 == line => {
                    if rank < m.1 {
                        *m = (line, rank, c);
                    }
                }
                _ => marks.push((line, rank, c)),
            }
        }
        for (line, _, c) in marks {
            if let Some(row) = galley.rows.get(line) {
                ui.painter().circle_filled(pos2(resp.rect.left() + 5.0, origin.y + row.pos.y + row.rect().height() / 2.0), 2.5, c);
            }
        }
    }

    // ----- typing: completion and signature help ---------------------------------------------
    let caret = out.cursor_range.map(|c| c.primary.index.0);
    // (A read-only text says it changed when typed into, though it did not.)
    if let (Some(c), true) = (caret, resp.changed() && resp.has_focus() && lsp_on && !file.read_only) {
        cmds.push(IdeCmd::Edited);
        let line = ix.line_of(c);
        let col = c - ix.line_start(line);
        let before = ix.line_text(text, line).chars().nth(col.wrapping_sub(1));
        match before {
            Some(ch) if is_ident(ch) && auto => {
                let open = ide.completion.as_mut().filter(|pop| pop.path == file.path && c > pop.start && ix.line_of(pop.start) == line);
                match open {
                    Some(pop) => {
                        crate::lsp::refilter_in(text, c, pop);
                        if pop.shown.is_empty() {
                            // Nothing left of an incomplete list: the server may have more for
                            // what is typed now. Asked again (its answer opens the list), not
                            // only at the next key.
                            if pop.incomplete {
                                cmds.push(IdeCmd::Complete(false));
                            }
                            ide.completion = None;
                        }
                    }
                    None => cmds.push(IdeCmd::Complete(false)),
                }
            }
            Some(ch) if auto && triggers.contains(&ch) => {
                ide.completion = None;
                cmds.push(IdeCmd::Complete(false));
            }
            _ => ide.completion = None,
        }
        match before {
            Some('(') | Some(',') => cmds.push(IdeCmd::Signature),
            Some(')') => ide.signature = None,
            _ => {}
        }
    }
    // The caret left the word being completed (a click, an arrow key).
    if let (Some(c), Some(pop)) = (caret, ide.completion.as_ref())
        && pop.path == file.path
        && (c < pop.start || ix.line_of(c) != ix.line_of(pop.start))
    {
        ide.completion = None;
    }
    // Where popups go: the caret, or where it was while the keyboard is in a popup's field.
    let caret_rect = caret.or(file.caret_index).map(|c| {
        let r = galley.pos_from_cursor(egui::text::CCursor::new(c));
        r.translate(origin.to_vec2())
    });

    #[cfg(debug_assertions)]
    if let Some(r) = caret_rect {
        crate::devtest::record("editor-caret", r);
    }

    // ----- the pointer: ⌘-click, hover, diagnostics under it ---------------------------------
    let mut geom = Geom { caret: caret_rect, word: None, diags: Vec::new() };
    let pointer = resp.hover_pos().filter(|p| clip.contains(*p));
    let mut on_word = None;
    if let Some(pos) = pointer {
        let row_i = galley.rows.partition_point(|r| origin.y + r.rect().bottom() < pos.y);
        if let Some(row) = galley.rows.get(row_i)
            && pos.x <= origin.x + row.rect().right() + 2.0
        {
            let at = galley.cursor_from_pos(pos - origin).index.0;
            let line = ix.line_of(at);
            let col = at - ix.line_start(line);
            // The char under the pointer, not the gap after it.
            let col = if pos.x < char_x(row, col) { col.saturating_sub(1) } else { col };
            if let Some((a, b)) = word_in(ix.line_text(text, line), col) {
                let x0 = char_x(row, a);
                let x1 = char_x(row, b);
                let y = origin.y + row.pos.y;
                let rect = Rect::from_min_max(pos2(x0, y), pos2(x1, y + row.rect().height()));
                on_word = Some((ix.line_start(line) + a, rect));
                geom.word = Some(rect);
            }
            if let Some((enc, list)) = diags {
                let here = ix.line_start(line) + col;
                for d in list.iter().filter(|d| d.range.start.line as usize <= line && d.range.end.line as usize >= line) {
                    let (s, e) = ix.chars(text, &d.range, *enc);
                    if here >= s && here <= e.max(s + 1) {
                        geom.diags.push((severity_color(d.severity, p), d.message.clone()));
                    }
                }
            }
        }
    }
    let command = ui.input(|i| i.modifiers.command);
    // The hover popup stays while the pointer is on it, or in the gap on the way to it (over
    // the next line's words), so a long one can be scrolled.
    let over_popup = ide.hover_rect.zip(ui.ctx().pointer_hover_pos()).is_some_and(|(r, p)| r.expand(6.0).contains(p));
    match on_word {
        Some((start, rect)) if command && lsp_on => {
            // ⌘ held: the symbol is a link to its definition.
            ui.painter().hline(rect.x_range(), rect.bottom() - 1.0, Stroke::new(1.0, p.accent_text));
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
            if resp.clicked() {
                cmds.push(IdeCmd::DefinitionAt(start));
            }
            ide.hover_wait = None;
        }
        Some((start, _)) if lsp_on && !over_popup && !ui.input(|i| i.pointer.any_down()) => {
            let key = (file.path.clone(), start);
            let waiting = ide.hover_wait.as_ref().is_some_and(|(p, s, _)| *p == key.0 && *s == start);
            if !waiting {
                ide.hover_wait = Some((key.0.clone(), start, Instant::now()));
                if ide.hover.as_ref().is_some_and(|h| h.at != start || h.path != file.path) {
                    ide.hover = None;
                }
            }
            if let Some((_, _, since)) = &ide.hover_wait {
                let rested = since.elapsed();
                if rested >= HOVER_DELAY {
                    if ide.hover_asked.as_ref() != Some(&key) {
                        ide.hover_asked = Some(key);
                        cmds.push(IdeCmd::HoverAt(start));
                    }
                } else {
                    ui.ctx().request_repaint_after(HOVER_DELAY - rested);
                }
            }
        }
        _ => {
            ide.hover_wait = None;
            if !over_popup {
                ide.hover = None;
                ide.hover_asked = None;
            }
        }
    }

    // ----- right-click: the commands, at the place clicked -----------------------------------
    if resp.secondary_clicked()
        && let Some(pos) = resp.interact_pointer_pos()
    {
        let at = galley.cursor_from_pos(pos - origin).index.0;
        let line = ix.line_of(at);
        file.caret = (line + 1, at - ix.line_start(line) + 1);
        file.caret_index = Some(at);
        kit::select_text(ui.ctx(), resp.id, at, at);
    }
    // Right-click: copy and paste always; the language's commands when they are on.
    resp.context_menu(|ui| {
        kit::menu_body(ui);
        let ctx = ui.ctx().clone();
        if kit::menu_item(ui, Some(icons::COPY), t("Copy"), kit::keys(&crate::actions::Action::Copy), false).clicked() {
            ctx.send_viewport_cmd(egui::ViewportCommand::RequestCopy);
            ui.close();
        }
        if kit::menu_item(ui, Some(icons::CLIPBOARD_TEXT), t("Paste"), kit::keys(&crate::actions::Action::Paste), false).clicked() {
            ctx.send_viewport_cmd(egui::ViewportCommand::RequestPaste);
            ui.close();
        }
        if kit::menu_item(ui, Some(icons::SELECTION), t("Select All"), kit::keys(&crate::actions::Action::SelectAll), false).clicked() {
            let len = file.text.chars().count();
            kit::select_text(&ctx, resp.id, 0, len);
            ui.close();
        }
        if !lsp_on {
            return;
        }
        kit::menu_sep(ui);
        let mut item = |ui: &mut Ui, icon: &str, label: &str, cmd: IdeCmd| {
            let keys = kit::keys(&crate::actions::Action::Ide(cmd.clone()));
            if kit::menu_item(ui, Some(icon), label, keys, false).clicked() {
                cmds.push(cmd);
                ui.close();
            }
        };
        item(ui, icons::ARROW_SQUARE_OUT, t("Go to Definition"), IdeCmd::Goto(Goto::Definition));
        item(ui, icons::ARROW_SQUARE_OUT, t("Go to Type Definition"), IdeCmd::Goto(Goto::TypeDefinition));
        item(ui, icons::ARROW_SQUARE_OUT, t("Go to Implementation"), IdeCmd::Goto(Goto::Implementation));
        item(ui, icons::LIST, t("Find References"), IdeCmd::References);
        kit::menu_sep(ui);
        item(ui, icons::PENCIL_SIMPLE, t("Rename Symbol"), IdeCmd::Rename);
        item(ui, icons::LIGHTNING, t("Code Actions…"), IdeCmd::CodeActions);
        item(ui, icons::TEXT_AA, t("Format Document"), IdeCmd::Format);
        kit::menu_sep(ui);
        item(ui, icons::LIST, t("Go to Symbol in File…"), IdeCmd::Symbols);
        item(ui, icons::WARNING, t("Problems"), IdeCmd::Problems);
        item(ui, icons::ARROW_LEFT, t("Back"), IdeCmd::Back);
        item(ui, icons::ARROW_RIGHT, t("Forward"), IdeCmd::Forward);
    });
    geom
}

impl App {
    /// The IDE's keys while the text has the keyboard, before it sees them: the completion
    /// list and code action menu, and the commands' shortcuts.
    pub fn ide_keys(&mut self, ctx: &egui::Context, project: tc_core::ProjectId, cmds: &mut Vec<IdeCmd>) {
        let Some(editor) = self.editors.get_mut(&project) else { return };
        let active = editor.active;
        let Some(file) = editor.files.get_mut(active) else { return };
        let text_id = egui::Id::new(("editor-text", file.path.clone()));
        if !ctx.memory(|m| m.has_focus(text_id)) {
            return;
        }
        let ide = &mut self.lsp.ide;
        let take = |k: Key, m: Modifiers| ctx.input_mut(|i| i.consume_key(m, k));
        let none = Modifiers::NONE;
        let caret = file.caret_index.unwrap_or(0);

        if let Some(pop) = ide.completion.as_mut().filter(|c| c.path == file.path && !c.shown.is_empty()) {
            let n = pop.shown.len();
            if take(Key::ArrowDown, none) {
                pop.selected = (pop.selected + 1) % n;
            }
            if take(Key::ArrowUp, none) {
                pop.selected = (pop.selected + n - 1) % n;
            }
            if take(Key::Escape, none) {
                ide.completion = None;
            } else if std::mem::take(&mut ide.accept_pending) || take(Key::Enter, none) || take(Key::Tab, none) {
                let item = pop.items[pop.shown[pop.selected]].clone();
                if let Some((text, at)) = crate::lsp::accept_completion(&file.text, pop, &item, caret) {
                    file.text = text;
                    file.rev += 1;
                    file.lines = crate::editor::count_lines(&file.text);
                    let ix = LineIndex::new(&file.text);
                    let line = ix.line_of(at);
                    file.caret = (line + 1, at - ix.line_start(line) + 1);
                    file.caret_index = Some(at);
                    kit::select_text(ctx, text_id, at, at);
                }
                ide.completion = None;
                // A call was completed with the caret inside its parentheses: its signature.
                let after_paren = file.caret_index.and_then(|c| c.checked_sub(1)).and_then(|i| file.text.chars().nth(i)) == Some('(');
                if item.label.ends_with('(') || after_paren {
                    cmds.push(IdeCmd::Signature);
                }
            }
        }
        if let Some(menu) = ide.actions.as_mut().filter(|a| a.path == file.path) {
            let n = menu.items.len().max(1);
            if take(Key::ArrowDown, none) {
                menu.selected = (menu.selected + 1) % n;
            }
            if take(Key::ArrowUp, none) {
                menu.selected = (menu.selected + n - 1) % n;
            }
            if take(Key::Enter, none) {
                cmds.push(IdeCmd::RunAction(menu.selected));
            } else if take(Key::Escape, none) {
                ide.actions = None;
            }
        }
        if ide.signature.is_some() && take(Key::Escape, none) {
            ide.signature = None;
        }
        if ide.hover.is_some() && take(Key::Escape, none) {
            ide.hover = None;
        }
        if !self.settings.lsp {
            return;
        }
        let cmd = Modifiers::COMMAND;
        // A key's most specific shortcut first: `consume_key` ignores an extra Shift or Alt,
        // so plain F12 would also take ⇧F12.
        let shortcuts: [(Key, Modifiers, IdeCmd); 12] = [
            (Key::F12, Modifiers::SHIFT, IdeCmd::References),
            (Key::F12, cmd, IdeCmd::Goto(Goto::Implementation)),
            (Key::F12, none, IdeCmd::Goto(Goto::Definition)),
            (Key::F2, none, IdeCmd::Rename),
            (Key::Period, cmd, IdeCmd::CodeActions),
            (Key::F, Modifiers::SHIFT | Modifiers::ALT, IdeCmd::Format),
            (Key::O, Modifiers::ALT | cmd, IdeCmd::Symbols),
            (Key::M, Modifiers::SHIFT | cmd, IdeCmd::Problems),
            (Key::Minus, Modifiers::CTRL | Modifiers::SHIFT, IdeCmd::Forward),
            (Key::Minus, Modifiers::CTRL, IdeCmd::Back),
            (Key::Space, Modifiers::CTRL, IdeCmd::Complete(true)),
            (Key::T, Modifiers::ALT | cmd, IdeCmd::WorkspaceSymbols),
        ];
        for (k, m, c) in shortcuts {
            if take(k, m) {
                // ⌥ keys also type a character (⇧⌥F is "Ï" on a Mac): it must not reach the text.
                if m.alt {
                    ctx.input_mut(|i| i.events.retain(|e| !matches!(e, egui::Event::Text(_))));
                }
                cmds.push(c);
            }
        }
    }

    /// The popups over the text: completion, signature help, hover (with the diagnostics under
    /// the pointer), code actions, rename.
    pub fn ide_popups(&mut self, ctx: &egui::Context, path: &std::path::Path, geom: &Geom, cmds: &mut Vec<IdeCmd>) {
        let p = style::palette(ctx.global_style().visuals.dark_mode);
        let ide = &mut self.lsp.ide;
        let frame = egui::Frame::popup(&ctx.global_style())
            .inner_margin(egui::Margin::same(6))
            .corner_radius(CornerRadius::same(8));
        let mono = egui::FontId::monospace((self.settings.font_size - 1.0).max(9.0));

        // Completion, under the caret.
        if let (Some(pop), Some(caret)) = (ide.completion.as_mut().filter(|c| c.path == path), geom.caret) {
            let mut clicked = None;
            egui::Area::new(egui::Id::new("ide-completion"))
                .order(egui::Order::Foreground)
                .fixed_pos(caret.left_bottom() + vec2(-4.0, 2.0))
                .show(ctx, |ui| {
                    // The chosen item's signature and documentation beside the list, when it has any.
                    let chosen = pop.shown.get(pop.selected).and_then(|&i| pop.items.get(i));
                    let docs: Vec<tc_lsp::assist::Block> = chosen.and_then(|it| it.documentation.as_ref()).map(tc_lsp::assist::doc_blocks).unwrap_or_default();
                    let sig = chosen.and_then(|it| it.detail.clone());
                    let side = !docs.is_empty();
                    frame.show(ui, |ui| {
                        ui.horizontal_top(|ui| {
                        ui.spacing_mut().item_spacing.x = 0.0;
                        ui.vertical(|ui| {
                        ui.set_width(if side { 240.0 } else { 380.0 });
                        egui::ScrollArea::vertical().max_height(230.0).show(ui, |ui| {
                            for (row, &i) in pop.shown.iter().enumerate().take(100) {
                                let it = &pop.items[i];
                                let selected = row == pop.selected;
                                let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 22.0), Sense::click());
                                if selected {
                                    ui.painter().rect_filled(rect, CornerRadius::same(4), p.accent_soft);
                                    ui.scroll_to_rect(rect, None);
                                } else if resp.hovered() {
                                    ui.painter().rect_filled(rect, CornerRadius::same(4), p.hover);
                                }
                                let (badge, color) = kind_badge(it.kind, &p);
                                ui.painter().text(pos2(rect.left() + 12.0, rect.center().y), Align2::CENTER_CENTER, badge, egui::FontId::monospace(10.0), color);
                                let mut job = egui::text::LayoutJob::simple_singleline(it.label.clone(), mono.clone(), p.text);
                                job.wrap = egui::text::TextWrapping::truncate_at_width(rect.width() * 0.6);
                                let g = ui.painter().layout_job(job);
                                ui.painter().galley(pos2(rect.left() + 26.0, rect.center().y - g.size().y / 2.0), g, p.text);
                                if let Some(detail) = &it.detail {
                                    let mut job = egui::text::LayoutJob::simple_singleline(detail.clone(), egui::FontId::proportional(style::SMALL), p.text_faint);
                                    job.wrap = egui::text::TextWrapping::truncate_at_width(rect.width() * 0.36);
                                    let g = ui.painter().layout_job(job);
                                    ui.painter().galley(pos2(rect.right() - 6.0 - g.size().x, rect.center().y - g.size().y / 2.0), g, p.text_faint);
                                }
                                if resp.clicked() {
                                    clicked = Some(row);
                                }
                            }
                        });
                        });
                        if side {
                            let top = ui.min_rect().top();
                            let x = ui.cursor().left();
                            ui.painter().vline(x + 0.5, top..=(top + 230.0_f32.min(ui.min_rect().height())), Stroke::new(1.0, p.border));
                            ui.add_space(10.0);
                            ui.vertical(|ui| {
                                ui.set_width(220.0);
                                ui.set_max_height(230.0);
                                egui::ScrollArea::vertical().id_salt("completion-doc").max_height(230.0).show(ui, |ui| {
                                    ui.add_space(4.0);
                                    if let Some(sig) = &sig {
                                        ui.add(egui::Label::new(egui::RichText::new(sig).font(egui::FontId::monospace(11.5)).color(p.text)).wrap());
                                        ui.add_space(6.0);
                                    }
                                    for b in &docs {
                                        if b.code {
                                            ui.add(egui::Label::new(egui::RichText::new(&b.text).font(egui::FontId::monospace(11.5)).color(p.text)).wrap());
                                        } else {
                                            ui.add(egui::Label::new(egui::RichText::new(&b.text).size(12.0).color(p.text_muted)).wrap());
                                        }
                                    }
                                    ui.add_space(8.0);
                                    ui.label(egui::RichText::new(t("↵ insert · ⇥ insert · esc")).size(style::SMALL).color(p.text_faint));
                                });
                            });
                        }
                        });
                    });
                });
            #[cfg(debug_assertions)]
            crate::devtest::record("ide-completion", Rect::from_min_size(caret.left_bottom(), vec2(380.0, 30.0)));
            if let Some(row) = clicked {
                pop.selected = row;
                ide.accept_pending = true;
                ctx.memory_mut(|m| m.request_focus(egui::Id::new(("editor-text", path.to_path_buf()))));
            }
        }

        // Signature help, above the caret.
        if let (Some(sig), Some(caret)) = (ide.signature.as_ref().filter(|s| s.path == path), geom.caret) {
            egui::Area::new(egui::Id::new("ide-signature"))
                .order(egui::Order::Foreground)
                .pivot(Align2::LEFT_BOTTOM)
                .fixed_pos(caret.left_top() + vec2(-4.0, -4.0))
                .show(ctx, |ui| {
                    frame.show(ui, |ui| {
                        let mut job = egui::text::LayoutJob::default();
                        let chars: Vec<char> = sig.label.chars().collect();
                        let (a, b) = sig.active.unwrap_or((usize::MAX, usize::MAX));
                        let fmt = |strong: bool| egui::TextFormat {
                            font_id: mono.clone(),
                            color: if strong { p.accent_text } else { p.text_muted },
                            underline: if strong { Stroke::new(1.0, p.accent_text) } else { Stroke::NONE },
                            ..Default::default()
                        };
                        let part = |from: usize, to: usize| chars[from.min(chars.len())..to.min(chars.len())].iter().collect::<String>();
                        if a < chars.len() {
                            job.append(&part(0, a), 0.0, fmt(false));
                            job.append(&part(a, b), 0.0, fmt(true));
                            job.append(&part(b, chars.len()), 0.0, fmt(false));
                        } else {
                            job.append(&sig.label, 0.0, fmt(false));
                        }
                        job.wrap.max_width = 560.0;
                        ui.label(job);
                    });
                });
        }

        // Hover: the diagnostics under the pointer, then the server's information.
        let hover = ide.hover.as_ref().filter(|h| h.path == path);
        // With the pointer on the popup (off the word) it stays where it was, with the
        // diagnostics it showed, so a long one can be scrolled.
        let diags_id = egui::Id::new("ide-hover-diags");
        let kept = ide.hover_rect.filter(|r| ctx.pointer_hover_pos().is_some_and(|p| r.expand(6.0).contains(p)));
        let (at, diags) = match kept {
            Some(r) => (Some(r.min), ctx.data(|d| d.get_temp::<Vec<(Color32, String)>>(diags_id)).unwrap_or_default()),
            None => {
                ctx.data_mut(|d| d.insert_temp(diags_id, geom.diags.clone()));
                (geom.word.map(|w| w.left_bottom() + vec2(0.0, 4.0)), geom.diags.clone())
            }
        };
        ide.hover_rect = None;
        if (hover.is_some() || !diags.is_empty())
            && let Some(at) = at
        {
            let r = egui::Area::new(egui::Id::new("ide-hover"))
                .order(egui::Order::Tooltip)
                .fixed_pos(at)
                .show(ctx, |ui| {
                    frame.show(ui, |ui| {
                        ui.set_max_width(540.0);
                        egui::ScrollArea::vertical().max_height(300.0).show(ui, |ui| {
                            for (c, msg) in &diags {
                                ui.label(egui::RichText::new(msg).color(*c));
                            }
                            if let Some(h) = hover {
                                if !diags.is_empty() {
                                    ui.separator();
                                }
                                for b in &h.blocks {
                                    if b.code {
                                        egui::Frame::NONE
                                            .fill(style::soft(p.text, 0.06))
                                            .corner_radius(CornerRadius::same(4))
                                            .inner_margin(egui::Margin::symmetric(8, 6))
                                            .show(ui, |ui| {
                                                ui.label(egui::RichText::new(&b.text).font(mono.clone()).color(p.text));
                                            });
                                    } else {
                                        ui.label(egui::RichText::new(&b.text).size(12.5).color(p.text_muted));
                                    }
                                }
                                // Where to go from here.
                                ui.add_space(4.0);
                                ui.horizontal(|ui| {
                                    ui.spacing_mut().item_spacing.x = 12.0;
                                    for (label, cmd) in [(t("Go to Definition"), IdeCmd::Goto(Goto::Definition)), (t("References"), IdeCmd::References)] {
                                        let keys = kit::keys(&crate::actions::Action::Ide(cmd.clone())).unwrap_or_default();
                                        let r = ui.add(egui::Label::new(egui::RichText::new(format!("{label} {keys}")).size(style::SMALL).color(p.accent_text)).sense(Sense::click()));
                                        if r.hovered() {
                                            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                                        }
                                        if r.clicked() {
                                            cmds.push(cmd);
                                        }
                                    }
                                });
                            }
                        });
                    });
                });
            ide.hover_rect = Some(r.response.rect);
        }

        // Code actions, at the caret.
        if let (Some(menu), Some(caret)) = (ide.actions.as_ref().filter(|a| a.path == path), geom.caret) {
            let mut close = false;
            egui::Area::new(egui::Id::new("ide-actions"))
                .order(egui::Order::Foreground)
                .fixed_pos(caret.left_bottom() + vec2(-4.0, 2.0))
                .show(ctx, |ui| {
                    egui::Frame::menu(ui.style()).show(ui, |ui| {
                        kit::menu_body(ui);
                        ui.set_min_width(300.0);
                        // Grouped by kind: quick fixes first, then refactors, then the rest.
                        let kind_of = |it: &lt::CodeActionOrCommand| match it {
                            lt::CodeActionOrCommand::CodeAction(a) => {
                                let k = a.kind.as_ref().map(|k| k.as_str().to_string()).unwrap_or_default();
                                if k.starts_with("quickfix") { 0 } else if k.starts_with("refactor") { 1 } else { 2 }
                            }
                            lt::CodeActionOrCommand::Command(_) => 2,
                        };
                        let mut order: Vec<usize> = (0..menu.items.len()).collect();
                        order.sort_by_key(|i| kind_of(&menu.items[*i]));
                        let mut group = None;
                        for i in order {
                            let it = &menu.items[i];
                            let k = kind_of(it);
                            if group != Some(k) {
                                group = Some(k);
                                kit::menu_header(ui, [t("Quick fix"), t("Refactor"), t("Actions")][k]);
                            }
                            let (title, disabled) = match it {
                                lt::CodeActionOrCommand::Command(c) => (c.title.clone(), false),
                                lt::CodeActionOrCommand::CodeAction(a) => (a.title.clone(), a.disabled.is_some()),
                            };
                            let icon = if k == 0 { icons::LIGHTNING } else { icons::PENCIL_SIMPLE };
                            let r = ui.add_enabled_ui(!disabled, |ui| {
                                if i == menu.selected {
                                    let rect = Rect::from_min_size(ui.cursor().min, vec2(ui.available_width(), 26.0));
                                    ui.painter().rect_filled(rect, CornerRadius::same(5), p.accent_soft);
                                }
                                kit::menu_item(ui, Some(icon), &title, None, false)
                            }).inner;
                            if r.clicked() {
                                cmds.push(IdeCmd::RunAction(i));
                            }
                        }
                    });
                    if ui.input(|i| i.pointer.any_click()) && !ui.ui_contains_pointer() {
                        close = true;
                    }
                });
            if close {
                ide.actions = None;
            }
        }

        // Rename: the new name, typed where the symbol is.
        if let (Some(field), Some(caret)) = (ide.rename.as_mut().filter(|r| r.path == path), geom.caret) {
            let id = egui::Id::new("ide-rename");
            let mut done = None;
            egui::Area::new(egui::Id::new("ide-rename-area"))
                .order(egui::Order::Foreground)
                .fixed_pos(caret.left_bottom() + vec2(-4.0, 2.0))
                .show(ctx, |ui| {
                    frame.show(ui, |ui| {
                        ui.label(egui::RichText::new(t("Rename symbol")).size(style::SMALL).color(p.text_faint));
                        ui.add_space(2.0);
                        let r = kit::field(ui, id, &mut field.text, None, "", 280.0, true);
                        ui.add_space(4.0);
                        ui.label(egui::RichText::new(t("↵ rename everywhere · esc cancel")).size(style::SMALL).color(p.text_faint));
                        #[cfg(debug_assertions)]
                        crate::devtest::record("ide-rename", r.rect);
                        if std::mem::take(&mut field.focus) {
                            r.request_focus();
                            kit::select_text(ctx, id, 0, field.text.chars().count());
                        }
                        if r.lost_focus() {
                            done = Some(ui.input(|i| i.key_pressed(Key::Enter)));
                        }
                    });
                });
            match done {
                Some(true) => {
                    let name = field.text.trim().to_string();
                    if name.is_empty() {
                        // Nothing to rename to: the field closes, as with Esc.
                        ide.rename = None;
                    } else {
                        cmds.push(IdeCmd::RenameTo(name));
                    }
                    ctx.memory_mut(|m| m.request_focus(egui::Id::new(("editor-text", path.to_path_buf()))));
                }
                Some(false) => {
                    ide.rename = None;
                    ctx.memory_mut(|m| m.request_focus(egui::Id::new(("editor-text", path.to_path_buf()))));
                }
                None => {}
            }
        }
    }

    /// The language server's state and the file's problems, for the editor's status line.
    pub fn ide_status(&mut self, ui: &mut Ui, path: &std::path::Path) {
        let p = style::pal(ui);
        // Laid out right to left: the server, then the problems, then Undo Refactor.
        self.lsp_chip(ui, path);
        let (errors, warnings) = self.lsp.counts(path);
        if errors + warnings > 0 {
            let mut job = egui::text::LayoutJob::default();
            let small = egui::FontId::proportional(style::SMALL);
            if errors > 0 {
                job.append(icons::X_CIRCLE, 0.0, egui::TextFormat::simple(style::icon_font(12.0), p.err));
                job.append(&errors.to_string(), 3.0, egui::TextFormat::simple(small.clone(), p.err));
            }
            if warnings > 0 {
                job.append(icons::WARNING, if errors > 0 { 8.0 } else { 0.0 }, egui::TextFormat::simple(style::icon_font(12.0), p.warn));
                job.append(&warnings.to_string(), 3.0, egui::TextFormat::simple(small, p.warn));
            }
            let r = ui.add(egui::Label::new(job).sense(Sense::click()));
            if r.hovered() {
                ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
            }
            if r.on_hover_text(kit::tip(t("Problems"), Action::Ide(IdeCmd::Problems))).clicked() {
                self.actions.push(Action::Ide(IdeCmd::Problems));
            }
        }
        if let Some(u) = &self.lsp.ide.undo
            && u.at.elapsed() < Duration::from_secs(60)
        {
            let r = ui.add(egui::Label::new(egui::RichText::new(t("Undo Refactor")).size(style::SMALL).color(p.accent_text)).sense(Sense::click()));
            if r.clicked() {
                self.actions.push(Action::Ide(IdeCmd::UndoRefactor));
            }
        }
    }

    /// The language server of the file: its state as a dot and words; a server that starts
    /// when needed and has not yet says so.
    fn lsp_chip(&mut self, ui: &mut Ui, path: &std::path::Path) {
        let p = style::pal(ui);
        let chip = |ui: &mut Ui, dot: Option<Color32>, text: &str| {
            let w = 6.0 + 6.0 + style::text_width(ui, text) * (style::SMALL / style::BODY) + 6.0;
            let g = ui.painter().layout_no_wrap(text.to_string(), egui::FontId::proportional(style::SMALL), p.text_faint);
            let (rect, r) = ui.allocate_exact_size(vec2(6.0 + 6.0 + g.size().x, 18.0), Sense::click());
            let c = pos2(rect.left() + 3.0, rect.center().y);
            match dot {
                Some(d) => {
                    ui.painter().circle_filled(c, 3.0, d);
                }
                None => {
                    ui.painter().circle_stroke(c, 2.5, Stroke::new(1.0, p.text_faint));
                }
            }
            ui.painter().galley(pos2(rect.left() + 12.0, rect.center().y - g.size().y / 2.0), g, p.text_faint);
            let _ = w;
            #[cfg(debug_assertions)]
            crate::devtest::record("ide-status", rect);
            r
        };
        let Some(server) = self.lsp.server_for(path) else {
            // Not started: on demand, the first edit or language feature starts it.
            if self.settings.lsp
                && self.settings.lsp_on_demand
                && let Some((spec, _)) = tc_lsp::servers::for_path(path)
            {
                chip(ui, None, &format!("{} · {}", spec.name, t("starts when needed")))
                    .on_hover_text(t("It starts when you edit the file or ask for a language feature."));
            }
            return;
        };
        use crate::lsp::ServerState as S;
        let (dot, text) = match &server.state {
            S::Ready => match &server.progress {
                Some(work) => (p.info, format!("{} · {work}", server.spec.name)),
                None => (p.ok, server.spec.name.to_string()),
            },
            S::Starting => (p.info, format!("{} · {}", server.spec.name, t("starting"))),
            S::Missing => (p.warn, format!("{} · {}", server.spec.name, t("not installed"))),
            S::Failed(_) => (p.err, format!("{} · {}", server.spec.name, t("stopped"))),
        };
        let tip = match &server.state {
            S::Missing => format!("{}\n{}", t("Install it from a terminal (click to copy):"), server.spec.install),
            S::Failed(why) => format!("{why}\n{}", t("Click to restart it.")),
            _ => format!("{}\n{}", server.spec.name, pid_text(server)),
        };
        let state = server.state.clone();
        let install = server.spec.install;
        let r = chip(ui, Some(dot), &text).on_hover_text(tip);
        if r.clicked() {
            match state {
                S::Missing => {
                    ui.ctx().copy_text(install.to_string());
                    self.toast_ok(t("Install command copied.").into());
                }
                S::Failed(_) => self.actions.push(Action::Ide(IdeCmd::RestartServer)),
                _ => {}
            }
        }
    }

    /// The problems panel at the bottom of the editor (⇧⌘M): the language server's errors,
    /// warnings and notes, for this file or the whole project. A click goes there.
    pub fn problems_panel(&mut self, ui: &mut Ui, project: tc_core::ProjectId, rect: Rect) {
        let p = style::pal(ui);
        ui.painter().rect_filled(rect, CornerRadius::ZERO, p.chrome);
        ui.painter().hline(rect.x_range(), rect.top() + 0.5, Stroke::new(1.0, p.border));
        let root = self.project(project).map(|x| x.path.clone()).unwrap_or_default();
        let current = self.editors.get(&project).and_then(|e| e.file()).map(|f| f.path.clone());
        let mut rows: Vec<(u8, String, std::path::PathBuf, usize, usize)> = Vec::new();
        for (path, (_, diags)) in &self.lsp.diagnostics {
            let here = if self.problems_project { path.starts_with(&root) } else { Some(path) == current.as_ref() };
            if !here {
                continue;
            }
            for d in diags {
                let sev = match d.severity {
                    Some(lt::DiagnosticSeverity::ERROR) | None => 0,
                    Some(lt::DiagnosticSeverity::WARNING) => 1,
                    _ => 2,
                };
                let msg = d.message.lines().next().unwrap_or_default().to_string();
                rows.push((sev, msg, path.clone(), d.range.start.line as usize, d.range.start.character as usize));
            }
        }
        rows.sort_by(|a, b| (a.0, &a.2, a.3).cmp(&(b.0, &b.2, b.3)));
        let counts = [0, 1, 2].map(|s| rows.iter().filter(|r| r.0 == s).count());
        // Header.
        let head = Rect::from_min_size(rect.min, vec2(rect.width(), 30.0));
        let mut x = head.left() + 12.0;
        let g = ui.painter().layout_no_wrap(t("Problems").to_string(), style::semibold(12.0), p.text);
        let gw = g.size().x;
        ui.painter().galley(pos2(x, head.center().y - g.size().y / 2.0), g, p.text);
        x += gw + 8.0;
        let dark = ui.visuals().dark_mode;
        for (i, (icon, c)) in [(icons::X_CIRCLE, p.err), (icons::WARNING, p.warn), (icons::INFO, p.info)].into_iter().enumerate() {
            let r = kit::paint_pill(ui.painter(), pos2(x, head.center().y), Some(icon), &counts[i].to_string(), c, style::tint(c, dark));
            x = r.right() + 6.0;
        }
        let close = Rect::from_center_size(pos2(head.right() - 6.0 - 11.0, head.center().y), vec2(22.0, 22.0));
        if ui.scope_builder(egui::UiBuilder::new().max_rect(close), |ui| kit::icon_button_ex(ui, icons::X, t("Close"), false, 22.0)).inner.clicked() {
            self.problems_open = false;
        }
        let options = [kit::Seg::new(t("This file"), ""), kit::Seg::new(t("Project"), "")];
        let sw = kit::segmented_width(ui, &options);
        let seg = Rect::from_min_size(pos2(close.left() - 6.0 - sw, head.center().y - kit::SEGMENTED_H / 2.0), vec2(sw, kit::SEGMENTED_H));
        if let Some(i) = kit::segmented_in(ui, seg, egui::Id::new("problems-scope"), &options, self.problems_project as usize) {
            self.problems_project = i == 1;
        }
        // Rows.
        let list = Rect::from_min_max(pos2(rect.left(), head.bottom()), rect.max);
        let mut go = None;
        ui.scope_builder(egui::UiBuilder::new().max_rect(list), |ui| {
            ui.set_clip_rect(list);
            if rows.is_empty() {
                ui.painter().text(pos2(list.left() + 12.0, list.top() + 14.0), Align2::LEFT_CENTER, t("No problems found."), egui::FontId::proportional(style::SMALL), p.text_faint);
                return;
            }
            egui::ScrollArea::vertical().id_salt("problems").auto_shrink([false, false]).show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 0.0;
                for (sev, msg, path, line, col) in &rows {
                    let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 26.0), Sense::click());
                    if resp.hovered() {
                        ui.painter().rect_filled(r, CornerRadius::ZERO, p.faint_row());
                    }
                    let (icon, c) = [(icons::X_CIRCLE, p.err), (icons::WARNING, p.warn), (icons::INFO, p.info)][*sev as usize];
                    ui.painter().text(pos2(r.left() + 12.0 + 7.0, r.center().y), Align2::CENTER_CENTER, icon, style::icon_font(14.0), c);
                    let place = format!("{}:{}", path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(), line + 1);
                    let pg = ui.painter().layout_no_wrap(place, egui::FontId::proportional(style::SMALL), p.text_faint);
                    let pw = pg.size().x;
                    ui.painter().galley(pos2(r.right() - 12.0 - pw, r.center().y - pg.size().y / 2.0), pg, p.text_faint);
                    let mut job = egui::text::LayoutJob::simple_singleline(msg.clone(), egui::FontId::proportional(12.5), p.text);
                    job.wrap = egui::text::TextWrapping::truncate_at_width((r.width() - 12.0 - 14.0 - 8.0 - pw - 24.0).max(10.0));
                    let g = ui.painter().layout_job(job);
                    ui.painter().galley(pos2(r.left() + 12.0 + 14.0 + 8.0, r.center().y - g.size().y / 2.0), g, p.text);
                    if resp.clicked() {
                        go = Some((path.clone(), *line, *col));
                    }
                }
            });
        });
        if let Some((path, line, col)) = go {
            self.actions.push(Action::GotoLocation(path, line, col));
        }
    }
}

/// "pid 4242" once the server runs (empty before).
fn pid_text(s: &crate::lsp::Server) -> String {
    s.pid.map(|p| format!("pid {p}")).unwrap_or_default()
}

/// A short tag and colour for a completion item's kind.
fn kind_badge(k: Option<lt::CompletionItemKind>, p: &style::Palette) -> (&'static str, Color32) {
    use lt::CompletionItemKind as K;
    match k {
        Some(K::FUNCTION) | Some(K::METHOD) | Some(K::CONSTRUCTOR) => ("ƒ", p.accent_text),
        Some(K::VARIABLE) | Some(K::FIELD) | Some(K::PROPERTY) => ("v", p.info),
        Some(K::STRUCT) | Some(K::CLASS) | Some(K::INTERFACE) | Some(K::ENUM) | Some(K::TYPE_PARAMETER) => ("T", p.warn),
        Some(K::MODULE) => ("m", p.text_muted),
        Some(K::KEYWORD) => ("k", p.text_muted),
        Some(K::SNIPPET) => ("⋯", p.text_muted),
        Some(K::CONSTANT) | Some(K::ENUM_MEMBER) => ("c", p.ok),
        _ => ("·", p.text_faint),
    }
}

#[cfg(test)]
mod tests {
    use super::word_in;

    #[test]
    fn words_in_a_line() {
        assert_eq!(word_in("let foo_bar = 1;", 5), Some((4, 11)));
        assert_eq!(word_in("let foo_bar = 1;", 3), Some((0, 3)));
        assert_eq!(word_in("a + b", 2), None);
    }
}

