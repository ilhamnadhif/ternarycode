//! Documents in the editor: one toolbar (where you are at the left; zoom, find, open in the
//! default app and show in the folder at the right), the document, and a status line. PDF
//! pages are pictures rendered for the zoom shown; Word and PowerPoint files are laid out once
//! for the width shown and only what is on screen is drawn; sheets are a grid with its header
//! row and row numbers kept in view.

use std::collections::HashSet;
use std::sync::Arc;

use eframe::egui::{self, Align2, Color32, CornerRadius, Galley, Pos2, Rect, Sense, Stroke, Ui, pos2, vec2};
use tc_core::ProjectId;
use tc_doc::office::{Block, Para, ParaStyle, Picture, Span};

use super::{icons, kit, style};
use crate::app::App;
use crate::docview::{Content, DocMatch, DocView, MAX_DOC_MATCHES, Pic, State};
use crate::i18n::{t, tf};

const BAR_H: f32 = 40.0;
const STATUS_H: f32 = 28.0;
const TABS_H: f32 = 36.0;
const CELL_BAR_H: f32 = 40.0;
/// The toolbar's small icon buttons.
const ICON_S: f32 = 24.0;
const ZOOMS: [f32; 13] = [0.5, 0.67, 0.8, 0.9, 1.0, 1.1, 1.25, 1.5, 1.75, 2.0, 2.5, 3.0, 4.0];

/// What a click in the document's toolbar asks of the app.
enum Ask {
    OpenDefault,
    Reveal,
    EditAsText,
}

impl App {
    /// The editor's active file is a document: its toolbar, the document and a status line (a
    /// workbook of several sheets: its sheets' tabs), the find bar over the document's corner.
    pub fn doc_panel(&mut self, ui: &mut Ui, project: ProjectId, body: Rect) {
        let ctx = ui.ctx().clone();
        let p = style::pal(ui);
        let root = self.project(project).map(|x| x.path.clone()).unwrap_or_default();
        let allow_keys = !self.overlay_open() && !self.updater.dialog_open;
        let Some(view) = self
            .editors
            .get_mut(&project)
            .and_then(|e| e.file_mut())
            .and_then(|f| f.doc.as_deref_mut())
        else {
            return;
        };
        view.poll(&ctx);
        // A workbook of several sheets says what it is beside their tabs, at its foot.
        let sheets = matches!(&view.state, State::Ready(Content::Sheet(b)) if b.sheets.len() > 1);
        let bar = Rect::from_min_size(body.min, vec2(body.width(), BAR_H));
        let status = Rect::from_min_max(pos2(body.left(), body.bottom() - if sheets { 0.0 } else { STATUS_H }), body.max);
        let content = Rect::from_min_max(pos2(body.left(), bar.bottom()), pos2(body.right(), status.top()));
        // A click in the document gives it the keyboard (a terminal takes the keyboard back
        // whenever nobody has it): ⌘F then searches here and ⌘W closes the document. Widgets
        // inside (a sheet's grid) take it from there.
        let doc_id = egui::Id::new(("doc-view", project));
        let doc_area = ui.interact(content, doc_id, Sense::click());
        if allow_keys && ui.input(|i| i.pointer.primary_pressed()) && ui.rect_contains_pointer(content) {
            doc_area.request_focus();
        }
        if doc_area.has_focus() && allow_keys {
            self.editor_seen = true;
            // ← → move through a deck's slides.
            if let State::Ready(Content::Slides(deck)) = &view.state {
                ui.memory_mut(|m| m.set_focus_lock_filter(doc_id, egui::EventFilter { horizontal_arrows: true, ..Default::default() }));
                let n = deck.slides.len();
                let (back, on) = ui.input_mut(|i| {
                    (
                        i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowLeft),
                        i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowRight),
                    )
                });
                if n > 0 && (back || on) {
                    let at = view.slide.min(n - 1);
                    view.slide_jump = Some(if on { (at + 1).min(n - 1) } else { at.saturating_sub(1) });
                }
            }
        }
        #[cfg(debug_assertions)]
        crate::devtest::record("doc-body", content);
        let mut ask = None;
        toolbar(ui, view, bar, &root, &mut ask);
        if !sheets {
            status_line(ui, view, status);
        }
        if view.find.is_some() {
            refresh_find(view);
            follow_sheet_match(view);
        }
        let mut sheet_focused = false;
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
                State::Ready(Content::Pdf) => {
                    // Pages lie on the sunken colour.
                    ui.painter().rect_filled(content, CornerRadius::ZERO, p.inset);
                    pdf_body(ui, view, content);
                }
                State::Ready(Content::Text(_) | Content::Slides(_)) => rich_body(ui, view, content, &p),
                State::Ready(Content::Sheet(_)) => sheet_focused = sheet_body(ui, view, content, &p),
                State::Ready(Content::Image) => image_body(ui, view, content, &p),
            }
        });
        // The find bar floats at the document's top right (under a sheet's cell bar).
        if view.find.is_some() {
            let top = content.top() + 8.0 + if matches!(view.state, State::Ready(Content::Sheet(_))) { CELL_BAR_H } else { 0.0 };
            if find_bar(ui, view, content, top, project, allow_keys) {
                // The search field has the keyboard: the editor's keys (⌘W…) apply.
                self.editor_seen = true;
            }
        }
        let path = view.path.clone();
        if sheet_focused {
            self.editor_seen = true;
        }
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

// ----- selecting text ------------------------------------------------------------------------

/// A PDF's text is selected (noted while it is drawn, cleared after each frame's actions).
const DOC_SELECTION: &str = "doc-selection";

fn set_doc_selection(ctx: &egui::Context) {
    ctx.data_mut(|d| d.insert_temp(egui::Id::new(DOC_SELECTION), true));
}

/// Text is selected in a document (a PDF's, or a Word or PowerPoint file's): ⌘C copies it.
pub fn has_doc_selection(ctx: &egui::Context) -> bool {
    ctx.data(|d| d.get_temp::<bool>(egui::Id::new(DOC_SELECTION))).unwrap_or(false)
        || ctx.plugin::<egui::text_selection::LabelSelectionState>().lock().has_selection()
}

/// Forgets last frame's note (the PDF notes it again while it is shown with a selection).
pub fn end_doc_selection_frame(ctx: &egui::Context) {
    ctx.data_mut(|d| d.remove::<bool>(egui::Id::new(DOC_SELECTION)));
}

// ----- toolbar and status line ---------------------------------------------------------------

/// The document's toolbar (40): at the left what moves through it (a PDF's pages, a deck's
/// slides: ↑, the number, "of N", ↓) or where it is (its path); at the right − 100% +, find,
/// open in the default app and show in the folder (and Open as Text for a CSV).
fn toolbar(ui: &mut Ui, view: &mut DocView, rect: Rect, root: &std::path::Path, ask: &mut Option<Ask>) {
    let p = style::pal(ui);
    ui.painter().hline(rect.x_range(), rect.bottom() - 0.5, Stroke::new(1.0, p.border));
    let inner = rect.shrink2(vec2(10.0, 0.0));
    let cy = inner.center().y;
    // What the arrows at the left move through: a PDF's pages, or a deck's slides.
    let slides = match &view.state {
        State::Ready(Content::Slides(deck)) => deck.slides.len(),
        _ => 0,
    };
    let pages = view
        .pdf
        .as_ref()
        .map(|p| (p.page, p.pages.len()))
        .or((slides > 0).then_some((view.slide.min(slides.saturating_sub(1)), slides)))
        .filter(|(_, n)| *n > 0);
    // Right side first, from the right edge.
    let mut right = inner.right();
    let mut button = |ui: &mut Ui, icon: &str, tip: &str| {
        let r = Rect::from_min_max(pos2(right - ICON_S, cy - ICON_S / 2.0), pos2(right, cy + ICON_S / 2.0));
        right = r.left() - 6.0;
        ui.scope_builder(egui::UiBuilder::new().max_rect(r), |ui| kit::icon_button_ex(ui, icon, tip, false, ICON_S)).inner.clicked()
    };
    if button(ui, icons::FOLDER_OPEN, reveal_label()) {
        *ask = Some(Ask::Reveal);
    }
    if button(ui, icons::ARROW_SQUARE_OUT, t("Open in default app")) {
        *ask = Some(Ask::OpenDefault);
    }
    let ready = matches!(view.state, State::Ready(_));
    if ready && button(ui, icons::MAGNIFYING_GLASS, &format!("{}   {}", t("Find"), kit::keys(&crate::actions::Action::Find).unwrap_or(""))) {
        view.open_find();
    }
    if ready {
        right = zoom_controls(ui, view, right, cy) - 6.0;
    }
    if view.kind == tc_doc::Kind::Csv {
        let narrow = inner.width() < 420.0;
        let w = if narrow { ICON_S } else { kit::button_width(ui, Some(icons::CODE), t("Open as text"), None, true) };
        let r = Rect::from_min_max(pos2(right - w, cy - 12.0), pos2(right, cy + 12.0));
        right = r.left() - 6.0;
        let clicked = ui
            .scope_builder(egui::UiBuilder::new().max_rect(r), |ui| {
                if narrow {
                    kit::icon_button_ex(ui, icons::CODE, t("Open as text"), false, ICON_S).clicked()
                } else {
                    kit::button_ex(ui, kit::Kind::Ghost, Some(icons::CODE), t("Open as text"), true).clicked()
                }
            })
            .inner;
        if clicked {
            *ask = Some(Ask::EditAsText);
        }
    }
    let left = Rect::from_min_max(pos2(inner.left(), rect.top()), pos2((right - 8.0).max(inner.left()), rect.bottom() - 1.0));
    ui.scope_builder(egui::UiBuilder::new().max_rect(left).layout(egui::Layout::left_to_right(egui::Align::Center)), |ui| {
        ui.set_clip_rect(left.intersect(ui.clip_rect()));
        ui.spacing_mut().item_spacing.x = 6.0;
        if let Some((page, n)) = pages {
            let deck = view.pdf.is_none();
            let jump = |view: &mut DocView, to: usize| match view.pdf.as_mut() {
                Some(pdf) => pdf.jump = Some(to),
                None => view.slide_jump = Some(to),
            };
            let (prev, next) = if deck { (t("Previous Slide"), t("Next Slide")) } else { (t("Previous Page"), t("Next Page")) };
            if kit::icon_button_ex(ui, icons::CARET_UP, prev, false, ICON_S).clicked() {
                jump(view, page.saturating_sub(1));
            }
            // The number can be typed (Enter goes there, Esc leaves it).
            let id = egui::Id::new(("doc-page", view.path.clone()));
            let focused_id = id.with("was-focused");
            let focused = ui.ctx().memory(|m| m.has_focus(id));
            let was_focused = ui.data(|d| d.get_temp::<bool>(focused_id)).unwrap_or(false);
            let mut text: String = ui.data(|d| d.get_temp::<String>(id)).filter(|_| focused || was_focused).unwrap_or_else(|| (page + 1).to_string());
            let (well, _) = ui.allocate_exact_size(vec2(32.0, 24.0), Sense::hover());
            ui.painter().rect(well, CornerRadius::same(5), p.inset, Stroke::new(1.0, if focused { p.accent } else { Color32::TRANSPARENT }), egui::StrokeKind::Inside);
            let r = ui
                .scope_builder(egui::UiBuilder::new().max_rect(well.shrink2(vec2(3.0, 2.0))), |ui| {
                    ui.add_sized(
                        well.shrink2(vec2(3.0, 2.0)).size(),
                        egui::TextEdit::singleline(&mut text)
                            .id(id)
                            .frame(egui::Frame::NONE)
                            .margin(egui::Margin::ZERO)
                            .font(style::mono(style::DATA))
                            .horizontal_align(egui::Align::Center)
                            .vertical_align(egui::Align::Center),
                    )
                })
                .inner;
            ui.data_mut(|d| {
                d.insert_temp(id, text.clone());
                d.insert_temp(focused_id, r.has_focus());
            });
            if r.lost_focus()
                && !ui.input(|i| i.key_pressed(egui::Key::Escape))
                && let Ok(n_typed) = text.trim().parse::<usize>()
            {
                jump(view, n_typed.clamp(1, n) - 1);
            }
            ui.label(egui::RichText::new(tf("of {}", &[&n.to_string()])).size(style::SMALL).color(p.text_faint));
            if kit::icon_button_ex(ui, icons::CARET_DOWN, next, false, ICON_S).clicked() {
                jump(view, (page + 1).min(n - 1));
            }
        } else {
            // The crumb's own room at its left lines its text up with the toolbar's edge; its
            // own line under it falls outside (the toolbar's is there).
            let cell = Rect::from_min_size(ui.cursor().min, vec2(left.width(), rect.height() - 1.0));
            let crumb = Rect::from_min_max(pos2(inner.left() - 12.0, rect.top()), pos2(cell.right(), rect.bottom() + 1.0));
            super::editor_ui::paint_crumb(ui, crumb, root, &view.path, &p);
            ui.allocate_rect(cell, Sense::hover());
        }
    });
}

fn reveal_label() -> &'static str {
    if cfg!(target_os = "macos") { t("Show in Finder") } else { t("Show in Folder") }
}

/// − 100% + in one outlined pill whose right edge is at `right`: the percentage goes back to
/// 100%. Returns its left edge.
fn zoom_controls(ui: &mut Ui, view: &mut DocView, right: f32, cy: f32) -> f32 {
    let p = style::pal(ui);
    let pct = format!("{:.0}%", view.zoom * 100.0);
    let g = ui.painter().layout_no_wrap(pct, style::mono(style::DATA), p.text_muted);
    const B: f32 = 20.0;
    let w = 2.0 + B + 4.0 + g.size().x.max(30.0) + 4.0 + B + 2.0;
    let rect = Rect::from_min_max(pos2(right - w, cy - 12.0), pos2(right, cy + 12.0));
    ui.painter().rect_stroke(rect, CornerRadius::same(style::RADIUS_ROW), Stroke::new(1.0, p.border_strong), egui::StrokeKind::Inside);
    let minus = Rect::from_center_size(pos2(rect.left() + 2.0 + B / 2.0, cy), vec2(B, B));
    let plus = Rect::from_center_size(pos2(rect.right() - 2.0 - B / 2.0, cy), vec2(B, B));
    let mid = Rect::from_min_max(pos2(minus.right() + 2.0, rect.top() + 3.0), pos2(plus.left() - 2.0, rect.bottom() - 3.0));
    let button = |ui: &mut Ui, r: Rect, icon: &str, tip: &str| ui.scope_builder(egui::UiBuilder::new().max_rect(r), |ui| kit::icon_button_ex(ui, icon, tip, false, B)).inner.clicked();
    if button(ui, minus, icons::MINUS, t("Zoom out")) {
        view.zoom = ZOOMS.iter().rev().copied().find(|z| *z < view.zoom - 0.01).unwrap_or(view.zoom);
    }
    if button(ui, plus, icons::PLUS, t("Zoom in")) {
        view.zoom = ZOOMS.iter().copied().find(|z| *z > view.zoom + 0.01).unwrap_or(view.zoom);
    }
    let resp = ui.interact(mid, egui::Id::new(("doc-zoom", "reset")), Sense::click());
    if resp.hovered() {
        ui.painter().rect_filled(mid, CornerRadius::same(4), p.hover);
    }
    let color = if resp.hovered() { p.text } else { p.text_muted };
    ui.painter().galley(pos2(mid.center().x - g.size().x / 2.0, mid.center().y - g.size().y / 2.0), g, color);
    if resp.on_hover_text(t("Actual size")).clicked() {
        view.zoom = 1.0;
    }
    rect.left()
}

/// What the document is ("PDF · 12 pages · 1.4 MB"; an image's format too).
fn summary(ui: &Ui, view: &DocView) -> String {
    let mut s = view.summary();
    if matches!(view.state, State::Ready(Content::Image))
        && let Some(ext) = view.path.extension()
    {
        s = format!("{s} · {}", ext.to_string_lossy().to_uppercase());
    }
    // Its size, read once while it is shown (again after a reload).
    let id = egui::Id::new(("doc-size", view.path.clone()));
    if matches!(view.state, State::Loading) {
        ui.data_mut(|d| d.remove::<u64>(id));
        return s;
    }
    let size = ui.data(|d| d.get_temp::<u64>(id)).or_else(|| {
        let n = std::fs::metadata(&view.path).ok()?.len();
        ui.data_mut(|d| d.insert_temp(id, n));
        Some(n)
    });
    match size {
        Some(n) => format!("{s} · {}", super::update_ui::human_size(n)),
        None => s,
    }
}

/// The status line (28): what the document is in the data font; for a sheet, how to see a
/// cell's whole text, and a warning when it was cut short.
fn status_line(ui: &mut Ui, view: &DocView, rect: Rect) {
    let p = style::pal(ui);
    ui.painter().hline(rect.x_range(), rect.top() + 0.5, Stroke::new(1.0, p.border));
    let mut sum_job = egui::text::LayoutJob::simple_singleline(summary(ui, view), style::mono(style::DATA), p.text_faint);
    sum_job.wrap = egui::text::TextWrapping::truncate_at_width((rect.width() - 24.0).max(20.0));
    let sum_g = ui.painter().layout_job(sum_job);
    let summary = Rect::from_min_size(pos2(rect.left() + 12.0, rect.center().y - sum_g.size().y / 2.0), sum_g.size());
    ui.painter().galley(summary.min, sum_g, p.text_faint);
    // A sheet: how to see a cell's whole text, at the right.
    let hint = matches!(view.state, State::Ready(Content::Sheet(_))).then(|| t("Click a cell to see all of its text."));
    let hint_w = hint.map_or(0.0, |h| {
        let g = ui.painter().layout_no_wrap(h.to_string(), egui::FontId::proportional(style::LABEL), p.text_faint);
        let w = g.size().x;
        if rect.width() - summary.width() - 60.0 > w {
            ui.painter().galley(pos2(rect.right() - 12.0 - w, rect.center().y - g.size().y / 2.0), g, p.text_faint);
            w + 12.0
        } else {
            0.0
        }
    });
    if let Some(w) = truncated(view) {
        paint_truncated(ui, &w, summary.right() + 12.0, rect.right() - 12.0 - hint_w, rect.center().y);
    }
}

/// A sheet cut short (too many cells, or a CSV record that could not be read) would look
/// complete otherwise: the warning, when it was.
fn truncated(view: &DocView) -> Option<String> {
    let State::Ready(Content::Sheet(b)) = &view.state else { return None };
    let s = b.sheets.get(view.sheet).filter(|s| s.truncated)?;
    Some(format!("{}  {}", icons::WARNING, tf("Only the first {} rows are shown.", &[&s.rows.to_string()])))
}

fn paint_truncated(ui: &Ui, text: &str, left: f32, right: f32, cy: f32) {
    let p = style::pal(ui);
    let mut job = egui::text::LayoutJob::simple_singleline(text.to_string(), egui::FontId::proportional(style::LABEL), p.warn);
    job.wrap = egui::text::TextWrapping::truncate_at_width((right - left).max(0.0));
    let g = ui.painter().layout_job(job);
    ui.painter().galley(pos2(left, cy - g.size().y / 2.0), g, p.warn);
}

fn loading(ui: &mut Ui, rect: Rect) {
    let p = style::pal(ui);
    let c = rect.center();
    super::status::paint_spinner(ui.painter(), c - vec2(0.0, 14.0), 20.0, p.text_faint);
    ui.painter().text(c + vec2(0.0, 14.0), Align2::CENTER_CENTER, t("Opening…"), egui::FontId::proportional(style::SMALL), p.text_muted);
}

fn failed(ui: &mut Ui, rect: Rect, error: &str, ask: &mut Option<Ask>) {
    // The reader's reason, as a title and a sentence a person would write.
    let (icon, title, body) = match error {
        e if e.contains("password") => (icons::LOCK_SIMPLE, t("This PDF is password-protected"), t("TernaryCode can’t show protected PDFs. Your default app can.").to_string()),
        e if e.contains("too big") => (icons::FILE, t("This file is too big to show here"), e.to_string()),
        e if e.contains("cannot be shown here") => (icons::FILE, t("This format can’t be shown here"), t("Supported: PDF, Word, PowerPoint, Excel, ODS, CSV/TSV and images.").to_string()),
        e if e.contains("no pages") => (icons::FILE, t("This PDF has no pages"), String::new()),
        e if e.contains("no sheets") => (icons::FILE, t("This workbook has no sheets"), String::new()),
        e => (icons::FILE, t("This document cannot be shown"), {
            let mut c = e.chars();
            c.next().map(|f| f.to_uppercase().collect::<String>() + c.as_str()).unwrap_or_default()
        }),
    };
    ui.scope_builder(egui::UiBuilder::new().max_rect(rect), |ui| {
        kit::empty_state(ui, icon, title, &body, |ui| {
            if kit::button_ex(ui, kit::Kind::Secondary, Some(icons::ARROW_SQUARE_OUT), t("Open in default app"), true).clicked() {
                *ask = Some(Ask::OpenDefault);
            }
        });
    });
}

// ----- search ---------------------------------------------------------------------------------

/// The find bar, floating at the top right of `content` (its top at `top`), as the pages' and
/// the terminals'. Returns whether its field has the keyboard.
fn find_bar(ui: &mut Ui, view: &mut DocView, content: Rect, top: f32, project: ProjectId, allow_keys: bool) -> bool {
    let p = style::pal(ui);
    let Some(f) = view.find.as_mut() else { return false };
    let id = egui::Id::new(("doc-find", project));
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
    let focus = std::mem::take(&mut f.focus);
    let max_w = (content.width() - 16.0).max(120.0);
    let w = super::browser_ui::find_card_width(ui, &info, 140.0).min(max_w);
    let at = pos2((content.right() - 8.0 - w).max(content.left() + 8.0), top);
    let out = super::browser_ui::find_card(
        ui,
        at,
        max_w,
        super::browser_ui::FindCard { id, query: &mut f.query, hint: t("Find in document"), count: info, count_color: color, field_w: 140.0, focus, shadow: true },
        allow_keys,
    );
    #[cfg(debug_assertions)]
    crate::devtest::record("doc-find", out.field);
    if let Some(step) = out.step {
        f.step(step > 0);
    }
    if out.close {
        view.find = None;
        ui.memory_mut(|m| m.surrender_focus(id));
        return false;
    }
    out.focused
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

/// Highlight colours: every match, and the current one, as the editor's (the warn colour at
/// 22% and 55%: readable on white pages and on both themes).
fn match_colors(dark: bool) -> (Color32, Color32) {
    let p = style::palette(dark);
    (style::soft(p.warn, 0.22), style::soft(p.warn, 0.55))
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
    let gap = 16.0;
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
    // White pages faded into the sheet's colour at the edges would look soiled.
    ui.spacing_mut().scroll.fade.strength = 0.0;
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
    let (all, current) = match_colors(ui.visuals().dark_mode);
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
    let sel_fill = ui.visuals().selection.bg_fill.gamma_multiply(0.45);
    area.show_viewport(ui, |ui, vp| {
        let (r, resp) = ui.allocate_exact_size(vec2(total.x.max(vp.width()), total.y), Sense::click_and_drag());
        let o = r.min;
        let page_rect = |i: usize| {
            let (w, h) = (pages[i].0 * k, pages[i].1 * k);
            Rect::from_min_size(pos2(o.x + (r.width() - w) / 2.0, o.y + tops[i]), vec2(w, h))
        };
        // A point of the view as a page and a point on it (in points), clamped to the page.
        let page_at = |pos: Pos2| -> (usize, [f32; 2]) {
            let i = (0..pages.len()).rfind(|i| o.y + tops[*i] <= pos.y).unwrap_or(0);
            let pr = page_rect(i);
            let q = (pos.clamp(pr.min, pr.max) - pr.min) / k;
            (i, [q.x, q.y])
        };
        // Selecting text: drag over it, double-click a word, click elsewhere to clear.
        if let Some(h) = resp.hover_pos()
            && (0..pages.len()).any(|i| page_rect(i).contains(h))
        {
            ui.ctx().set_cursor_icon(egui::CursorIcon::Text);
        }
        if resp.drag_started() || resp.double_clicked() {
            view.want_text();
        }
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
        if resp.drag_started()
            && let Some(at) = ui.input(|i| i.pointer.press_origin())
        {
            let at = page_at(at);
            pdf.sel = Some((at, at));
        } else if resp.dragged()
            && let Some(pos) = resp.interact_pointer_pos()
            && let Some((_, head)) = pdf.sel.as_mut()
        {
            *head = page_at(pos);
        }
        if resp.double_clicked()
            && let Some(pos) = resp.interact_pointer_pos()
        {
            let (page, pt) = page_at(pos);
            if !pdf.select_word(page, pt) {
                pdf.sel = None;
            }
        } else if resp.clicked() {
            pdf.sel = None;
        }
        let selection = pdf.selection();
        if !selection.is_empty() {
            set_doc_selection(ui.ctx());
            if ui.input(|i| i.events.iter().any(|e| matches!(e, egui::Event::Copy))) {
                ui.ctx().copy_text(pdf.selected_text());
            }
        }
        let painter = ui.painter();
        for i in visible {
            let (w, h) = (pages[i].0 * k, pages[i].1 * k);
            let page = Rect::from_min_size(pos2(o.x + (r.width() - w) / 2.0, o.y + tops[i]), vec2(w, h));
            painter.add(egui::epaint::Shadow { offset: [0, 2], blur: 6, spread: 0, color: Color32::from_black_alpha(64) }.as_shape(page, CornerRadius::ZERO));
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
            for (_, a, b) in selection.iter().filter(|(pg, _, _)| *pg == i) {
                let Some(text) = pdf.text.get(i).and_then(|t| t.as_ref()) else { continue };
                for r in text.boxes_between(*a, *b) {
                    let hr = Rect::from_min_max(page.min + vec2(r[0] * k, r[1] * k), page.min + vec2(r[2] * k, r[3] * k));
                    painter.rect_filled(hr, CornerRadius::ZERO, sel_fill);
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
    /// A deck: where each slide starts (its "Slide N" label).
    slides: Vec<f32>,
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
        ParaStyle::Quote => (base, false, p.text_muted, false, false),
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
            let mark = self.ui.painter().layout_no_wrap(list.mark.clone(), egui::FontId::proportional(base), self.p.text_faint);
            let mr = Rect::from_min_size(pos2(x + indent - mark.size().x - 6.0 * z, y), mark.size());
            self.items.push(Item::Text { rect: mr, galley: mark });
            x += indent;
            w -= indent;
        }
        if para.style == ParaStyle::Quote {
            self.items.push(Item::Fill { rect: Rect::from_min_size(pos2(x, y), vec2(3.0 * z, 1.0)), color: self.p.border_heavy, radius: 1, stroke: Stroke::NONE });
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
            Layout { key, items: b.items, height: y + margin, slides: Vec::new() }
        }
        Content::Slides(deck) => {
            let card_w = (width - margin * 2.0).min(960.0 * zoom).max(200.0);
            let x = ((width - card_w) / 2.0).max(margin);
            // Text sized for the card, as on the slide.
            let s = card_w / 900.0;
            let mut b = Builder { ui, p, zoom: s, pictures: &deck.images, items: Vec::new() };
            let mut y = margin;
            let mut slides = Vec::with_capacity(deck.slides.len());
            for (i, slide) in deck.slides.iter().enumerate() {
                slides.push(y);
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
            Layout { key, items: b.items, height: y + margin, slides }
        }
        _ => Layout { key, items: Vec::new(), height: 0.0, slides: Vec::new() },
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
    let (all, current) = match_colors(ui.visuals().dark_mode);
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
    // A slide asked for (the toolbar's arrows or number, ← →): its top a little below the view's.
    if let Some(j) = view.slide_jump.take()
        && let Some(top) = view.layout.as_ref().and_then(|l| l.slides.get(j))
    {
        area = area.vertical_scroll_offset((top - 12.0).max(0.0));
    }
    area.show_viewport(ui, |ui, vp| {
            let Some(layout) = view.layout.as_ref() else { return };
            let (r, _) = ui.allocate_exact_size(vec2(rect.width(), layout.height), Sense::hover());
            let o = r.min.to_vec2();
            if !layout.slides.is_empty() {
                // The slide at the view's upper third.
                let at = vp.min.y + vp.height() / 3.0;
                view.slide = layout.slides.iter().rposition(|top| *top <= at).unwrap_or(0);
            }
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
                        // Selectable (drag across paragraphs, double-click a word); ⌘C copies it.
                        let pos = rect.min + o;
                        let resp = ui.interact(galley.rect.translate(pos.to_vec2()), ui.id().with(("doc-text", ix)), Sense::click_and_drag());
                        if resp.hovered() {
                            ui.ctx().set_cursor_icon(egui::CursorIcon::Text);
                        }
                        egui::text_selection::LabelSelectionState::label_text_selection(ui, &resp, pos, galley.clone(), p.text, Stroke::NONE);
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

fn sheet_body(ui: &mut Ui, view: &mut DocView, rect: Rect, p: &style::Palette) -> bool {
    let about = summary(ui, view);
    let cut_short = truncated(view);
    let State::Ready(Content::Sheet(book)) = &view.state else { return false };
    let multi = book.sheets.len() > 1;
    let names: Vec<String> = book.sheets.iter().map(|s| s.name.clone()).collect();
    let tabs = Rect::from_min_max(pos2(rect.left(), rect.bottom() - TABS_H), rect.max);
    let grid_rect = Rect::from_min_max(pos2(rect.left(), rect.top() + CELL_BAR_H), if multi { pos2(rect.right(), tabs.top()) } else { rect.max });
    let bar = Rect::from_min_max(rect.min, pos2(rect.right(), rect.top() + CELL_BAR_H));
    let Some(sheet) = book.sheets.get(view.sheet) else { return false };
    let z = view.zoom;
    let fs = style::DATA * z;
    let row_h = (26.0 * z).round().max(12.0);
    let char_w = fs * 0.62;
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
    let head_h = (24.0 * z).round().max(12.0);
    let font = style::mono(fs);
    // The header row (the sheet's first) stays in view under the column letters.
    let sticky = if sheet.rows > 1 { row_h } else { 0.0 };

    // The selected cell: its name in a well, its whole text, what it is in words (its column's
    // header, the column's first values); Copy at the right.
    ui.painter().hline(bar.x_range(), bar.bottom() - 0.5, Stroke::new(1.0, p.border));
    let cy = bar.center().y;
    let name = view.cell.map_or_else(|| "—".to_string(), |(r, c)| cell_name(sheet, r, c));
    let ng = ui.painter().layout_no_wrap(name, style::mono(style::DATA), p.text);
    let nr = Rect::from_min_size(pos2(bar.left() + 10.0, cy - 12.0), vec2((ng.size().x + 14.0).max(44.0), 24.0));
    ui.painter().rect_filled(nr, CornerRadius::same(5), p.inset);
    ui.painter().galley(pos2(nr.center().x - ng.size().x / 2.0, nr.center().y - ng.size().y / 2.0), ng, p.text);
    if let Some((r, c)) = view.cell {
        let cell = sheet.cell(r, c);
        // What kind of value it is, on the name.
        ui.interact(nr, egui::Id::new("doc-cell-name"), Sense::hover()).on_hover_text(cell.kind_label());
        let copy = Rect::from_min_max(pos2(bar.right() - 10.0 - ICON_S, cy - ICON_S / 2.0), pos2(bar.right() - 10.0, cy + ICON_S / 2.0));
        let copied = ui.scope_builder(egui::UiBuilder::new().max_rect(copy), |ui| kit::icon_button_ex(ui, icons::COPY, t("Copy"), false, ICON_S)).inner;
        if copied.clicked() {
            ui.ctx().copy_text(cell.text().to_string());
        }
        // The column's header, and its first values under it.
        let first_line = |x: &str| x.lines().next().unwrap_or("").trim().to_string();
        let header = if r > 0 && sticky > 0.0 { first_line(sheet.cell(0, c).text()) } else { String::new() };
        let firsts: Vec<String> = (if sticky > 0.0 { 1 } else { 0 }..sheet.rows.min(200)).map(|row| first_line(sheet.cell(row, c).text())).filter(|x| !x.is_empty()).take(3).collect();
        let mut context = Vec::new();
        if !header.is_empty() {
            context.push(header);
        }
        if !firsts.is_empty() {
            context.push(tf("first values {}", &[&firsts.join(", ")]));
        }
        let text = cell.text().replace('\n', " ⏎ ");
        let room = (copy.left() - 10.0 - (nr.right() + 10.0)).max(10.0);
        let mut job = egui::text::LayoutJob::default();
        job.append(&text, 0.0, egui::TextFormat::simple(style::mono(style::MONO), p.text));
        if !context.is_empty() {
            job.append(&context.join(" · "), 10.0, egui::TextFormat::simple(egui::FontId::proportional(style::SMALL), p.text_faint));
        }
        job.wrap = egui::text::TextWrapping::truncate_at_width(room);
        let g = ui.painter().layout_job(job);
        ui.painter().galley(pos2(nr.right() + 10.0, cy - g.size().y / 2.0), g, p.text);
    }
    if let Some(e) = &sheet.error {
        ui.painter().text(grid_rect.center(), Align2::CENTER_CENTER, e, egui::FontId::proportional(style::BODY), p.err);
        return false;
    }

    // The grid has a margin round it (8 at 100%), its gutters lit, the cells on the sheet.
    let pad = (8.0 * z).round();
    let total = vec2(pad + rn_w + acc + pad, pad + head_h + sheet.rows as f32 * row_h + pad);
    let mut select = None;
    let mut keys_moved = None;
    let mut focused = false;
    ui.scope_builder(egui::UiBuilder::new().max_rect(grid_rect), |ui| {
        ui.set_clip_rect(grid_rect);
        let salt = ("sheet", view.path.clone(), view.sheet);
        let mut area = egui::ScrollArea::both().id_salt(&salt).auto_shrink([false, false]);
        if std::mem::take(&mut view.scroll_to_cell) {
            // Each sheet scrolls on its own: from this one's offset, not that of the sheet shown
            // last (a match on another sheet may have just switched to this one).
            let id = ui.make_persistent_id(egui::IdSalt::new(&salt));
            view.sheet_scroll = egui::scroll_area::State::load(ui.ctx(), id).map_or(egui::Vec2::ZERO, |s| s.offset);
            if let Some(target) = scroll_target(view.cell, view.sheet_scroll, &xs, row_h, head_h + pad, sticky, rn_w + pad, grid_rect) {
                area = area.scroll_offset(target);
            }
        }
        area.show_viewport(ui, |ui, vp| {
            let (r, resp) = ui.allocate_exact_size(total.max(vp.size()), Sense::click());
            // The grid's origin: its top-left corner inside the margin.
            let o = r.min + vec2(pad, pad);
            let painter = ui.painter();
            let row1 = (((vp.max.y - pad) / row_h).ceil().max(0.0) as usize).min(sheet.rows);
            // The offset is clamped only at the frame's end: zoomed out at the bottom of a long
            // sheet, the view can start below the last row for a frame.
            let row0 = ((((vp.min.y - pad - head_h) / row_h).floor().max(0.0)) as usize).min(row1);
            let col0 = xs.partition_point(|x| *x <= vp.min.x - pad - rn_w).saturating_sub(1);
            let col1 = xs.partition_point(|x| *x < vp.max.x - pad - rn_w).min(widths.len());
            let (all, current) = match_colors(ui.visuals().dark_mode);
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
            // What stays in view: the column letters at the view's top, the row numbers at its
            // left (inside the margin while the grid is not scrolled past it).
            let top = (r.min.y + vp.min.y).max(o.y);
            let left = (r.min.x + vp.min.x).max(o.x);
            // One row's cells at `y`: a find's match, the text (numbers at the right; the header
            // row in the strong small font).
            let paint_cells = |row: usize, y: f32| {
                let header = row == 0 && sticky > 0.0;
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
                        _ if header => (p.text, false),
                        tc_doc::sheet::Cell::Number(_) => (p.text_muted, true),
                        tc_doc::sheet::Cell::Bool(_) => (p.accent_text, false),
                        tc_doc::sheet::Cell::Error(_) => (p.err, false),
                        _ => (p.text_muted, false),
                    };
                    let f = if header { style::medium(style::SMALL * z) } else { font.clone() };
                    let mut job = egui::text::LayoutJob::simple_singleline(text.lines().next().unwrap_or("").to_string(), f, color);
                    job.wrap = egui::text::TextWrapping::truncate_at_width((cr.width() - 16.0 * z).max(4.0));
                    let g = painter.layout_job(job);
                    let gx = if right { cr.right() - 8.0 * z - g.size().x } else { cr.left() + 8.0 * z };
                    painter.galley(pos2(gx, cr.center().y - g.size().y / 2.0), g, color);
                }
            };
            let first = if sticky > 0.0 { row0.max(1) } else { row0 };
            for row in first..row1 {
                paint_cells(row, o.y + head_h + row as f32 * row_h);
            }
            // The selected cell: the accent's tint, ringed in it.
            let selected = |painter: &egui::Painter, row: usize, y: f32| {
                if let Some((sr, sc)) = view.cell
                    && sr == row
                    && sc < widths.len()
                {
                    let cr = Rect::from_min_size(pos2(o.x + rn_w + xs[sc], y), vec2(widths[sc], row_h));
                    painter.rect_filled(cr, CornerRadius::ZERO, p.accent_tint);
                    painter.rect_stroke(cr, CornerRadius::ZERO, Stroke::new(2.0, p.accent), egui::StrokeKind::Inside);
                }
            };
            if let Some((sr, _)) = view.cell
                && (sticky == 0.0 || sr > 0)
            {
                let y = o.y + head_h + sr as f32 * row_h;
                selected(painter, sr, y);
            }
            let cells_x = o.x + rn_w + xs[col0]..=o.x + rn_w + xs[col1];
            // The header row, kept under the column letters.
            let sticky_y = top + head_h;
            if sticky > 0.0 {
                let band = Rect::from_x_y_ranges(cells_x.clone(), sticky_y..=sticky_y + row_h);
                painter.rect_filled(band, CornerRadius::ZERO, p.canvas);
                if view.cell.is_some_and(|(sr, _)| sr == 0) {
                    selected(painter, 0, sticky_y);
                }
                paint_cells(0, sticky_y);
                if sticky_y > o.y + head_h + 0.5 {
                    painter.hline(band.x_range(), band.bottom() - 0.5, Stroke::new(1.0, p.border));
                }
            }
            if let Some((sr, sc)) = view.cell
                && sc < widths.len()
                && (sticky == 0.0 || sr > 0)
            {
                // The selected cell's text again over its tint.
                let y = o.y + head_h + sr as f32 * row_h;
                if y >= sticky_y + sticky - 0.5 {
                    let cr = Rect::from_min_size(pos2(o.x + rn_w + xs[sc], y), vec2(widths[sc], row_h));
                    let right = matches!(sheet.cell(sr, sc), tc_doc::sheet::Cell::Number(_));
                    let mut job = egui::text::LayoutJob::simple_singleline(sheet.cell(sr, sc).text().lines().next().unwrap_or("").to_string(), font.clone(), p.text);
                    job.wrap = egui::text::TextWrapping::truncate_at_width((cr.width() - 16.0 * z).max(4.0));
                    let g = painter.layout_job(job);
                    let gx = if right { cr.right() - 8.0 * z - g.size().x } else { cr.left() + 8.0 * z };
                    painter.galley(pos2(gx, cr.center().y - g.size().y / 2.0), g, p.text);
                }
            }
            // The column letters and the row numbers stay in view: lit blocks a point apart.
            let lit = |painter: &egui::Painter, r: Rect, on: bool| {
                painter.rect_filled(r, CornerRadius::ZERO, p.canvas);
                painter.rect_filled(r, CornerRadius::ZERO, if on { p.accent_tint } else { p.hover });
            };
            let head_y = top;
            painter.rect_filled(Rect::from_min_size(pos2(o.x + rn_w + xs[col0], head_y), vec2(xs[col1] - xs[col0], head_h + 1.0)), CornerRadius::ZERO, p.canvas);
            for col in col0..col1 {
                let cr = Rect::from_min_size(pos2(o.x + rn_w + xs[col] + 1.0, head_y), vec2(widths[col] - 1.0, head_h));
                let on = view.cell.is_some_and(|(_, c)| c == col);
                lit(painter, cr, on);
                painter.text(cr.center(), Align2::CENTER_CENTER, tc_doc::sheet::column_name(sheet.origin.1 + col as u32), style::mono(style::DATA * z), if on { p.accent_text } else { p.text_faint });
            }
            let number = |painter: &egui::Painter, row: usize, y: f32| {
                let rr = Rect::from_min_size(pos2(left, y + 1.0), vec2(rn_w, row_h - 1.0));
                let on = view.cell.is_some_and(|(r, _)| r == row);
                lit(painter, rr, on);
                painter.text(rr.center(), Align2::CENTER_CENTER, (sheet.origin.0 as usize + row + 1).to_string(), style::mono(style::DATA * z), if on { p.accent_text } else { p.text_faint });
            };
            painter.rect_filled(Rect::from_min_size(pos2(left, o.y + head_h + row0 as f32 * row_h), vec2(rn_w + 1.0, (row1 - row0) as f32 * row_h)), CornerRadius::ZERO, p.canvas);
            for row in first..row1 {
                let y = o.y + head_h + row as f32 * row_h;
                if y + row_h < top + head_h + sticky {
                    continue;
                }
                number(painter, row, y);
            }
            if sticky > 0.0 {
                number(painter, 0, sticky_y);
            }
            // The corner over the row numbers.
            lit(painter, Rect::from_min_size(pos2(left, head_y), vec2(rn_w, head_h)), false);
            // A click picks the cell under it (ignoring clicks on the column letters or row numbers).
            if resp.clicked()
                && let Some(pos) = resp.interact_pointer_pos()
            {
                let (x, y) = (pos.x - o.x - rn_w, pos.y - o.y - head_h);
                // On the header row kept in view: that row.
                let on_sticky = sticky > 0.0 && pos.y >= sticky_y && pos.y < sticky_y + row_h;
                if pos.x >= left + rn_w && pos.y >= top + head_h && x >= 0.0 && y >= 0.0 {
                    let row = if on_sticky { 0 } else { (y / row_h) as usize };
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
                ui.memory_mut(|m| {
                    m.set_focus_lock_filter(resp.id, egui::EventFilter { horizontal_arrows: true, vertical_arrows: true, ..Default::default() });
                });
                focused = true;
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
        // The sheets' tabs, what the workbook is at the right.
        ui.painter().hline(tabs.x_range(), tabs.top() + 0.5, Stroke::new(1.0, p.border));
        let g = ui.painter().layout_no_wrap(about, style::mono(style::DATA), p.text_faint);
        let about_w = g.size().x.min(tabs.width() * 0.5);
        let about_left = tabs.right() - 12.0 - about_w;
        let about_y = tabs.center().y - g.size().y / 2.0;
        ui.painter().with_clip_rect(Rect::from_min_max(pos2(about_left, tabs.top()), tabs.max)).galley(pos2(about_left, about_y), g, p.text_faint);
        let mut strip_right = about_left - 12.0;
        if let Some(w) = &cut_short {
            let wg = ui.painter().layout_no_wrap(w.clone(), egui::FontId::proportional(style::LABEL), p.warn);
            let ww = wg.size().x.min((strip_right - tabs.left()) * 0.5);
            paint_truncated(ui, w, strip_right - ww, strip_right, tabs.center().y);
            strip_right -= ww + 12.0;
        }
        let strip = Rect::from_min_max(pos2(tabs.left() + 8.0, tabs.top() + 1.0), pos2(strip_right.max(tabs.left() + 40.0), tabs.bottom()));
        let mut pick = None;
        ui.scope_builder(egui::UiBuilder::new().max_rect(strip), |ui| {
            ui.set_clip_rect(strip.intersect(ui.clip_rect()));
            egui::ScrollArea::horizontal()
                .id_salt(("sheet-tabs", view.path.clone()))
                .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden)
                .show(ui, |ui| {
                    ui.horizontal_centered(|ui| {
                        ui.spacing_mut().item_spacing.x = 4.0;
                        for (i, name) in names.iter().enumerate() {
                            let w = super::browser_ui::segment_width(ui, Some(icons::TABLE), name).min(220.0);
                            let (r, _) = ui.allocate_exact_size(vec2(w, 24.0), Sense::hover());
                            if super::browser_ui::segment(ui, r, egui::Id::new(("sheet-tab", view.path.clone(), i)), Some(icons::TABLE), name, i == view.sheet).clicked() {
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
    focused
}

/// Where to scroll (from `at`) so the cell the arrows moved to is in view.
/// `sticky`: the height of the header row kept in view (row 0, never scrolled to).
#[allow(clippy::too_many_arguments)]
fn scroll_target(cell: Option<(usize, usize)>, at: egui::Vec2, xs: &[f32], row_h: f32, head_h: f32, sticky: f32, rn_w: f32, area: Rect) -> Option<egui::Vec2> {
    let (r, c) = cell?;
    let (x0, x1) = (xs.get(c).copied()?, xs.get(c + 1).copied()?);
    let (y0, y1) = ((r as f32 * row_h - sticky).max(0.0), (r + 1) as f32 * row_h);
    let (vw, vh) = (area.width() - rn_w - 12.0, area.height() - head_h - 12.0);
    let mut out = at;
    if x0 < at.x {
        out.x = x0;
    } else if x1 > at.x + vw {
        out.x = x1 - vw;
    }
    if r == 0 && sticky > 0.0 {
        // The header row is always in view.
    } else if y0 < at.y {
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
    let fit = ((rect.width() - margin * 2.0).max(0.0) / natural.x.max(1.0))
        .min((rect.height() - margin * 2.0).max(0.0) / natural.y.max(1.0))
        .min(1.0);
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

/// What kind of value a cell holds, for the cell bar.
trait KindLabel {
    fn kind_label(&self) -> &'static str;
}

impl KindLabel for tc_doc::sheet::Cell {
    fn kind_label(&self) -> &'static str {
        use tc_doc::sheet::Cell;
        match self {
            Cell::Empty => t("Empty"),
            Cell::Text(_) => t("Text"),
            Cell::Number(_) => t("Number"),
            Cell::Bool(_) => t("True or false"),
            Cell::Error(_) => t("Error"),
        }
    }
}
