//! The status bar (the most urgent waiting pane, ⌘J, counts and memory), the attention list
//! (FR-N7) and toasts.

use std::time::{Duration, Instant};

use eframe::egui::{self, Align2, CornerRadius, Rect, Sense, Stroke, Ui, pos2, vec2};
use tc_core::status::Status;

use super::{icons, kit, status, style};
use crate::actions::Action;
use crate::app::{App, ToastKind};
use crate::i18n::{t, tf};

/// Width of a toast.
const TOAST_W: f32 = 380.0;

impl App {
    /// The status bar under the terminals (26 high, quiet). Left: where you are (the branch,
    /// the language server, problems). Right: agents not connected, an update, what waits for
    /// you (⌘J), how many agents run, the memory TernaryCode uses.
    pub fn status_bar(&mut self, ui: &mut Ui) {
        let p = style::pal(ui);
        let bar = ui.max_rect();
        // Agents that ask something (finished and failed ones wait in the Inbox).
        let waiting = self.panes.values().filter(|x| x.tracker.status == Status::NeedsInput).count();
        let running = self.panes.values().filter(|x| x.tracker.status == Status::Running && x.tracker.agent.is_some()).count();
        let unconnected = self.unconnected_agents();
        let right_used = ui
            .scope_builder(egui::UiBuilder::new().max_rect(bar).layout(egui::Layout::right_to_left(egui::Align::Center)), |ui| {
                ui.spacing_mut().item_spacing.x = 16.0;
                let now = Instant::now();
                if self.mem_bytes.1.is_none_or(|t| now.duration_since(t) > Duration::from_secs(2)) {
                    self.mem_bytes = (tc_agent::proc::own_resident_bytes(), Some(now));
                    self.browser_bytes = self.browsers_resident_bytes();
                }
                if let Some(b) = self.mem_bytes.0 {
                    let mut tip = t("Memory TernaryCode uses (programs in the panes not counted)").to_string();
                    if let Some(pb) = self.browser_bytes.filter(|b| *b > 0) {
                        tip.push_str(&format!("\n{}", tf("Pages: {} MB", &[&(pb / (1024 * 1024)).to_string()])));
                    }
                    ui.label(egui::RichText::new(format!("{} MB", b / (1024 * 1024))).font(style::mono(style::DATA)).color(p.text_faint)).on_hover_text(tip);
                }
                let r = bar_item(
                    ui,
                    |painter, at| {
                        if running > 0 {
                            status::paint_glyph(painter, at, Status::Running, &p, 12.0);
                        } else {
                            painter.text(at, Align2::CENTER_CENTER, icons::CIRCLE_NOTCH, style::icon_font(12.0), p.text_faint);
                        }
                    },
                    &tf("{} running", &[&running.to_string()]),
                    egui::FontId::proportional(style::SMALL),
                    if running > 0 { p.text_muted } else { p.text_faint },
                    None,
                );
                #[cfg(debug_assertions)]
                crate::devtest::record("status-running", r.rect);
                r.on_hover_text(t("Agents at work in every project"));
                let auto_running = self.running_automations_count();
                if auto_running > 0 {
                    let label = crate::i18n::count(auto_running, t("1 automation"), t("{} automations"));
                    let r = bar_item(
                        ui,
                        |painter, at| status::paint_glyph(painter, at, Status::Running, &p, 12.0),
                        &label,
                        style::medium(style::SMALL),
                        p.info,
                        None,
                    );
                    if r.on_hover_text(t("Background automations running now")).clicked() {
                        self.actions.push(Action::SetView(tc_core::model::View::Automations));
                    }
                }
                let r = if waiting > 0 {
                    let keys = kit::keys(&Action::NextAttention);
                    bar_item(ui, |painter, at| status::paint_glyph(painter, at, Status::NeedsInput, &p, 12.0), &tf("{} waiting", &[&waiting.to_string()]), style::medium(style::SMALL), p.warn, keys)
                } else {
                    bar_item(ui, |painter, at| { painter.text(at, Align2::CENTER_CENTER, icons::CHECK, style::icon_font(12.0), p.text_faint); }, t("Nothing waiting"), egui::FontId::proportional(style::SMALL), p.text_faint, None)
                };
                #[cfg(debug_assertions)]
                crate::devtest::record("attention-summary", r.rect);
                if r.on_hover_text(t("Show everything that waits for you")).clicked() {
                    self.actions.push(Action::ToggleAttention);
                }
                self.update_status_item(ui);
                if !unconnected.is_empty() {
                    let text = crate::i18n::count(unconnected.len(), t("1 agent not connected"), t("{} agents not connected"));
                    let r = bar_item(ui, |painter, at| { painter.text(at, Align2::CENTER_CENTER, icons::PLUGS, style::icon_font(12.0), p.warn); }, &text, egui::FontId::proportional(style::SMALL), p.warn, None);
                    #[cfg(debug_assertions)]
                    crate::devtest::record("unconnected", r.rect);
                    if r.on_hover_text(self.unconnected_tip(&unconnected)).clicked() {
                        self.connect_agents(&unconnected);
                    }
                }
                if self.view == tc_core::model::View::Grid
                    && let Some(target) = self.keyboard_target()
                {
                    let keys = kit::keys(&Action::OpenGridTile);
                    let r = bar_item(ui, |painter, at| { painter.text(at, Align2::CENTER_CENTER, icons::KEYBOARD, style::icon_font(12.0), p.accent_text); }, &target, egui::FontId::proportional(style::SMALL), p.accent_text, keys);
                    if r.on_hover_text(t("Where your typing goes. Click (or press the keys) to open it in Focus.")).clicked() {
                        self.actions.push(Action::OpenGridTile);
                    }
                }
                ui.min_rect().left()
            })
            .inner;
        // Left: the project in view.
        let Some(pid) = self.active.filter(|_| self.view == tc_core::model::View::Focus) else { return };
        let left = Rect::from_min_max(bar.min, pos2((right_used - 16.0).max(bar.left()), bar.bottom()));
        let git = self.project(pid).and_then(|x| x.git.as_ref()).map(|g| (g.branch.clone(), g.ahead, g.behind));
        let root = self.project(pid).map(|x| x.path.clone()).unwrap_or_default();
        let server = self
            .lsp
            .servers
            .iter()
            .filter(|(k, _)| k.root.starts_with(&root) || root.starts_with(&k.root))
            .map(|(k, s)| (k.name, s.state.clone(), s.progress.clone()))
            .min_by_key(|(name, ..)| *name);
        let (errs, warns) = self.project_diagnostic_counts(pid);
        ui.scope_builder(egui::UiBuilder::new().max_rect(left).layout(egui::Layout::left_to_right(egui::Align::Center)), |ui| {
            ui.set_clip_rect(left.intersect(ui.clip_rect()));
            ui.spacing_mut().item_spacing.x = 16.0;
            if let Some((branch, ahead, behind)) = git {
                let mut job = egui::text::LayoutJob::default();
                job.append(&branch, 0.0, egui::TextFormat::simple(style::mono(style::DATA), p.text_muted));
                if ahead + behind > 0 {
                    job.append(&format!("↑{ahead} ↓{behind}"), 5.0, egui::TextFormat::simple(style::mono(style::DATA), p.text_faint));
                }
                let g = ui.painter().layout_job(job);
                let (r, resp) = ui.allocate_exact_size(vec2(12.0 + 5.0 + g.size().x, 20.0), Sense::click());
                ui.painter().text(pos2(r.left() + 6.0, r.center().y), Align2::CENTER_CENTER, icons::GIT_BRANCH, style::icon_font(12.0), p.text_faint);
                ui.painter().galley(pos2(r.left() + 17.0, r.center().y - g.size().y / 2.0), g, p.text_muted);
                if resp.on_hover_cursor(egui::CursorIcon::PointingHand).on_hover_text(kit::tip(t("History"), Action::History)).clicked() {
                    self.actions.push(Action::History);
                }
            }
            if let Some((name, state, progress)) = &server {
                let (dot, tip) = match &state {
                    crate::lsp::ServerState::Ready => (p.ok, progress.clone().unwrap_or_else(|| t("Ready").to_string())),
                    crate::lsp::ServerState::Starting => (p.info, t("Starting…").to_string()),
                    crate::lsp::ServerState::Failed(why) => (p.err, why.clone()),
                    crate::lsp::ServerState::Missing => (p.text_ghost, t("Not installed").to_string()),
                };
                let g = ui.painter().layout_no_wrap(name.to_string(), style::mono(style::DATA), p.text_faint);
                let (r, resp) = ui.allocate_exact_size(vec2(6.0 + 5.0 + g.size().x, 20.0), Sense::hover());
                ui.painter().circle_filled(pos2(r.left() + 3.0, r.center().y), 3.0, dot);
                ui.painter().galley(pos2(r.left() + 11.0, r.center().y - g.size().y / 2.0), g, p.text_faint);
                resp.on_hover_text(tip);
            }
            if server.is_some() || errs + warns > 0 {
                let mut job = egui::text::LayoutJob::default();
                job.append(&errs.to_string(), 0.0, egui::TextFormat::simple(style::mono(style::DATA), if errs > 0 { p.err } else { p.text_faint }));
                let eg = ui.painter().layout_job(job);
                let wg = ui.painter().layout_no_wrap(warns.to_string(), style::mono(style::DATA), if warns > 0 { p.warn } else { p.text_faint });
                let w = 12.0 + 4.0 + eg.size().x + 6.0 + 12.0 + 4.0 + wg.size().x;
                let (r, resp) = ui.allocate_exact_size(vec2(w, 20.0), Sense::hover());
                let cy = r.center().y;
                let mut x = r.left();
                ui.painter().text(pos2(x + 6.0, cy), Align2::CENTER_CENTER, icons::X_CIRCLE, style::icon_font(12.0), p.text_faint);
                x += 16.0;
                let ew = eg.size().x;
                ui.painter().galley(pos2(x, cy - eg.size().y / 2.0), eg, p.text_faint);
                x += ew + 6.0;
                ui.painter().text(pos2(x + 6.0, cy), Align2::CENTER_CENTER, icons::WARNING, style::icon_font(12.0), p.text_faint);
                x += 16.0;
                ui.painter().galley(pos2(x, cy - wg.size().y / 2.0), wg, p.text_faint);
                resp.on_hover_text(tf("{} errors, {} warnings in this project", &[&errs.to_string(), &warns.to_string()]));
            }
        });
    }

    /// Errors and warnings the language servers report for files of `pid`.
    fn project_diagnostic_counts(&self, pid: tc_core::ProjectId) -> (usize, usize) {
        let Some(root) = self.project(pid).map(|x| &x.path) else {
            return (0, 0);
        };
        let mut errs = 0usize;
        let mut warns = 0usize;
        for (path, (_, diags)) in &self.lsp.diagnostics {
            if path.starts_with(root) {
                for d in diags {
                    match d.severity {
                        Some(tc_lsp::types::DiagnosticSeverity::ERROR) => errs += 1,
                        Some(tc_lsp::types::DiagnosticSeverity::WARNING) => warns += 1,
                        _ => {}
                    }
                }
            }
        }
        (errs, warns)
    }

    /// The toasts: bottom-right over the status bar, stacked (three at most), newest on top.
    /// Each goes after 4 s (6 s with an action; not while the pointer is on it); its X closes
    /// it, its action does what it says.
    pub fn toast_ui(&mut self, ctx: &egui::Context) {
        let now = Instant::now();
        let life = |t: &crate::app::Toast| if t.action.is_some() { Duration::from_secs(6) } else { Duration::from_secs(4) };
        self.toasts.retain(|t| now.duration_since(t.at) < life(t));
        let Some(next) = self.toasts.iter().map(|t| life(t).saturating_sub(now.duration_since(t.at))).min() else { return };
        ctx.request_repaint_after(next);
        let screen = ctx.content_rect();
        let dark = ctx.global_style().visuals.dark_mode;
        let w = TOAST_W.min(screen.width() - 32.0).max(200.0);
        let right = screen.right() - 16.0;
        let mut bottom = screen.bottom() - if self.zen_mode { 0.0 } else { super::view::STATUS_H } - 12.0;
        let (mut act, mut closed, mut held) = (None, None, Vec::new());
        // The oldest lowest, each newer one above it.
        for (i, toast) in self.toasts.iter().enumerate() {
            let shown = egui::Area::new(egui::Id::new(("toast", i)))
                .order(egui::Order::Tooltip)
                .pivot(Align2::RIGHT_BOTTOM)
                .fixed_pos(pos2(right, bottom))
                .show(ctx, |ui| {
                    let p = style::palette(dark);
                    let painter = ui.painter().clone();
                    // Glyph 16, the text, the action (Ghost S), close (20): 10 · 10 · 10 apart, padded
                    // 10 (12 at the left).
                    let (glyph, color, filled) = match toast.kind {
                        ToastKind::Info => (icons::INFO, p.info, true),
                        ToastKind::Ok => (icons::CHECK_CIRCLE, p.ok, true),
                        ToastKind::Warn => (icons::WARNING, p.warn, true),
                        ToastKind::Err => (icons::X_CIRCLE, p.err, true),
                        ToastKind::Worktree => (icons::GIT_FORK, p.ok, false),
                    };
                    let action_w = toast.action.as_ref().map_or(0.0, |(label, _)| kit::button_width(ui, None, label, None, true) + 10.0);
                    let text_x = 12.0 + 16.0 + 10.0;
                    let text_w = (w - text_x - action_w - 20.0 - 10.0 - 10.0).max(60.0);
                    let title = painter.layout(toast.text.clone(), style::medium(style::BODY), p.text, text_w);
                    let body = toast.body.as_ref().map(|b| painter.layout(b.clone(), egui::FontId::proportional(style::SMALL), p.text_muted, text_w));
                    let text_h = title.size().y + body.as_ref().map_or(0.0, |g| 2.0 + g.size().y);
                    let h = 10.0 + text_h.max(if toast.action.is_some() { 24.0 } else { 20.0 }) + 10.0;
                    let (rect, resp) = ui.allocate_exact_size(vec2(w, h), Sense::hover());
                    #[cfg(debug_assertions)]
                    crate::devtest::record(format!("toast:{i}"), rect);
                    let radius = CornerRadius::same(10);
                    painter.add(egui::epaint::Shadow { offset: [0, 12], blur: 32, spread: 0, color: egui::Color32::from_black_alpha(0x66) }.as_shape(rect, radius));
                    painter.rect(rect, radius, p.menu, Stroke::new(1.0, p.border_strong), egui::StrokeKind::Inside);
                    let top = rect.top() + 10.0;
                    let first_row = title.rows.first().map_or(18.0, |r| r.rect().height());
                    let font = if filled { style::icon_fill(16.0) } else { style::icon_font(16.0) };
                    painter.text(pos2(rect.left() + 12.0 + 8.0, top + first_row / 2.0), Align2::CENTER_CENTER, glyph, font, color);
                    painter.galley(pos2(rect.left() + text_x, top), title.clone(), p.text);
                    if let Some(g) = body {
                        painter.galley(pos2(rect.left() + text_x, top + title.size().y + 2.0), g, p.text_muted);
                    }
                    // Close, at the top right.
                    let close = Rect::from_min_size(pos2(rect.right() - 10.0 - 20.0, top), vec2(20.0, 20.0));
                    let cr = ui.interact(close, egui::Id::new(("toast-close", i)), Sense::click());
                    #[cfg(debug_assertions)]
                    crate::devtest::record(format!("toast-close:{i}"), close);
                    if cr.hovered() {
                        painter.rect_filled(close, CornerRadius::same(5), p.hover);
                        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                    }
                    painter.text(close.center(), Align2::CENTER_CENTER, icons::X, style::icon_font(12.0), if cr.hovered() { p.text } else { p.text_faint });
                    if cr.clicked() {
                        closed = Some(i);
                    }
                    if let Some((label, action)) = &toast.action {
                        let bw = action_w - 10.0;
                        let br = Rect::from_min_size(pos2(close.left() - 10.0 - bw, top), vec2(bw, 24.0));
                        let r = ui.scope_builder(egui::UiBuilder::new().max_rect(br), |ui| kit::button_ex(ui, kit::Kind::Ghost, None, label, true)).inner;
                        #[cfg(debug_assertions)]
                        crate::devtest::record(format!("toast-action:{i}"), r.rect);
                        if r.clicked() {
                            act = Some((i, action.clone()));
                        }
                    }
                    resp.contains_pointer() || cr.contains_pointer()
                });
            if shown.inner {
                held.push(i);
            }
            bottom = shown.response.rect.top() - 10.0;
        }
        // Not gone while the pointer is on it.
        for i in held {
            if let Some(t) = self.toasts.get_mut(i) {
                let keep = life(t).saturating_sub(Duration::from_millis(1500));
                if now.duration_since(t.at) > keep {
                    t.at = now - keep;
                }
            }
        }
        if let Some((i, a)) = act {
            self.actions.push(a);
            self.toasts.remove(i);
        } else if let Some(i) = closed {
            self.toasts.remove(i);
        }
    }
}

/// Key hints in a row ("↑ ↓ choose   ↵ open"), and one at the right (Esc).
/// How wide `paint_key_hints` draws `hints` (without the one at the right).
pub fn key_hints_width(ui: &Ui, hints: &[(&[&str], &str)]) -> f32 {
    let p = style::pal(ui);
    let painter = ui.painter();
    let mut w = 0.0;
    for (keys, what) in hints {
        for k in *keys {
            w += kit::kbd_width(painter, k) + 4.0;
        }
        w += painter.layout_no_wrap(what.to_string(), egui::FontId::proportional(11.5), p.text_faint).size().x + 14.0;
    }
    (w - 14.0).max(0.0)
}

pub fn paint_key_hints(ui: &Ui, rect: Rect, hints: &[(&[&str], &str)], right: Option<&str>) {
    let p = style::pal(ui);
    let painter = ui.painter();
    let cy = rect.center().y;
    let mut x = rect.left();
    for (keys, what) in hints {
        for k in *keys {
            x += kit::paint_kbd_text(painter, pos2(x, cy), k, &p) + 4.0;
        }
        let g = painter.layout_no_wrap(what.to_string(), egui::FontId::proportional(11.5), p.text_faint);
        let w = g.size().x;
        painter.galley(pos2(x, cy - g.size().y / 2.0), g, p.text_faint);
        x += w + 14.0;
    }
    if let Some(k) = right {
        // "esc close" at the right.
        let g = painter.layout_no_wrap(t("close").to_string(), egui::FontId::proportional(11.5), p.text_faint);
        let gw = g.size().x;
        painter.galley(pos2(rect.right() - gw, cy - g.size().y / 2.0), g, p.text_faint);
        let w = kit::kbd_width(painter, k);
        kit::paint_kbd_text(painter, pos2(rect.right() - gw - 5.0 - w, cy), k, &p);
    }
}






/// A status bar item: a 12 px mark (painted by `mark` at its centre), its text, its keys.
fn bar_item(ui: &mut Ui, mark: impl FnOnce(&egui::Painter, egui::Pos2), text: &str, font: egui::FontId, color: egui::Color32, keys: Option<&str>) -> egui::Response {
    let p = style::pal(ui);
    let g = ui.painter().layout_no_wrap(text.to_string(), font, color);
    let kw = keys.map_or(0.0, |k| kit::kbd_width(ui.painter(), k) + 6.0);
    let (r, resp) = ui.allocate_exact_size(vec2(12.0 + 6.0 + g.size().x + kw, 20.0), Sense::click());
    let cy = r.center().y;
    mark(ui.painter(), pos2(r.left() + 6.0, cy));
    let gw = g.size().x;
    ui.painter().galley(pos2(r.left() + 18.0, cy - g.size().y / 2.0), g, color);
    if let Some(k) = keys {
        kit::paint_kbd_text(ui.painter(), pos2(r.left() + 18.0 + gw + 6.0, cy), k, &p);
    }
    if resp.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    resp
}
