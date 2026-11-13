//! The status bar (the most urgent waiting pane, ⌘J, counts and memory), the attention list
//! (FR-N7) and toasts.

use std::time::{Duration, Instant};

use eframe::egui::{self, Align2, CornerRadius, Rect, RichText, Sense, Stroke, Ui, pos2, vec2};
use tc_core::status::Status;

use super::{icons, kit, status, style};
use crate::actions::Action;
use crate::app::{App, ToastKind};
use crate::i18n::{t, tf};

/// Width of the attention list.
const LIST_W: f32 = 470.0;

impl App {
    pub fn status_bar(&mut self, ui: &mut Ui) {
        let p = style::pal(ui);
        let queue = self.attention_queue();
        ui.horizontal_centered(|ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            // Left: the most urgent pane waiting for the user, or that none is.
            match queue.first().and_then(|e| self.panes.get(&e.pane)) {
                Some(pane) => {
                    let s = pane.tracker.status;
                    let c = status::color(s, &p);
                    let project = self.project(pane.project).map(|x| x.name.clone()).unwrap_or_default();
                    let what = super::sidebar::attention_what(pane);
                    let text = format!("{project} · {}: {what}", pane.label());
                    let more = (queue.len() > 1).then(|| tf("(+{} more)", &[&(queue.len() - 1).to_string()]));
                    let room = (ui.available_width() * 0.6).max(120.0);
                    let r = item(ui, Some(status::icon(s)), &text, c, more.as_deref(), kit::keys(&Action::NextAttention), room, true);
                    #[cfg(debug_assertions)]
                    crate::devtest::record("attention-summary", r.rect);
                    let tip = format!("{}\n{}", status::described(&pane.tracker), t("Show everything that waits for you"));
                    if r.on_hover_text(tip).clicked() {
                        self.actions.push(Action::ToggleAttention);
                    }
                }
                None => {
                    let r = item(ui, Some(icons::CHECK_CIRCLE), t("No agent is waiting for you."), p.text_faint, None, None, 400.0, true);
                    if r.on_hover_text(t("Agents that need you show up here")).clicked() {
                        self.actions.push(Action::ToggleAttention);
                    }
                }
            }
            // Agents that can't say when they need the user (started without the hooks).
            let unconnected = self.unconnected_agents();
            if !unconnected.is_empty() {
                dot_sep(ui, &p);
                let text = crate::i18n::count(unconnected.len(), t("1 agent not connected"), t("{} agents not connected"));
                let r = item(ui, Some(icons::PLUGS_CONNECTED), &text, p.warn, None, None, 260.0, true);
                #[cfg(debug_assertions)]
                crate::devtest::record("unconnected", r.rect);
                if r.on_hover_text(self.unconnected_tip(&unconnected)).clicked() {
                    self.connect_agents(&unconnected);
                }
            }
            // Right: commands, then what is running and the memory it takes.
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = 2.0;
                let key = kit::keys(&Action::Palette).unwrap_or_default();
                let commands = item(ui, Some(icons::MAGNIFYING_GLASS), t("Commands"), p.text_muted, None, Some(key), 200.0, true);
                if commands.on_hover_text(t("Everything TernaryCode can do, searchable")).clicked() {
                    self.actions.push(Action::Palette);
                }
                dot_sep(ui, &p);
                let now = Instant::now();
                if self.mem_bytes.1.is_none_or(|t| now.duration_since(t) > Duration::from_secs(2)) {
                    self.mem_bytes = (tc_agent::proc::own_resident_bytes(), Some(now));
                }
                let mem = self.mem_bytes.0.map(|b| format!(" · {} MB", b / (1024 * 1024))).unwrap_or_default();
                let counts = format!(
                    "{} · {}{mem}",
                    crate::i18n::count(self.projects.len(), t("1 project"), t("{} projects")),
                    crate::i18n::count(self.panes.len(), t("1 pane"), t("{} panes")),
                );
                item(ui, None, &counts, p.text_faint, None, None, 280.0, false)
                    .on_hover_text(t("Memory TernaryCode uses (programs in the panes not counted)"));
                if let Some(target) = self.keyboard_target() {
                    dot_sep(ui, &p);
                    let keys = kit::keys(&Action::OpenGridTile);
                    if item(ui, Some(icons::KEYBOARD), &target, p.accent_text, None, keys, 300.0, true)
                        .on_hover_text(t("Where your typing goes. Click (or press the keys) to open it in Focus."))
                        .clicked()
                    {
                        self.actions.push(Action::OpenGridTile);
                    }
                }
                self.update_status_item(ui);
            });
        });
    }

    /// The attention list (⌥⌘J, or a click on the status bar's summary): everything that waits,
    /// grouped (needs input, failed, done, command finished), oldest first in each group. The
    /// row the keyboard is on shows what can be done: jump there, review, mark as seen.
    ///
    /// With the sidebar shown its inbox opens in full instead (`sidebar::inbox`); this then
    /// only takes the keys and closes it on a click elsewhere.
    pub fn attention_panel(&mut self, ctx: &egui::Context) {
        let inbox = self.inbox_rect.take();
        if !self.attention_open {
            return;
        }
        let queue = self.attention_queue();
        let mut close = false;
        let screen = ctx.content_rect();
        let on_top = self.dialog.is_none() && self.palette.is_none() && self.review.is_none();
        let in_sidebar = inbox.is_some();
        // The keyboard: ↑↓ choose, ↵ jump, R review, ⌫ mark as seen, Esc close.
        let n = queue.len();
        if self.attention_sel >= n {
            self.attention_sel = n.saturating_sub(1);
        }
        let mut chosen: Option<(usize, Choice)> = None;
        let sel_before = self.attention_sel;
        if on_top {
            ctx.input_mut(|i| {
                if i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowDown) && n > 0 {
                    self.attention_sel = (self.attention_sel + 1) % n;
                }
                if i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowUp) && n > 0 {
                    self.attention_sel = (self.attention_sel + n - 1) % n;
                }
                if n > 0 && i.consume_key(egui::Modifiers::NONE, egui::Key::Enter) {
                    chosen = Some((self.attention_sel, Choice::Jump));
                }
                if n > 0 && i.consume_key(egui::Modifiers::NONE, egui::Key::R) {
                    chosen = Some((self.attention_sel, Choice::Review));
                }
                if n > 0 && i.consume_key(egui::Modifiers::NONE, egui::Key::Backspace) {
                    chosen = Some((self.attention_sel, Choice::Seen));
                }
                if i.consume_key(egui::Modifiers::NONE, egui::Key::Escape) {
                    close = true;
                }
            });
        }
        if let Some(rect) = inbox.filter(|_| in_sidebar) {
            // The sidebar drew this frame before the keys: it shows the new row in the next.
            if self.attention_sel != sel_before || chosen.is_some() || close {
                ctx.request_repaint();
            }
            if let Some(i) = chosen {
                self.attention_choose(&queue, i, &mut close);
            }
            let toggled = self.actions.iter().any(|a| matches!(a, Action::ToggleAttention));
            let elsewhere = ctx.input(|i| i.pointer.any_click() && i.pointer.interact_pos().is_some_and(|p| !rect.contains(p)));
            if close || (elsewhere && !toggled) {
                self.attention_open = false;
            }
            return;
        }
        let sel = self.attention_sel;
        let panel = egui::Area::new(egui::Id::new("attention-panel"))
            .order(egui::Order::Foreground)
            .pivot(Align2::LEFT_BOTTOM)
            .fixed_pos(screen.left_bottom() + vec2(8.0, -super::view::STATUS_H - 4.0))
            .show(ctx, |ui| {
                let p = style::pal(ui);
                egui::Frame::popup(ui.style())
                    .inner_margin(egui::Margin::same(6))
                    .corner_radius(CornerRadius::same(10))
                    .show(ui, |ui| {
                        ui.set_width(LIST_W - 12.0);
                        ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
                        // Header.
                        let (head, _) = ui.allocate_exact_size(vec2(ui.available_width(), 36.0), Sense::hover());
                        let painter = ui.painter();
                        let cy = head.center().y;
                        painter.text(pos2(head.left() + 10.0 + 6.5, cy), Align2::CENTER_CENTER, icons::TRAY, style::icon_font(13.0), p.warn);
                        let g = painter.layout_no_wrap(t("Waiting for you").to_string(), style::semibold(13.0), p.text);
                        let gw = g.size().x;
                        painter.galley(pos2(head.left() + 10.0 + 13.0 + 8.0, cy - g.size().y / 2.0), g, p.text);
                        if n > 0 {
                            let x = head.left() + 10.0 + 13.0 + 8.0 + gw + 8.0;
                            let w = kit::paint_badge(painter, pos2(10_000.0, cy), &n.to_string(), egui::Color32::TRANSPARENT, egui::Color32::TRANSPARENT, 16.0).width();
                            kit::paint_badge(painter, pos2(x + w, cy), &n.to_string(), p.warn, p.on_accent, 16.0);
                        }
                        let x_rect = Rect::from_center_size(pos2(head.right() - 6.0 - 11.0, cy), vec2(22.0, 22.0));
                        if ui.scope_builder(egui::UiBuilder::new().max_rect(x_rect), |ui| kit::icon_button_ex(ui, icons::X, t("Close"), false, 22.0)).inner.clicked() {
                            close = true;
                        }
                        if n > 0 {
                            let w = kit::button_width(ui, None, t("Mark All Seen"), None, true);
                            let r = Rect::from_center_size(pos2(x_rect.left() - 4.0 - w / 2.0, cy), vec2(w, 24.0));
                            if ui.scope_builder(egui::UiBuilder::new().max_rect(r), |ui| kit::button_ex(ui, kit::Kind::Ghost, None, t("Mark All Seen"), true)).inner.clicked() {
                                self.actions.push(Action::MarkAllSeen);
                            }
                        }
                        if queue.is_empty() {
                            ui.add_space(4.0);
                            let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), 30.0), Sense::hover());
                            ui.painter().text(pos2(r.left() + 10.0, r.center().y), Align2::LEFT_CENTER, t("No agent is waiting for you."), egui::FontId::proportional(13.0), p.text_muted);
                            ui.add_space(6.0);
                        }
                        // Groups, in the queue's order.
                        let mut group: Option<u8> = None;
                        for (i, e) in queue.iter().enumerate() {
                            let Some(pane) = self.panes.get(&e.pane) else { continue };
                            let rank = e.status.priority().unwrap_or(9);
                            if group != Some(rank) {
                                group = Some(rank);
                                let count = queue.iter().filter(|x| x.status.priority() == Some(rank)).count();
                                let name = match e.status {
                                    Status::NeedsInput => t("Needs input"),
                                    Status::Failed(_) => t("Failed"),
                                    Status::Done => t("Done"),
                                    _ => t("Command finished"),
                                };
                                let g = kit::section_galley(ui.painter(), &format!("{name} · {count}"), p.text_faint);
                                let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), g.size().y + 14.0), Sense::hover());
                                ui.painter().galley(pos2(r.left() + 10.0, r.top() + 10.0), g, p.text_faint);
                            }
                            let project = self.project(pane.project).map(|x| x.name.clone()).unwrap_or_default();
                            let detail = super::sidebar::attention_what(pane);
                            let detail = match e.status {
                                Status::Failed(_) => match &pane.program_title {
                                    Some(cmd) => format!("{cmd} · {detail}"),
                                    None => detail,
                                },
                                Status::Done if pane.tracker.message.is_some() => format!("“{detail}”"),
                                _ => detail,
                            };
                            let selected = i == sel;
                            let row_h = if selected { 8.0 + 16.0 + 3.0 + 15.0 + 8.0 + 24.0 + 8.0 } else { 8.0 + 16.0 + 3.0 + 15.0 + 8.0 };
                            let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), row_h), Sense::click());
                            #[cfg(debug_assertions)]
                            crate::devtest::record(format!("att:{}", e.pane), rect);
                            let painter = ui.painter();
                            if selected {
                                painter.rect_filled(rect, CornerRadius::same(7), p.accent_soft);
                            } else if resp.hovered() {
                                painter.rect_filled(rect, CornerRadius::same(7), p.faint_row());
                            }
                            let c = status::color(e.status, &p);
                            let l1 = rect.top() + 8.0 + 8.0;
                            painter.text(pos2(rect.left() + 10.0 + 7.0, l1), Align2::CENTER_CENTER, status::icon(e.status), style::icon_font(14.0), c);
                            let x = rect.left() + 10.0 + 14.0 + 10.0;
                            let right = rect.right() - 10.0;
                            let age = painter.layout_no_wrap(status::short_age(e.since), egui::FontId::proportional(style::SMALL), p.text_faint);
                            let aw = age.size().x;
                            painter.galley(pos2(right - aw, l1 - age.size().y / 2.0), age, p.text_faint);
                            let mut job = egui::text::LayoutJob::default();
                            job.append(&project, 0.0, egui::TextFormat::simple(style::medium(12.5), p.text));
                            job.append(&format!("· {}", pane.label()), 6.0, egui::TextFormat::simple(egui::FontId::proportional(style::SMALL), p.text_faint));
                            job.wrap = egui::text::TextWrapping::truncate_at_width((right - aw - 8.0 - x).max(10.0));
                            let g = painter.layout_job(job);
                            painter.galley(pos2(x, l1 - g.size().y / 2.0), g, p.text);
                            let mut job = egui::text::LayoutJob::simple_singleline(detail, egui::FontId::proportional(style::SMALL), p.text_muted);
                            job.wrap = egui::text::TextWrapping::truncate_at_width((right - x).max(10.0));
                            let g = painter.layout_job(job);
                            painter.galley(pos2(x, rect.top() + 8.0 + 16.0 + 3.0), g, p.text_muted);
                            if selected {
                                // What can be done with it.
                                let by = rect.bottom() - 8.0 - 12.0;
                                let mut bx = x;
                                let mut button = |ui: &mut Ui, kind: kit::Kind, icon: Option<&str>, text: &str, keys: Option<&str>| {
                                    let w = kit::button_width(ui, icon, text, keys, true);
                                    let r = Rect::from_min_size(pos2(bx, by - 12.0), vec2(w, 24.0));
                                    bx += w + 6.0;
                                    ui.scope_builder(egui::UiBuilder::new().max_rect(r), |ui| kit::button_keys(ui, kind, icon, text, keys, true)).inner
                                };
                                if button(ui, kit::Kind::Primary, None, t("Jump"), Some("↵")).clicked() {
                                    chosen = Some((i, Choice::Jump));
                                }
                                if e.status == Status::Done {
                                    let r = button(ui, kit::Kind::Secondary, Some(icons::GIT_DIFF), t("Review"), None);
                                    #[cfg(debug_assertions)]
                                    crate::devtest::record(format!("att-review:{}", e.pane), r.rect);
                                    if r.clicked() {
                                        chosen = Some((i, Choice::Review));
                                    }
                                }
                                if button(ui, kit::Kind::Ghost, None, t("Mark as Seen"), None).clicked() {
                                    chosen = Some((i, Choice::Seen));
                                }
                            } else if resp.clicked() {
                                chosen = Some((i, Choice::Jump));
                            } else if resp.hovered() && ui.input(|i| i.pointer.delta() != egui::Vec2::ZERO) {
                                self.attention_sel = i;
                            }
                        }
                        // The keys.
                        ui.add_space(6.0);
                        let (foot, _) = ui.allocate_exact_size(vec2(ui.available_width(), 32.0), Sense::hover());
                        let fill = style::over(p.surface, p.chrome, 0.5);
                        let full = Rect::from_min_max(pos2(foot.left() - 6.0, foot.top()), pos2(foot.right() + 6.0, foot.bottom() + 6.0));
                        ui.painter().rect_filled(full, CornerRadius { nw: 0, ne: 0, sw: 10, se: 10 }, fill);
                        ui.painter().hline(full.x_range(), full.top() + 0.5, Stroke::new(1.0, p.border));
                        paint_key_hints(ui, Rect::from_min_max(pos2(full.left() + 14.0, full.top()), pos2(full.right() - 14.0, full.bottom() - 6.0)), &[
                            (&["↑", "↓"], t("choose")),
                            (&["↵"], t("jump")),
                            (&["R"], t("review")),
                            (&["⌫"], t("seen")),
                        ], Some("esc"));
                    });
            })
            .response;
        if let Some(i) = chosen {
            self.attention_choose(&queue, i, &mut close);
        }
        // A click anywhere else closes it, but for what toggles it (that click, queued this
        // frame, closes it already).
        let toggled = self.actions.iter().any(|a| matches!(a, Action::ToggleAttention));
        if panel.clicked_elsewhere() && !toggled {
            close = true;
        }
        if close {
            self.attention_open = false;
        }
    }

    pub fn toast_ui(&mut self, ctx: &egui::Context) {
        let Some(toast) = self.toast.clone() else { return };
        let shown = toast.at.elapsed();
        let life = if toast.action.is_some() { Duration::from_secs(6) } else { Duration::from_secs(4) };
        if shown > life {
            self.toast = None;
            return;
        }
        ctx.request_repaint_after(life - shown);
        let screen = ctx.content_rect();
        let mut act = None;
        egui::Area::new(egui::Id::new("toast"))
            .order(egui::Order::Tooltip)
            .pivot(Align2::CENTER_BOTTOM)
            .fixed_pos(screen.center_bottom() + vec2(0.0, -super::view::STATUS_H - 16.0))
            .interactable(toast.action.is_some())
            .show(ctx, |ui| {
                let p = style::pal(ui);
                egui::Frame::popup(ui.style())
                    .inner_margin(egui::Margin { left: 12, right: if toast.action.is_some() { 8 } else { 14 }, top: 8, bottom: 8 })
                    .corner_radius(CornerRadius::same(9))
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing.x = 10.0;
                            let (icon, c) = match toast.kind {
                                ToastKind::Info => (icons::INFO, p.text_faint),
                                ToastKind::Ok => (icons::CHECK_CIRCLE, p.ok),
                                ToastKind::Warn => (icons::WARNING, p.warn),
                                ToastKind::Err => (icons::X_CIRCLE, p.err),
                            };
                            ui.label(RichText::new(icon).font(style::icon_font(15.0)).color(c));
                            ui.add(egui::Label::new(RichText::new(&toast.text).size(13.0).color(p.text)).wrap_mode(egui::TextWrapMode::Extend));
                            if let Some((label, action)) = &toast.action
                                && kit::button_ex(ui, kit::Kind::Ghost, None, label, true).clicked()
                            {
                                act = Some(action.clone());
                            }
                        });
                    });
            });
        if let Some(a) = act {
            self.actions.push(a);
            self.toast = None;
        }
    }
}

/// What is done with a row of the attention list.
#[derive(Clone, Copy, PartialEq)]
pub(super) enum Choice {
    Jump,
    Review,
    Seen,
}

impl App {
    /// Does what was chosen for row `i` of the attention list; jumping (or reviewing) closes it.
    pub(super) fn attention_choose(&mut self, queue: &[tc_core::attention::Entry], (i, what): (usize, Choice), close: &mut bool) {
        let Some(e) = queue.get(i) else { return };
        match what {
            Choice::Jump => {
                self.actions.push(Action::FocusPane(e.pane));
                *close = true;
            }
            Choice::Review => {
                self.actions.push(Action::FocusPane(e.pane));
                self.actions.push(Action::Review);
                *close = true;
            }
            Choice::Seen => self.actions.push(Action::MarkSeen(e.pane)),
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
        w += painter.layout_no_wrap(what.to_string(), egui::FontId::proportional(style::SMALL), p.text_faint).size().x + 14.0;
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
        let g = painter.layout_no_wrap(what.to_string(), egui::FontId::proportional(style::SMALL), p.text_faint);
        let w = g.size().x;
        painter.galley(pos2(x, cy - g.size().y / 2.0), g, p.text_faint);
        x += w + 14.0;
    }
    if let Some(k) = right {
        let g = painter.layout_no_wrap(k.to_string(), style::medium(10.5), p.text_muted);
        let w = g.size().x + 12.0;
        kit::paint_kbd_text(painter, pos2(rect.right() - w, cy), k, &p);
    }
}

/// An item of the status bar: an icon, a text, then quieter extra text and a key hint,
/// highlighted on hover when it can be clicked. Texts longer than `max` are cut with "…".
#[allow(clippy::too_many_arguments)]
fn item(ui: &mut Ui, icon: Option<&str>, text: &str, color: egui::Color32, extra: Option<&str>, key: Option<&str>, max: f32, click: bool) -> egui::Response {
    let p = style::pal(ui);
    let pad = 6.0;
    let gap = 5.0;
    let small = egui::FontId::proportional(style::SMALL);
    let icon_g = icon.map(|i| ui.painter().layout_no_wrap(i.to_string(), style::icon_font(12.5), color));
    let extra_g = extra.map(|e| ui.painter().layout_no_wrap(e.to_string(), small.clone(), p.text_faint));
    let key_g = key.filter(|k| !k.is_empty()).map(|k| ui.painter().layout_no_wrap(k.to_string(), small.clone(), p.text_faint));
    let fixed = icon_g.as_ref().map_or(0.0, |g| g.size().x + gap)
        + extra_g.as_ref().map_or(0.0, |g| g.size().x + gap)
        + key_g.as_ref().map_or(0.0, |g| g.size().x + gap)
        + pad * 2.0;
    let mut job = egui::text::LayoutJob::simple_singleline(text.to_string(), small, color);
    job.wrap = egui::text::TextWrapping::truncate_at_width((max - fixed).max(24.0));
    let text_g = ui.painter().layout_job(job);
    let size = vec2(fixed + text_g.size().x, 20.0);
    let (rect, resp) = ui.allocate_exact_size(size, if click { Sense::click() } else { Sense::hover() });
    if ui.is_rect_visible(rect) {
        if click && resp.hovered() {
            ui.painter().rect_filled(rect, CornerRadius::same(5), p.hover);
        }
        let mut x = rect.left() + pad;
        let cy = rect.center().y;
        for (g, c) in [(icon_g, color), (Some(text_g), color), (extra_g, p.text_faint), (key_g, p.text_faint)].into_iter() {
            if let Some(g) = g {
                let w = g.size();
                ui.painter().galley(pos2(x, cy - w.y / 2.0), g, c);
                x += w.x + gap;
            }
        }
    }
    if click && resp.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    resp
}

/// A small dot between groups of the status bar.
fn dot_sep(ui: &mut Ui, p: &style::Palette) {
    let (rect, _) = ui.allocate_exact_size(vec2(9.0, 20.0), Sense::hover());
    ui.painter().circle_filled(rect.center(), 1.5, p.text_faint.gamma_multiply(0.7));
}

