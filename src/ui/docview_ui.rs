//! Documents in the editor: a toolbar (pages, zoom, open in the default app), the document,
//! and a status line. PDF pages are pictures rendered for the zoom shown; Word and PowerPoint
//! files are laid out once for the width shown and only what is on screen is drawn; sheets are
//! a grid with its header row and row numbers kept in view.

use std::collections::HashSet;
use std::sync::Arc;

use eframe::egui::{self, Align2, Color32, CornerRadius, Galley, Rect, Sense, Stroke, Ui, pos2, vec2};
use tc_core::ProjectId;
use tc_doc::office::{Block, Para, ParaStyle, Picture, Span};

use super::{icons, kit, style};
use crate::app::App;
use crate::docview::{Content, DocMatch, DocView, MAX_DOC_MATCHES, Pic, State};
use crate::i18n::{t, tf};

const BAR_H: f32 = 34.0;
const STATUS_H: f32 = 26.0;
const TABS_H: f32 = 30.0;
const CELL_BAR_H: f32 = 30.0;
const FIND_H: f32 = 36.0;
const ZOOMS: [f32; 13] = [0.5, 0.67, 0.8, 0.9, 1.0, 1.1, 1.25, 1.5, 1.75, 2.0, 2.5, 3.0, 4.0];

/// What a click in the document's toolbar asks of the app.
enum Ask {
    OpenDefault,
    Reveal,
    EditAsText,
}

impl App {
    /// The editor's active file is a document: its toolbar, the document and a status line.
    pub fn doc_panel(&mut self, ui: &mut Ui, project: ProjectId, body: Rect) {
        let ctx = ui.ctx().clone();
        let p = style::pal(ui);
        let Some(view) = self
            .editors
            .get_mut(&project)
            .and_then(|e| e.file_mut())
            .and_then(|f| f.doc.as_deref_mut())
        else {
            return;
        };
        view.poll(&ctx);
        let bar = Rect::from_min_size(body.min, vec2(body.width(), BAR_H));
        let status = Rect::from_min_max(pos2(body.left(), body.bottom() - STATUS_H), body.max);
        let find_h = if view.find.is_some() { FIND_H } else { 0.0 };
        let find_rect = Rect::from_min_size(pos2(body.left(), bar.bottom()), vec2(body.width(), find_h));
        let content = Rect::from_min_max(pos2(body.left(), find_rect.bottom()), pos2(body.right(), status.top()));
        // A click in the document gives it the keyboard (a terminal takes the keyboard back
        // whenever nobody has it): ⌘F then searches here and ⌘W closes the document. Widgets
        // inside (a sheet's grid) take it from there.
        let doc_id = egui::Id::new(("doc-view", project));
        let doc_area = ui.interact(content, doc_id, Sense::click());
        if ui.input(|i| i.pointer.primary_pressed()) && ui.rect_contains_pointer(content) {
            doc_area.request_focus();
        }
        if doc_area.has_focus() {
            self.editor_seen = true;
        }
        #[cfg(debug_assertions)]
        crate::devtest::record("doc-body", content);
        let mut ask = None;
        toolbar(ui, view, bar, &mut ask);
        status_line(ui, view, status);
        if view.find.is_some() {
            refresh_find(view);
            if find_bar(ui, view, find_rect, project) {
                // The search field has the keyboard: the editor's keys (⌘W…) apply.
                self.editor_seen = true;
            }
            follow_sheet_match(view);
        }
        ui.scope_builder(egui::UiBuilder::new().max_rect(content), |ui| {
            ui.set_clip_rect(content);
            // Pinch or Ctrl+wheel over the document zooms it.
            if ui.rect_contains_pointer(content) {
                let z = ui.input(|i| i.zoom_delta());
                if (z - 1.0).abs() > 0.001 {
                    view.zoom = (view.zoom * z).clamp(0.25, 5.0);
                }
            }
            match &view.state {
                State::Loading => loading(ui, content),
                State::Failed(e) => {
                    let e = e.clone();
                    failed(ui, content, &e, &mut ask);
                }
                State::Ready(Content::Pdf) => pdf_body(ui, view, content),
                State::Ready(Content::Text(_) | Content::Slides(_)) => rich_body(ui, view, content, &p),
                State::Ready(Content::Sheet(_)) => sheet_body(ui, view, content, &p),
                State::Ready(Content::Image) => image_body(ui, view, content, &p),
            }
        });
        let path = view.path.clone();
        match ask {
            Some(Ask::OpenDefault) => crate::platform::reveal(&path),
            Some(Ask::Reveal) => crate::platform::reveal_in_folder(&path),
            Some(Ask::EditAsText) => {
                if let Ok(mut text) = crate::editor::load(&path)
                    && let Some(f) = self.editors.get_mut(&project).and_then(|e| e.file_mut())
                {
                    text.goto_line = None;
                    *f = text;
                }
            }
            None => {}
        }
    }
}

// ----- toolbar and status line ---------------------------------------------------------------

fn toolbar(ui: &mut Ui, view: &mut DocView, rect: Rect, ask: &mut Option<Ask>) {
    let p = style::pal(ui);
    ui.painter().rect_filled(rect, CornerRadius::ZERO, p.canvas);
    ui.painter().hline(rect.x_range(), rect.bottom() - 0.5, Stroke::new(1.0, p.border));
    let name = view.path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let (icon, color) = icons::file_icon(&name, false, false, &p);
    ui.scope_builder(
        egui::UiBuilder::new()
            .max_rect(rect.shrink2(vec2(10.0, 3.0)))
            .layout(egui::Layout::left_to_right(egui::Align::Center)),
        |ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            ui.label(egui::RichText::new(icon).font(style::icon_font(15.0)).color(color));
            ui.add_space(2.0);
            let pdf_pages = view.pdf.as_ref().map(|p| (p.page, p.pages.len())).filter(|(_, n)| *n > 0);
            ui.add(egui::Label::new(egui::RichText::new(&name).font(style::medium(style::BODY - 0.5)).color(p.text)).truncate());
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = 2.0;
                if kit::icon_button_ex(ui, icons::FOLDER_OPEN, reveal_label(), false, 26.0).clicked() {
                    *ask = Some(Ask::Reveal);
                }
                if kit::icon_button_ex(ui, icons::ARROW_SQUARE_OUT, t("Open in Default App"), false, 26.0).clicked() {
                    *ask = Some(Ask::OpenDefault);
                }
                if view.kind == tc_doc::Kind::Csv
                    && kit::button_ex(ui, kit::Kind::Ghost, Some(icons::CODE), t("Edit as Text"), true).clicked()
                {
                    *ask = Some(Ask::EditAsText);
                }
                if matches!(view.state, State::Ready(_)) {
                    kit::vdivider(ui, 16.0);
                    zoom_controls(ui, view);
                }
                if let Some((page, n)) = pdf_pages {
                    kit::vdivider(ui, 16.0);
                    if kit::icon_button_ex(ui, icons::CARET_DOWN, t("Next Page"), false, 24.0).clicked()
                        && let Some(pdf) = view.pdf.as_mut()
                    {
                        pdf.jump = Some((page + 1).min(n - 1));
                    }
                    ui.label(
                        egui::RichText::new(tf("{} of {}", &[&(page + 1).to_string(), &n.to_string()]))
                            .size(style::SMALL + 0.5)
                            .color(p.text_muted),
                    );
                    if kit::icon_button_ex(ui, icons::CARET_UP, t("Previous Page"), false, 24.0).clicked()
                        && let Some(pdf) = view.pdf.as_mut()
                    {
                        pdf.jump = Some(page.saturating_sub(1));
                    }
                }
            });
        },
    );
}

fn reveal_label() -> &'static str {
    if cfg!(target_os = "macos") { t("Show in Finder") } else { t("Show in Folder") }
}

fn zoom_controls(ui: &mut Ui, view: &mut DocView) {
    let p = style::pal(ui);
    if kit::icon_button_ex(ui, icons::MAGNIFYING_GLASS_PLUS, t("Zoom In"), false, 24.0).clicked() {
        view.zoom = ZOOMS.iter().copied().find(|z| *z > view.zoom + 0.01).unwrap_or(view.zoom);
    }
    let pct = format!("{:.0}%", view.zoom * 100.0);
    let r = ui.add(egui::Button::new(egui::RichText::new(pct).size(style::SMALL + 0.5).color(p.text_muted)).frame(false).min_size(vec2(44.0, 22.0)));
    if r.on_hover_text(t("Fit")).clicked() {
        view.zoom = 1.0;
    }
    if kit::icon_button_ex(ui, icons::MAGNIFYING_GLASS_MINUS, t("Zoom Out"), false, 24.0).clicked() {
        view.zoom = ZOOMS.iter().rev().copied().find(|z| *z < view.zoom - 0.01).unwrap_or(view.zoom);
    }
}

fn status_line(ui: &mut Ui, view: &DocView, rect: Rect) {
    let p = style::pal(ui);
    ui.painter().rect_filled(rect, CornerRadius::ZERO, p.canvas);
    ui.painter().hline(rect.x_range(), rect.top() + 0.5, Stroke::new(1.0, p.border));
    ui.painter().text(
        pos2(rect.left() + 14.0, rect.center().y),
        Align2::LEFT_CENTER,
        view.summary(),
        egui::FontId::proportional(style::SMALL),
        p.text_faint,
    );
    let right = match (&view.state, view.cell) {
        (State::Ready(Content::Sheet(b)), Some((r, c))) => b.sheets.get(view.sheet).map(|s| cell_name(s, r, c)),
        _ => None,
    };
    if let Some(right) = right {
        ui.painter().text(
            pos2(rect.right() - 14.0, rect.center().y),
            Align2::RIGHT_CENTER,
            right,
            egui::FontId::proportional(style::SMALL),
            p.text_faint,
        );
    }
}

fn loading(ui: &mut Ui, rect: Rect) {
    let p = style::pal(ui);
    let c = rect.center();
    ui.put(Rect::from_center_size(c - vec2(0.0, 14.0), vec2(22.0, 22.0)), egui::Spinner::new().size(20.0).color(p.text_faint));
    ui.painter().text(c + vec2(0.0, 14.0), Align2::CENTER_CENTER, t("Opening…"), egui::FontId::proportional(style::SMALL + 0.5), p.text_faint);
}

fn failed(ui: &mut Ui, rect: Rect, error: &str, ask: &mut Option<Ask>) {
    ui.scope_builder(egui::UiBuilder::new().max_rect(rect), |ui| {
        kit::empty_state(ui, icons::FILE, t("This document cannot be shown"), error, |ui| {
            if kit::button_ex(ui, kit::Kind::Secondary, Some(icons::ARROW_SQUARE_OUT), t("Open in Default App"), false).clicked() {
                *ask = Some(Ask::OpenDefault);
            }
        });
    });
}

// ----- search ---------------------------------------------------------------------------------

/// The search bar under the toolbar. Returns whether its field has the keyboard.
fn find_bar(ui: &mut Ui, view: &mut DocView, rect: Rect, project: ProjectId) -> bool {
    let p = style::pal(ui);
    let Some(f) = view.find.as_mut() else { return false };
    let id = egui::Id::new(("doc-find", project));
    let mut close = ui.memory(|m| m.has_focus(id)) && ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape));
    let mut focused = false;
    ui.painter().rect_filled(rect, CornerRadius::ZERO, p.chrome);
    ui.painter().hline(rect.x_range(), rect.bottom() - 0.5, Stroke::new(1.0, p.border));
    ui.scope_builder(
        egui::UiBuilder::new().max_rect(rect.shrink2(vec2(12.0, 5.0))).layout(egui::Layout::left_to_right(egui::Align::Center)),
        |ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            let r = ui.add(egui::TextEdit::singleline(&mut f.query).id(id).hint_text(t("Find in document")).desired_width(240.0));
            #[cfg(debug_assertions)]
            crate::devtest::record("doc-find", r.rect);
            if f.focus {
                r.request_focus();
                kit::select_text(ui.ctx(), id, 0, f.query.chars().count());
                f.focus = false;
            }
            focused = r.has_focus();
            if r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                let back = ui.input(|i| i.modifiers.shift);
                f.step(!back);
                r.request_focus();
                focused = true;
            }
            let n = f.matches.len();
            let more = if n >= MAX_DOC_MATCHES { "+" } else { "" };
            let (info, color) = if f.query.trim().is_empty() {
                (String::new(), p.text_muted)
            } else if n == 0 && f.searching {
                (t("Searching…").to_string(), p.text_muted)
            } else if n == 0 {
                (t("No results").to_string(), p.err)
            } else {
                match f.current {
                    Some(c) => (tf("{} of {}", &[&(c + 1).to_string(), &format!("{n}{more}")]), p.text_muted),
                    None => (tf("{} results", &[&format!("{n}{more}")]), p.text_muted),
                }
            };
            ui.label(egui::RichText::new(info).size(style::SMALL).color(color));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if kit::icon_button_ex(ui, icons::X, t("Close"), false, 24.0).clicked() {
                    close = true;
                }
                if kit::icon_button_ex(ui, icons::ARROW_DOWN, t("Next"), false, 24.0).clicked() {
                    f.step(true);
                }
                if kit::icon_button_ex(ui, icons::ARROW_UP, t("Previous"), false, 24.0).clicked() {
                    f.step(false);
                }
            });
        },
    );
    if close {
        view.find = None;
        ui.memory_mut(|m| m.surrender_focus(id));
        return false;
    }
    focused
}

/// Finds the query again when it or the document changed (more of a PDF's text read, another
/// layout). Typing jumps to the first match.
fn refresh_find(view: &mut DocView) {
    let stamp = match &view.state {
        State::Ready(Content::Pdf) => view.pdf.as_ref().map_or(0, |p| p.text.iter().filter(|t| t.is_some()).count() as u64),
        // No layout yet (let go while the tab was not shown, made again below): the matches
        // stay as they are, so coming back keeps the current one.
        State::Ready(Content::Text(_) | Content::Slides(_)) => match view.layout.as_ref() {
            Some(l) => l.items.len() as u64,
            None => return,
        },
        _ => 0,
    };
    let Some(f) = view.find.as_mut() else { return };
    let query = f.query.trim().to_lowercase();
    let key = (query.clone(), stamp);
    if f.key.as_ref() == Some(&key) {
        return;
    }
    let new_query = f.key.as_ref().is_none_or(|(q, _)| *q != query);
    f.key = Some(key);
    f.searching = false;
    let mut found = Vec::new();
    if !query.is_empty() {
        match &view.state {
            State::Ready(Content::Pdf) => {
                if let Some(pdf) = &view.pdf {
                    f.searching = pdf.text.iter().any(Option::is_none);
                    'pages: for (page, text) in pdf.text.iter().enumerate() {
                        for rects in text.as_ref().map(|t| t.find(&query)).unwrap_or_default() {
                            found.push(DocMatch::Pdf { page, rects });
                            if found.len() >= MAX_DOC_MATCHES {
                                break 'pages;
                            }
                        }
                    }
                }
            }
            State::Ready(Content::Text(_) | Content::Slides(_)) => {
                if let Some(layout) = &view.layout {
                    let q: Vec<char> = query.chars().collect();
                    for (item, it) in layout.items.iter().enumerate() {
                        let Item::Text { galley, .. } = it else { continue };
                        let text: Vec<char> = galley.text().chars().map(|c| c.to_lowercase().next().unwrap_or(c)).collect();
                        let mut i = 0;
                        while i + q.len() <= text.len() && found.len() < MAX_DOC_MATCHES {
                            if text[i..i + q.len()] == q[..] {
                                found.push(DocMatch::Text { item, start: i, end: i + q.len() });
                                i += q.len();
                            } else {
                                i += 1;
                            }
                        }
                    }
                }
            }
            State::Ready(Content::Sheet(book)) => {
                'sheets: for (sheet, s) in book.sheets.iter().enumerate() {
                    for row in 0..s.rows {
                        for col in 0..s.cols {
                            if s.cell(row, col).text().to_lowercase().contains(&query) {
                                found.push(DocMatch::Cell { sheet, row, col });
                                if found.len() >= MAX_DOC_MATCHES {
                                    break 'sheets;
                                }
                            }
                        }
                    }
                }
            }
            _ => {}
        }
    }
    let had = f.matches.len();
    f.matches = found;
    if new_query || had == 0 {
        f.current = None;
        if !f.matches.is_empty() {
            f.step(true);
        }
    } else if let Some(c) = f.current {
        f.current = Some(c.min(f.matches.len().saturating_sub(1)));
    }
}

/// A sheet's search moves the selected cell to the current match (and its sheet).
fn follow_sheet_match(view: &mut DocView) {
    let Some(f) = view.find.as_mut() else { return };
    if f.jump
        && let Some(DocMatch::Cell { sheet, row, col }) = f.current.and_then(|c| f.matches.get(c)).cloned()
    {
        f.jump = false;
        view.sheet = sheet;
        view.cell = Some((row, col));
        view.scroll_to_cell = true;
    }
}

/// The boxes of chars `start..end` of a laid-out text, one per row, relative to its origin.
fn char_boxes(galley: &Galley, start: usize, end: usize) -> Vec<Rect> {
    let mut out: Vec<Rect> = Vec::new();
    for i in start..end {
        let a = galley.pos_from_cursor(egui::text::CCursor::new(i));
        let b = galley.pos_from_cursor(egui::text::CCursor::new(i + 1));
        // The next char may start the next row: this one then ends at its row's end.
        let r = if (b.min.y - a.min.y).abs() < 0.5 {
            Rect::from_min_max(a.min, pos2(b.min.x, a.max.y))
        } else {
            Rect::from_min_max(a.min, pos2(a.min.x + a.height() * 0.5, a.max.y))
        };
        match out.last_mut() {
            Some(l) if (l.min.y - r.min.y).abs() < 0.5 => *l = l.union(r),
            _ => out.push(r),
        }
    }
    out
}

/// Highlight colours: every match, and the current one (marker yellow and orange, readable
/// on white pages and on both themes).
fn match_colors() -> (Color32, Color32) {
    (Color32::from_rgba_unmultiplied(255, 200, 40, 80), Color32::from_rgba_unmultiplied(255, 130, 0, 150))
}

// ----- PDF -----------------------------------------------------------------------------------

/// A render scale rounded to quarter steps of a doubling, so small zoom or size changes keep
/// the pages already rendered.
fn quantize(scale: f32) -> f32 {
    let steps = (scale.max(0.05).log2() * 4.0).round();
    2f32.powf(steps / 4.0)
}

fn pdf_body(ui: &mut Ui, view: &mut DocView, rect: Rect) {
    let p = style::pal(ui);
    let Some(pdf) = view.pdf.as_mut() else { return };
    if pdf.pages.is_empty() {
        return;
    }
    let margin = 20.0;
    let gap = 14.0;
    let avail = (rect.width() - margin * 2.0 - 10.0).max(80.0);
    let max_w = pdf.pages.iter().map(|p| p.0).fold(1.0f32, f32::max);
    // Fit the width (but small pages are not blown up past twice their size).
    let k = avail.min(max_w * 2.0) / max_w * view.zoom;
    let mut tops = Vec::with_capacity(pdf.pages.len());
    let mut y = margin;
    for (_, h) in &pdf.pages {
        tops.push(y);
        y += h * k + gap;
    }
    let total = vec2(max_w * k + margin * 2.0, y - gap + margin);
    let mut area = egui::ScrollArea::both()
        .id_salt(("pdf", view.path.clone()))
        .auto_shrink([false, false]);
    if let Some(j) = pdf.jump.take()
        && let Some(top) = tops.get(j)
    {
        area = area.vertical_scroll_offset(top - margin);
    }
    // The search's current match, a third down the view.
    let find_jump = view.find.as_mut().filter(|f| f.jump).and_then(|f| {
        f.jump = false;
        match f.matches.get(f.current?)? {
            DocMatch::Pdf { page, rects } => Some((*page, rects.first()?[1])),
            _ => None,
        }
    });
    if let Some((page, y)) = find_jump
        && let Some(top) = tops.get(page)
    {
        area = area.vertical_scroll_offset((top + y * k - rect.height() / 3.0).max(0.0));
    }
    let (all, current) = match_colors();
    let current_match = view.find.as_ref().and_then(|f| f.current);
    let page_matches: Vec<(usize, Vec<[f32; 4]>, bool)> = view
        .find
        .as_ref()
        .map(|f| {
            f.matches
                .iter()
                .enumerate()
                .filter_map(|(i, m)| match m {
                    DocMatch::Pdf { page, rects } => Some((*page, rects.clone(), Some(i) == current_match)),
                    _ => None,
                })
                .collect()
        })
        .unwrap_or_default();
    let Some(pdf) = view.pdf.as_mut() else { return };
    let ppp = ui.ctx().pixels_per_point();
    let scale = quantize(k * ppp);
    let pages = pdf.pages.clone();
    area.show_viewport(ui, |ui, vp| {
        let (r, _) = ui.allocate_exact_size(vec2(total.x.max(vp.width()), total.y), Sense::hover());
        let o = r.min;
        let visible: Vec<usize> = (0..pages.len())
            .filter(|i| {
                let top = tops[*i];
                top + pages[*i].1 * k >= vp.min.y && top <= vp.max.y
            })
            .collect();
        let (first, last) = (visible.first().copied().unwrap_or(0), visible.last().copied().unwrap_or(0));
        // The pages shown, and one either side so scrolling finds them ready.
        for i in first.saturating_sub(1)..(last + 2).min(pages.len()) {
            view.want_page(i, scale);
        }
        view.keep_pages(first.saturating_sub(2)..(last + 3).min(pages.len()));
        let mid = vp.center().y;
        let Some(pdf) = view.pdf.as_mut() else { return };
        pdf.page = (0..pages.len()).rfind(|i| tops[*i] <= mid).unwrap_or(0);
        let painter = ui.painter();
        for i in visible {
            let (w, h) = (pages[i].0 * k, pages[i].1 * k);
            let page = Rect::from_min_size(pos2(o.x + (r.width() - w) / 2.0, o.y + tops[i]), vec2(w, h));
            painter.rect_filled(page.translate(vec2(0.0, 2.0)).expand(1.0), CornerRadius::same(2), Color32::from_black_alpha(60));
            painter.rect_filled(page, CornerRadius::ZERO, Color32::WHITE);
            match pdf.textures.get(&i) {
                Some((_, tex)) => {
                    painter.image(tex.id(), page, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
                }
                None => {
                    painter.text(page.center(), Align2::CENTER_CENTER, tf("Page {}", &[&(i + 1).to_string()]), egui::FontId::proportional(style::SMALL), p.text_faint);
                }
            }
            for (_, rects, is_current) in page_matches.iter().filter(|(pg, _, _)| *pg == i) {
                for r in rects {
                    let hr = Rect::from_min_max(page.min + vec2(r[0] * k, r[1] * k), page.min + vec2(r[2] * k, r[3] * k)).expand(1.0);
                    painter.rect_filled(hr, CornerRadius::same(2), if *is_current { current } else { all });
                }
            }
        }
    });
}

// ----- Word and PowerPoint -------------------------------------------------------------------

/// A Word or PowerPoint file laid out for one width, zoom and theme.
pub struct Layout {
    key: (u32, u32, bool),
    items: Vec<Item>,
    height: f32,
}

enum Item {
    Text { rect: Rect, galley: Arc<Galley> },
    Picture { rect: Rect, ix: usize },
    Fill { rect: Rect, color: Color32, radius: u8, stroke: Stroke },
    Line { rect: Rect, stroke: Stroke },
}

impl Item {
    fn rect(&self) -> Rect {
        match self {
            Item::Text { rect, .. } | Item::Picture { rect, .. } | Item::Fill { rect, .. } | Item::Line { rect, .. } => *rect,
        }
    }
}

struct Builder<'a> {
    ui: &'a Ui,
    p: style::Palette,
    zoom: f32,
    pictures: &'a [Picture],
    items: Vec<Item>,
}

/// How a paragraph style reads: size (before zoom), bold, colour, italic, monospace.
fn style_of(s: ParaStyle, p: &style::Palette, base: f32) -> (f32, bool, Color32, bool, bool) {
    match s {
        ParaStyle::Title => (base * 1.9, true, p.text, false, false),
        ParaStyle::Subtitle => (base * 1.3, false, p.text_muted, false, false),
        ParaStyle::Heading(1) => (base * 1.6, true, p.text, false, false),
        ParaStyle::Heading(2) => (base * 1.35, true, p.text, false, false),
        ParaStyle::Heading(3) => (base * 1.18, true, p.text, false, false),
        ParaStyle::Heading(_) => (base * 1.05, true, p.text, false, false),
        ParaStyle::Quote => (base, false, p.text_muted, true, false),
        ParaStyle::Code => (base * 0.9, false, p.text, false, true),
        ParaStyle::Normal => (base, false, p.text, false, false),
    }
}

/// A colour from the file, when it reads on this theme (black text on a dark theme does not).
fn usable(c: [u8; 3], dark: bool) -> Option<Color32> {
    let luma = 0.299 * c[0] as f32 + 0.587 * c[1] as f32 + 0.114 * c[2] as f32;
    let spread = c.iter().max().copied().unwrap_or(0) as i32 - c.iter().min().copied().unwrap_or(0) as i32;
    // Greys are the document's text colour: ours stands in for them.
    if spread < 40 {
        return None;
    }
    if dark && luma < 70.0 || !dark && luma > 200.0 {
        return None;
    }
    Some(Color32::from_rgb(c[0], c[1], c[2]))
}

impl Builder<'_> {
    fn job(&self, para: &Para, base: f32, wrap: f32) -> egui::text::LayoutJob {
        let dark = self.ui.visuals().dark_mode;
        let (size, bold_all, color, italic_all, mono_all) = style_of(para.style, &self.p, base);
        let mut job = egui::text::LayoutJob::default();
        for span in &para.spans {
            match span {
                Span::Text { text, bold, italic, underline, strike, code, link, size: run_size, color: run_color } => {
                    let sz = match (para.style, run_size) {
                        (ParaStyle::Normal, Some(pt)) => (pt * 1.25 * self.zoom).clamp(9.0 * self.zoom, 40.0 * self.zoom),
                        _ => size,
                    };
                    let font = if *code || mono_all {
                        egui::FontId::monospace(sz * 0.92)
                    } else if *bold || bold_all {
                        style::semibold(sz)
                    } else {
                        egui::FontId::proportional(sz)
                    };
                    let c = if *link {
                        self.p.accent_text
                    } else {
                        run_color.and_then(|c| usable(c, dark)).unwrap_or(color)
                    };
                    let line = Stroke::new(1.0, c);
                    job.append(
                        text,
                        0.0,
                        egui::TextFormat {
                            font_id: font,
                            color: c,
                            italics: *italic || italic_all,
                            underline: if *underline || *link { line } else { Stroke::NONE },
                            strikethrough: if *strike { line } else { Stroke::NONE },
                            ..Default::default()
                        },
                    );
                }
                Span::Tab => job.append("    ", 0.0, egui::TextFormat::simple(egui::FontId::proportional(size), color)),
                Span::Break => job.append("\n", 0.0, egui::TextFormat::simple(egui::FontId::proportional(size), color)),
                Span::Image(..) => {}
            }
        }
        job.wrap.max_width = wrap.max(20.0);
        job.halign = match para.align {
            tc_doc::office::Align::Center => egui::Align::Center,
            tc_doc::office::Align::Right => egui::Align::RIGHT,
            _ => egui::Align::LEFT,
        };
        job
    }

    /// Lays out a paragraph at `x` (width `w`) from `y`; returns the y after it.
    fn para(&mut self, para: &Para, x: f32, w: f32, mut y: f32, base: f32) -> f32 {
        let z = self.zoom;
        let (before, after) = match para.style {
            ParaStyle::Title => (6.0, 10.0),
            ParaStyle::Heading(1) => (18.0, 8.0),
            ParaStyle::Heading(_) => (14.0, 6.0),
            _ => (0.0, 8.0),
        };
        y += before * z;
        let mut x = x;
        let mut w = w;
        if let Some(list) = &para.list {
            let indent = 22.0 * z * (list.level as f32 + 1.0);
            let mark = self.ui.painter().layout_no_wrap(list.mark.clone(), egui::FontId::proportional(base), self.p.text_muted);
            let mr = Rect::from_min_size(pos2(x + indent - mark.size().x - 6.0 * z, y), mark.size());
            self.items.push(Item::Text { rect: mr, galley: mark });
            x += indent;
            w -= indent;
        }
        if para.style == ParaStyle::Quote {
            self.items.push(Item::Fill { rect: Rect::from_min_size(pos2(x, y), vec2(3.0 * z, 1.0)), color: self.p.border_strong, radius: 1, stroke: Stroke::NONE });
            x += 14.0 * z;
            w -= 14.0 * z;
        }
        let has_text = para.spans.iter().any(|s| matches!(s, Span::Text { text, .. } if !text.trim().is_empty()) || matches!(s, Span::Tab | Span::Break));
        let start = y;
        if has_text {
            let job = self.job(para, base, w);
            let galley = self.ui.painter().layout_job(job);
            let size = galley.size();
            let gx = match para.align {
                tc_doc::office::Align::Center => x + w / 2.0,
                tc_doc::office::Align::Right => x + w,
                _ => x,
            };
            if para.style == ParaStyle::Code {
                self.items.push(Item::Fill {
                    rect: Rect::from_min_size(pos2(x - 8.0 * z, y - 4.0 * z), vec2(w + 16.0 * z, size.y + 8.0 * z)),
                    color: self.p.subtle,
                    radius: 4,
                    stroke: Stroke::NONE,
                });
            }
            self.items.push(Item::Text { rect: Rect::from_min_size(pos2(gx, y), size), galley });
            y += size.y;
        } else if !para.spans.iter().any(|s| matches!(s, Span::Image(..))) {
            // An empty paragraph: a blank line.
            y += base * 0.9;
        }
        for span in &para.spans {
            if let Span::Image(ix, size_pt) = span
                && let Some(pic) = self.pictures.get(*ix)
            {
                let natural = size_pt.map_or(pic.width as f32 * 0.75, |s| s.0 * 1.33) * z;
                let pw = natural.min(w).max(8.0);
                let ph = pw * pic.height as f32 / pic.width.max(1) as f32;
                let px = match para.align {
                    tc_doc::office::Align::Center => x + (w - pw) / 2.0,
                    tc_doc::office::Align::Right => x + w - pw,
                    _ => x,
                };
                y += 4.0 * z;
                self.items.push(Item::Picture { rect: Rect::from_min_size(pos2(px, y), vec2(pw, ph)), ix: *ix });
                y += ph + 4.0 * z;
            }
        }
        if para.style == ParaStyle::Quote
            && let Some(Item::Fill { rect, .. }) = self.items.iter_mut().rev().find(|i| matches!(i, Item::Fill { radius: 1, .. }))
        {
            rect.max.y = rect.min.y + (y - start).max(4.0);
        }
        y + after * z
    }

    fn table(&mut self, rows: &[Vec<Vec<Para>>], x: f32, w: f32, mut y: f32, base: f32) -> f32 {
        let z = self.zoom;
        let cols = rows.iter().map(Vec::len).max().unwrap_or(0);
        if cols == 0 {
            return y;
        }
        let pad = 8.0 * z;
        // Columns as wide as their text wants, shrunk together when the table is too wide.
        let mut natural = vec![40.0 * z; cols];
        for row in rows {
            for (c, cell) in row.iter().enumerate() {
                for para in cell {
                    let g = self.ui.painter().layout_no_wrap(para.text(), egui::FontId::proportional(base), self.p.text);
                    natural[c] = natural[c].max((g.size().x + pad * 2.0).min(w * 0.7));
                }
            }
        }
        let sum: f32 = natural.iter().sum();
        let widths: Vec<f32> = if sum <= w { natural } else { natural.iter().map(|n| (n / sum * w).max(48.0 * z)).collect() };
        let table_w: f32 = widths.iter().sum();
        let top = y;
        let stroke = Stroke::new(1.0, self.p.border_strong);
        for (ri, row) in rows.iter().enumerate() {
            let mut heights = Vec::new();
            let mut cell_items = Vec::new();
            let mut cx = x;
            for (c, width) in widths.iter().enumerate() {
                let mut cy = y + pad;
                let saved = std::mem::take(&mut self.items);
                if let Some(cell) = row.get(c) {
                    for para in cell {
                        cy = self.para(para, cx + pad, width - pad * 2.0, cy, base) - 6.0 * z;
                    }
                }
                cell_items.extend(std::mem::replace(&mut self.items, saved));
                heights.push(cy - y + pad);
                cx += width;
            }
            let row_h = heights.iter().copied().fold(base * 1.6, f32::max);
            if ri == 0 {
                self.items.push(Item::Fill { rect: Rect::from_min_size(pos2(x, y), vec2(table_w, row_h)), color: self.p.subtle, radius: 0, stroke: Stroke::NONE });
            }
            self.items.extend(cell_items);
            y += row_h;
            self.items.push(Item::Line { rect: Rect::from_min_max(pos2(x, y), pos2(x + table_w, y)), stroke });
        }
        // Column lines and the frame.
        let mut cx = x;
        for width in &widths[..widths.len() - 1] {
            cx += width;
            self.items.push(Item::Line { rect: Rect::from_min_max(pos2(cx, top), pos2(cx, y)), stroke });
        }
        self.items.push(Item::Fill { rect: Rect::from_min_max(pos2(x, top), pos2(x + table_w, y)), color: Color32::TRANSPARENT, radius: 0, stroke });
        y + 12.0 * z
    }

    fn blocks(&mut self, blocks: &[Block], x: f32, w: f32, mut y: f32, base: f32) -> f32 {
        for b in blocks {
            y = match b {
                Block::Para(para) => self.para(para, x, w, y, base),
                Block::Table(rows) => self.table(rows, x, w, y, base),
                Block::PageBreak => {
                    let ly = y + 10.0 * self.zoom;
                    self.items.push(Item::Line { rect: Rect::from_min_max(pos2(x, ly), pos2(x + w, ly)), stroke: Stroke::new(1.0, self.p.border) });
                    ly + 14.0 * self.zoom
                }
            };
        }
        y
    }
}

fn build_layout(ui: &Ui, content: &Content, width: f32, zoom: f32, key: (u32, u32, bool)) -> Layout {
    let p = style::pal(ui);
    let margin = 28.0;
    match content {
        Content::Text(doc) => {
            let col = (width - margin * 2.0).min(760.0 * zoom.max(0.6)).max(160.0);
            let x = ((width - col) / 2.0).max(margin);
            let mut b = Builder { ui, p, zoom, pictures: &doc.images, items: Vec::new() };
            let y = b.blocks(&doc.blocks, x, col, margin, 14.5 * zoom);
            Layout { key, items: b.items, height: y + margin }
        }
        Content::Slides(deck) => {
            let card_w = (width - margin * 2.0).min(960.0 * zoom).max(200.0);
            let x = ((width - card_w) / 2.0).max(margin);
            // Text sized for the card, as on the slide.
            let s = card_w / 900.0;
            let mut b = Builder { ui, p, zoom: s, pictures: &deck.images, items: Vec::new() };
            let mut y = margin;
            for (i, slide) in deck.slides.iter().enumerate() {
                let label = ui.painter().layout_no_wrap(tf("Slide {}", &[&(i + 1).to_string()]), egui::FontId::proportional(style::SMALL), p.text_faint);
                b.items.push(Item::Text { rect: Rect::from_min_size(pos2(x, y), label.size()), galley: label });
                y += 20.0;
                let card_top = y;
                let card_index = b.items.len();
                b.items.push(Item::Fill { rect: Rect::NOTHING, color: p.surface, radius: 8, stroke: Stroke::new(1.0, p.border) });
                let pad = 36.0 * s;
                let mut cy = card_top + pad;
                if let Some(title) = &slide.title {
                    let g = ui.painter().layout(title.clone(), style::semibold(30.0 * s), p.text, card_w - pad * 2.0);
                    b.items.push(Item::Text { rect: Rect::from_min_size(pos2(x + pad, cy), g.size()), galley: g });
                    cy += b.items.last().map_or(0.0, |i| i.rect().height()) + 18.0 * s;
                }
                cy = b.blocks(&slide.blocks, x + pad, card_w - pad * 2.0, cy, 19.0 * s);
                let h = (card_w / deck.aspect.max(0.3)).max(cy - card_top + pad);
                if let Item::Fill { rect, .. } = &mut b.items[card_index] {
                    *rect = Rect::from_min_size(pos2(x, card_top), vec2(card_w, h));
                }
                y = card_top + h + 28.0;
            }
            Layout { key, items: b.items, height: y + margin }
        }
        _ => Layout { key, items: Vec::new(), height: 0.0 },
    }
}

fn rich_body(ui: &mut Ui, view: &mut DocView, rect: Rect, p: &style::Palette) {
    let width = ((rect.width() - 10.0) / 8.0).floor() * 8.0;
    let key = (width.to_bits(), view.zoom.to_bits(), ui.visuals().dark_mode);
    if view.layout.as_ref().is_none_or(|l| l.key != key)
        && let State::Ready(content) = &view.state
    {
        view.layout = Some(build_layout(ui, content, width, view.zoom, key));
    }
    let ctx = ui.ctx().clone();
    let mut wanted = Vec::new();
    let mut shown = HashSet::new();
    let (all, current) = match_colors();
    let cur = view.find.as_ref().and_then(|f| f.current);
    let text_matches: Vec<(usize, usize, usize, bool)> = view
        .find
        .as_ref()
        .map(|f| {
            f.matches
                .iter()
                .enumerate()
                .filter_map(|(i, m)| match m {
                    DocMatch::Text { item, start, end } => Some((*item, *start, *end, Some(i) == cur)),
                    _ => None,
                })
                .collect()
        })
        .unwrap_or_default();
    let mut area = egui::ScrollArea::vertical().id_salt(("doc", view.path.clone())).auto_shrink([false, false]);
    // The search's current match, a third down the view.
    if let Some(f) = view.find.as_mut().filter(|f| f.jump)
        && let Some(DocMatch::Text { item, start, .. }) = f.current.and_then(|c| f.matches.get(c)).cloned()
        && let Some(Item::Text { rect: ir, galley }) = view.layout.as_ref().and_then(|l| l.items.get(item))
    {
        f.jump = false;
        let y = ir.min.y + galley.pos_from_cursor(egui::text::CCursor::new(start)).min.y;
        area = area.vertical_scroll_offset((y - rect.height() / 3.0).max(0.0));
    }
    area.show_viewport(ui, |ui, vp| {
            let Some(layout) = view.layout.as_ref() else { return };
            let (r, _) = ui.allocate_exact_size(vec2(rect.width(), layout.height), Sense::hover());
            let o = r.min.to_vec2();
            let painter = ui.painter();
            for (ix, item) in layout.items.iter().enumerate() {
                let ir = item.rect();
                if ir.max.y < vp.min.y || ir.min.y > vp.max.y {
                    continue;
                }
                match item {
                    Item::Text { rect, galley } => {
                        for (_, start, end, is_current) in text_matches.iter().filter(|m| m.0 == ix) {
                            for b in char_boxes(galley, *start, *end) {
                                painter.rect_filled(b.translate(rect.min.to_vec2() + o).expand(1.0), CornerRadius::same(2), if *is_current { current } else { all });
                            }
                        }
                        painter.galley(rect.min + o, galley.clone(), p.text);
                    }
                    Item::Fill { rect, color, radius, stroke } => {
                        painter.rect(rect.translate(o), CornerRadius::same(*radius), *color, *stroke, egui::StrokeKind::Inside);
                    }
                    Item::Line { rect, stroke } => {
                        painter.line_segment([rect.min + o, rect.max + o], *stroke);
                    }
                    Item::Picture { rect, ix } => {
                        let pr = rect.translate(o);
                        shown.insert(*ix);
                        match view.pictures.get(ix) {
                            Some(Pic::Ready(tex)) => {
                                painter.image(tex.id(), pr, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
                            }
                            other => {
                                if other.is_none() {
                                    wanted.push(*ix);
                                }
                                painter.rect_filled(pr, CornerRadius::same(4), p.subtle);
                                painter.text(pr.center(), Align2::CENTER_CENTER, icons::IMAGE, style::icon_font(18.0), p.text_faint);
                            }
                        }
                    }
                }
            }
        });
    for ix in wanted {
        view.want_picture(ix, &ctx);
    }
    view.keep_pictures(&shown);
}

// ----- sheets --------------------------------------------------------------------------------

fn cell_name(sheet: &tc_doc::sheet::Sheet, row: usize, col: usize) -> String {
    format!(
        "{}{}",
        tc_doc::sheet::column_name(sheet.origin.1 + col as u32),
        sheet.origin.0 as usize + row + 1
    )
}

fn sheet_body(ui: &mut Ui, view: &mut DocView, rect: Rect, p: &style::Palette) {
    let State::Ready(Content::Sheet(book)) = &view.state else { return };
    let multi = book.sheets.len() > 1;
    let names: Vec<String> = book.sheets.iter().map(|s| s.name.clone()).collect();
    let tabs = Rect::from_min_max(pos2(rect.left(), rect.bottom() - TABS_H), rect.max);
    let grid_rect = Rect::from_min_max(pos2(rect.left(), rect.top() + CELL_BAR_H), if multi { pos2(rect.right(), tabs.top()) } else { rect.max });
    let bar = Rect::from_min_max(rect.min, pos2(rect.right(), rect.top() + CELL_BAR_H));
    let Some(sheet) = book.sheets.get(view.sheet) else { return };
    let z = view.zoom;
    let fs = 12.5 * z;
    let row_h = (22.0 * z).round().max(12.0);
    let char_w = fs * 0.56;
    let widths: Vec<f32> = sheet.widths.iter().map(|w| (*w as f32 * char_w + 18.0 * z).clamp(46.0 * z, 380.0 * z).round()).collect();
    let mut xs = Vec::with_capacity(widths.len() + 1);
    let mut acc = 0.0;
    xs.push(0.0);
    for w in &widths {
        acc += w;
        xs.push(acc);
    }
    let last_row = sheet.origin.0 as usize + sheet.rows;
    let rn_w = (last_row.max(1).to_string().len() as f32 * char_w + 20.0 * z).max(36.0 * z).round();
    let head_h = row_h;
    let font = egui::FontId::proportional(fs);

    // The selected cell's name and whole text.
    ui.painter().rect_filled(bar, CornerRadius::ZERO, p.canvas);
    ui.painter().hline(bar.x_range(), bar.bottom() - 0.5, Stroke::new(1.0, p.border));
    if let Some((r, c)) = view.cell {
        let name = cell_name(sheet, r, c);
        let nr = Rect::from_min_size(pos2(bar.left() + 10.0, bar.top() + 5.0), vec2(64.0, bar.height() - 10.0));
        ui.painter().rect(nr, CornerRadius::same(4), p.surface, Stroke::new(1.0, p.border), egui::StrokeKind::Inside);
        ui.painter().text(nr.center(), Align2::CENTER_CENTER, name, style::medium(style::SMALL), p.text_muted);
        let text = sheet.cell(r, c).text().replace('\n', " ⏎ ");
        let mut job = egui::text::LayoutJob::simple_singleline(text, egui::FontId::proportional(style::BODY - 1.0), p.text);
        job.wrap = egui::text::TextWrapping::truncate_at_width((bar.right() - nr.right() - 24.0).max(10.0));
        let g = ui.painter().layout_job(job);
        ui.painter().galley(pos2(nr.right() + 12.0, bar.center().y - g.size().y / 2.0), g, p.text);
    } else {
        ui.painter().text(pos2(bar.left() + 12.0, bar.center().y), Align2::LEFT_CENTER, t("Click a cell to see all of its text."), egui::FontId::proportional(style::SMALL), p.text_faint);
    }
    if let Some(e) = &sheet.error {
        ui.painter().text(grid_rect.center(), Align2::CENTER_CENTER, e, egui::FontId::proportional(style::BODY), p.err);
    }

    let total = vec2(rn_w + acc, head_h + sheet.rows as f32 * row_h);
    let mut select = None;
    let mut keys_moved = None;
    ui.scope_builder(egui::UiBuilder::new().max_rect(grid_rect), |ui| {
        ui.set_clip_rect(grid_rect);
        let salt = ("sheet", view.path.clone(), view.sheet);
        let mut area = egui::ScrollArea::both().id_salt(&salt).auto_shrink([false, false]);
        if std::mem::take(&mut view.scroll_to_cell) {
            // Each sheet scrolls on its own: from this one's offset, not that of the sheet shown
            // last (a match on another sheet may have just switched to this one).
            let id = ui.make_persistent_id(egui::IdSalt::new(&salt));
            view.sheet_scroll = egui::scroll_area::State::load(ui.ctx(), id).map_or(egui::Vec2::ZERO, |s| s.offset);
            if let Some(target) = scroll_target(view.cell, view.sheet_scroll, &xs, row_h, head_h, rn_w, grid_rect) {
                area = area.scroll_offset(target);
            }
        }
        area.show_viewport(ui, |ui, vp| {
            let (r, resp) = ui.allocate_exact_size(total.max(vp.size()), Sense::click());
            let o = r.min;
            let painter = ui.painter();
            let row1 = ((vp.max.y / row_h).ceil() as usize).min(sheet.rows);
            // The offset is clamped only at the frame's end: zoomed out at the bottom of a long
            // sheet, the view can start below the last row for a frame.
            let row0 = ((((vp.min.y - head_h) / row_h).floor().max(0.0)) as usize).min(row1);
            let col0 = xs.partition_point(|x| *x <= vp.min.x - rn_w).saturating_sub(1);
            let col1 = xs.partition_point(|x| *x < vp.max.x - rn_w).min(widths.len());
            let grid = Stroke::new(1.0, style::soft(p.border, 0.9));
            let (all, current) = match_colors();
            let cur = view.find.as_ref().and_then(|f| f.current);
            let hits: std::collections::HashMap<(usize, usize), bool> = view
                .find
                .as_ref()
                .map(|f| {
                    f.matches
                        .iter()
                        .enumerate()
                        .filter_map(|(i, m)| match m {
                            DocMatch::Cell { sheet, row, col } if *sheet == view.sheet => Some(((*row, *col), Some(i) == cur)),
                            _ => None,
                        })
                        .collect()
                })
                .unwrap_or_default();
            for row in row0..row1 {
                let y = o.y + head_h + row as f32 * row_h;
                painter.hline(o.x + rn_w + xs[col0]..=o.x + rn_w + xs[col1], y + row_h - 0.5, grid);
                for col in col0..col1 {
                    let cr = Rect::from_min_size(pos2(o.x + rn_w + xs[col], y), vec2(widths[col], row_h));
                    if let Some(is_current) = hits.get(&(row, col)) {
                        painter.rect_filled(cr.shrink(1.0), CornerRadius::ZERO, if *is_current { current } else { all });
                    }
                    let cell = sheet.cell(row, col);
                    let text = cell.text();
                    if text.is_empty() {
                        continue;
                    }
                    let (color, right) = match cell {
                        tc_doc::sheet::Cell::Number(_) => (p.text, true),
                        tc_doc::sheet::Cell::Bool(_) => (p.accent_text, true),
                        tc_doc::sheet::Cell::Error(_) => (p.err, false),
                        _ => (p.text, false),
                    };
                    let mut job = egui::text::LayoutJob::simple_singleline(text.lines().next().unwrap_or("").to_string(), font.clone(), color);
                    job.wrap = egui::text::TextWrapping::truncate_at_width((cr.width() - 12.0 * z).max(4.0));
                    let g = painter.layout_job(job);
                    let gx = if right { cr.right() - 6.0 * z - g.size().x } else { cr.left() + 6.0 * z };
                    painter.galley(pos2(gx, cr.center().y - g.size().y / 2.0), g, color);
                }
            }
            for x in &xs[col0..=col1.min(widths.len())] {
                let x = o.x + rn_w + x - 0.5;
                painter.vline(x, o.y + head_h + row0 as f32 * row_h..=o.y + head_h + row1 as f32 * row_h, grid);
            }
            // The selected cell.
            if let Some((sr, sc)) = view.cell
                && sc < widths.len()
            {
                let cr = Rect::from_min_size(pos2(o.x + rn_w + xs[sc], o.y + head_h + sr as f32 * row_h), vec2(widths[sc], row_h));
                painter.rect_filled(cr, CornerRadius::ZERO, style::soft(p.accent, 0.12));
                painter.rect_stroke(cr, CornerRadius::ZERO, Stroke::new(2.0, p.accent), egui::StrokeKind::Inside);
            }
            // The header row and the row numbers stay in view.
            let top = o.y + vp.min.y;
            let left = o.x + vp.min.x;
            let head = Rect::from_min_size(pos2(o.x + rn_w + xs[col0], top), vec2(xs[col1] - xs[col0], head_h));
            painter.rect_filled(head, CornerRadius::ZERO, p.chrome);
            for col in col0..col1 {
                let cr = Rect::from_min_size(pos2(o.x + rn_w + xs[col], top), vec2(widths[col], head_h));
                let on = view.cell.is_some_and(|(_, c)| c == col);
                if on {
                    painter.rect_filled(cr, CornerRadius::ZERO, style::soft(p.accent, 0.18));
                }
                painter.text(cr.center(), Align2::CENTER_CENTER, tc_doc::sheet::column_name(sheet.origin.1 + col as u32), style::medium(fs * 0.92), if on { p.accent_text } else { p.text_muted });
                painter.vline(cr.right() - 0.5, cr.y_range(), grid);
            }
            painter.hline(head.x_range(), head.bottom() - 0.5, Stroke::new(1.0, p.border_strong));
            let nums = Rect::from_min_size(pos2(left, o.y + head_h + row0 as f32 * row_h), vec2(rn_w, (row1 - row0) as f32 * row_h));
            painter.rect_filled(nums, CornerRadius::ZERO, p.chrome);
            for row in row0..row1 {
                let y = o.y + head_h + row as f32 * row_h;
                let rr = Rect::from_min_size(pos2(left, y), vec2(rn_w, row_h));
                if y + row_h < top + head_h {
                    continue;
                }
                let on = view.cell.is_some_and(|(r, _)| r == row);
                if on {
                    painter.rect_filled(rr, CornerRadius::ZERO, style::soft(p.accent, 0.18));
                }
                painter.text(pos2(rr.right() - 8.0 * z, rr.center().y), Align2::RIGHT_CENTER, (sheet.origin.0 as usize + row + 1).to_string(), egui::FontId::proportional(fs * 0.9), if on { p.accent_text } else { p.text_faint });
                painter.hline(rr.x_range(), rr.bottom() - 0.5, grid);
            }
            painter.vline(left + rn_w - 0.5, nums.y_range(), Stroke::new(1.0, p.border_strong));
            painter.rect_filled(Rect::from_min_size(pos2(left, top), vec2(rn_w, head_h)), CornerRadius::ZERO, p.chrome);
            // A click picks the cell under it; the arrows move from there.
            if resp.clicked()
                && let Some(pos) = resp.interact_pointer_pos()
            {
                let (x, y) = (pos.x - o.x - rn_w, pos.y - o.y - head_h);
                if x >= 0.0 && y >= 0.0 {
                    let row = (y / row_h) as usize;
                    let col = xs.partition_point(|v| *v <= x).saturating_sub(1);
                    if row < sheet.rows && col < widths.len() {
                        select = Some((row, col));
                    }
                }
                resp.request_focus();
            }
            // ⌘C (the menu asks for egui's copy): the selected cell's text.
            if resp.has_focus()
                && let Some((r, c)) = view.cell
                && ui.input(|i| i.events.iter().any(|e| matches!(e, egui::Event::Copy)))
            {
                ui.ctx().copy_text(sheet.cell(r, c).text().to_string());
            }
            if resp.has_focus() {
                let (rows, cols) = (sheet.rows, widths.len());
                let moved = ui.input_mut(|i| {
                    let mut d = (0i64, 0i64);
                    for (k, dd) in [(egui::Key::ArrowUp, (-1, 0)), (egui::Key::ArrowDown, (1, 0)), (egui::Key::ArrowLeft, (0, -1)), (egui::Key::ArrowRight, (0, 1))] {
                        if i.consume_key(egui::Modifiers::NONE, k) {
                            d = dd;
                        }
                    }
                    d
                });
                if moved != (0, 0) && rows > 0 && cols > 0 {
                    let (r, c) = view.cell.unwrap_or((0, 0));
                    let nr = (r as i64 + moved.0).clamp(0, rows as i64 - 1) as usize;
                    let nc = (c as i64 + moved.1).clamp(0, cols as i64 - 1) as usize;
                    keys_moved = Some((nr, nc));
                }
            }
        });
    });
    if let Some(cell) = select {
        view.cell = Some(cell);
    }
    if let Some(cell) = keys_moved {
        view.cell = Some(cell);
        view.scroll_to_cell = true;
    }
    if multi {
        let p = style::pal(ui);
        ui.painter().rect_filled(tabs, CornerRadius::ZERO, p.chrome);
        ui.painter().hline(tabs.x_range(), tabs.top() + 0.5, Stroke::new(1.0, p.border));
        let mut pick = None;
        ui.scope_builder(egui::UiBuilder::new().max_rect(tabs.shrink2(vec2(6.0, 0.0))), |ui| {
            egui::ScrollArea::horizontal()
                .id_salt(("sheet-tabs", view.path.clone()))
                .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden)
                .show(ui, |ui| {
                    ui.horizontal_centered(|ui| {
                        ui.spacing_mut().item_spacing.x = 2.0;
                        for (i, name) in names.iter().enumerate() {
                            let on = i == view.sheet;
                            if kit::button_full(ui, kit::Kind::Ghost, Some(icons::TABLE), name, true, on).clicked() {
                                pick = Some(i);
                            }
                        }
                    });
                });
        });
        if let Some(i) = pick
            && i != view.sheet
        {
            view.sheet = i;
            view.cell = None;
        }
    }
}

/// Where to scroll (from `at`) so the cell the arrows moved to is in view.
fn scroll_target(cell: Option<(usize, usize)>, at: egui::Vec2, xs: &[f32], row_h: f32, head_h: f32, rn_w: f32, area: Rect) -> Option<egui::Vec2> {
    let (r, c) = cell?;
    let (x0, x1) = (xs.get(c).copied()?, xs.get(c + 1).copied()?);
    let (y0, y1) = (r as f32 * row_h, (r + 1) as f32 * row_h);
    let (vw, vh) = (area.width() - rn_w - 12.0, area.height() - head_h - 12.0);
    let mut out = at;
    if x0 < at.x {
        out.x = x0;
    } else if x1 > at.x + vw {
        out.x = x1 - vw;
    }
    if y0 < at.y {
        out.y = y0;
    } else if y1 > at.y + vh {
        out.y = y1 - vh;
    }
    (out != at).then_some(out)
}

// ----- images --------------------------------------------------------------------------------

fn image_body(ui: &mut Ui, view: &mut DocView, rect: Rect, p: &style::Palette) {
    let Some((iw, ih)) = view.image_size else { return };
    let Some(Pic::Ready(tex)) = view.pictures.get(&0) else { return };
    let ppp = ui.ctx().pixels_per_point();
    let margin = 24.0;
    // Pixel for pixel when it fits, else fitted; then the zoom.
    let natural = vec2(iw as f32 / ppp, ih as f32 / ppp);
    let fit = ((rect.width() - margin * 2.0) / natural.x).min((rect.height() - margin * 2.0) / natural.y).min(1.0);
    let size = natural * fit * view.zoom;
    let tex = tex.id();
    egui::ScrollArea::both()
        .id_salt(("image", view.path.clone()))
        .auto_shrink([false, false])
        .show_viewport(ui, |ui, vp| {
            let total = vec2(size.x + margin * 2.0, size.y + margin * 2.0).max(vp.size());
            let (r, _) = ui.allocate_exact_size(total, Sense::hover());
            let img = Rect::from_center_size(r.center(), size);
            // A checkerboard shows through transparent parts.
            let cell = 10.0;
            let painter = ui.painter_at(img);
            painter.rect_filled(img, CornerRadius::ZERO, Color32::from_gray(if ui.visuals().dark_mode { 60 } else { 235 }));
            let shown = img.intersect(vp.translate(r.min.to_vec2()));
            let dark_cell = Color32::from_gray(if ui.visuals().dark_mode { 75 } else { 215 });
            let (cx0, cy0) = (((shown.left() - img.left()) / cell).floor() as i32, ((shown.top() - img.top()) / cell).floor() as i32);
            let (cx1, cy1) = (((shown.right() - img.left()) / cell).ceil() as i32, ((shown.bottom() - img.top()) / cell).ceil() as i32);
            if (cx1 - cx0) * (cy1 - cy0) < 40_000 {
                for y in cy0..cy1 {
                    for x in cx0..cx1 {
                        if (x + y) % 2 == 0 {
                            let c = Rect::from_min_size(pos2(img.left() + x as f32 * cell, img.top() + y as f32 * cell), vec2(cell, cell));
                            painter.rect_filled(c, CornerRadius::ZERO, dark_cell);
                        }
                    }
                }
            }
            painter.image(tex, img, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
            ui.painter().rect_stroke(img, CornerRadius::ZERO, Stroke::new(1.0, p.border), egui::StrokeKind::Outside);
        });
}
