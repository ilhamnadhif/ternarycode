//! The status bar (the most urgent waiting pane, ⌘J, counts and memory), the attention list
//! (FR-N7) and toasts.

use std::time::{Duration, Instant};

use eframe::egui::{self, Align2, CornerRadius, Rect, Sense, Stroke, Ui, pos2, vec2};
use tc_core::status::Status;

use super::{icons, kit, status, style};
use crate::actions::Action;
use crate::app::{App, ToastKind};
use crate::i18n::{t, tf};

/// Width of the attention list.
const LIST_W: f32 = 600.0;
/// Width of a toast.
const TOAST_W: f32 = 380.0;

impl App {
    pub fn status_bar(&mut self, ui: &mut Ui) {
        let p = style::pal(ui);
        let queue = self.attention_queue();
        let bar = ui.max_rect();
        let wide = bar.width() >= 920.0;
        // Right side laid out first so left items never collide or push it off-screen on narrow windows.
        let right_used = ui
            .scope_builder(
                egui::UiBuilder::new()
                    .max_rect(bar)
                    .layout(egui::Layout::right_to_left(egui::Align::Center)),
                |ui| {
                    ui.spacing_mut().item_spacing.x = 2.0;
                    let key = kit::keys(&Action::Palette).unwrap_or_default();
                    let commands = commands_item(ui, &p, key);
                    if commands.on_hover_text(t("Everything TernaryCode can do, searchable")).clicked() {
                        self.actions.push(Action::Palette);
                    }
                    dot_sep(ui, &p);
                    let now = Instant::now();
                    if self.mem_bytes.1.is_none_or(|t| now.duration_since(t) > Duration::from_secs(2)) {
                        self.mem_bytes = (tc_agent::proc::own_resident_bytes(), Some(now));
                        self.browser_bytes = self.browsers_resident_bytes();
                    }
                    let mut mem = if wide {
                        self.mem_bytes.0.map(|b| format!(" · {} MB", b / (1024 * 1024))).unwrap_or_default()
                    } else {
                        String::new()
                    };
                    // The pages' own processes (WebKit's), when there are any.
                    if wide && let Some(b) = self.browser_bytes.filter(|b| *b > 0) {
                        mem.push_str(&tf(" · pages {} MB", &[&(b / (1024 * 1024)).to_string()]));
                    }
                    let counts = format!(
                        "{} · {}{mem}",
                        crate::i18n::count(self.projects.iter().filter(|x| !x.is_worktree()).count(), t("1 project"), t("{} projects")),
                        crate::i18n::count(self.panes.len(), t("1 pane"), t("{} panes")),
                    );
                    let g = ui.painter().layout_no_wrap(counts, style::mono(10.5), p.text_faint);
                    let (r, resp) = ui.allocate_exact_size(vec2(g.size().x + 8.0, 20.0), Sense::hover());
                    ui.painter().galley(pos2(r.left() + 4.0, r.center().y - g.size().y / 2.0), g, p.text_faint);
                    resp.on_hover_text(t("Memory TernaryCode uses (programs in the panes not counted)"));
                    if let Some(target) = self.keyboard_target() {
                        let avail = (ui.min_rect().left() - bar.left() - 160.0).max(0.0);
                        if avail >= 90.0 {
                            dot_sep(ui, &p);
                            let keys = kit::keys(&Action::OpenGridTile);
                            if item(ui, Some(icons::KEYBOARD), &target, p.accent_text, None, keys, avail.min(280.0), true)
                                .on_hover_text(t("Where your typing goes. Click (or press the keys) to open it in Focus."))
                                .clicked()
                            {
                                self.actions.push(Action::OpenGridTile);
                            }
                        }
                    }
                    self.update_status_item(ui);
                    ui.min_rect()
                },
            )
            .inner;

        let left_max = (right_used.left() - 8.0).max(bar.left());
        let left_rect = Rect::from_min_max(bar.min, pos2(left_max, bar.bottom()));
        let unconnected = self.unconnected_agents();
        ui.scope_builder(
            egui::UiBuilder::new()
                .max_rect(left_rect)
                .layout(egui::Layout::left_to_right(egui::Align::Center)),
            |ui| {
                ui.set_clip_rect(left_rect.intersect(ui.clip_rect()));
                ui.spacing_mut().item_spacing.x = 2.0;
                let left_w = left_rect.width();
                let unc_reserve = if unconnected.is_empty() { 0.0 } else { 145.0_f32.min(left_w * 0.42) };
                let lead_room = (left_w - unc_reserve).max(80.0);
                // Left: the most urgent pane waiting for the user ("Next:" when that one is the pane
                // in view: ⌘J goes past it), or that none is.
                let here = if self.view == tc_core::model::View::Focus { self.current_pane() } else { None };
                let next = queue.iter().find(|e| Some(e.pane) != here).or(queue.first());
                match next.and_then(|e| self.panes.get(&e.pane).map(|pane| (e, pane))) {
                    Some((e, pane)) => {
                        let s = pane.tracker.status;
                        let project = self.project_label(pane.project);
                        let skipped = queue.first().is_some_and(|f| f.pane != e.pane);
                        let (who, what, code) = super::sidebar::attention_sentence(pane);
                        let more = queue.len() - 1 - usize::from(skipped);
                        let r = lead(ui, &p, s, skipped, &project, &who, &what, code.as_deref(), more, lead_room);
                        #[cfg(debug_assertions)]
                        crate::devtest::record("attention-summary", r.rect);
                        let tip = format!("{}\n{}", status::described(&pane.tracker), t("Show everything that waits for you"));
                        if r.on_hover_text(tip).clicked() {
                            self.actions.push(Action::ToggleAttention);
                        }
                    }
                    None => {
                        let r = item(ui, Some(icons::CHECK_CIRCLE), t("No agent is waiting for you."), p.text_faint, None, None, lead_room.min(400.0), true);
                        if r.on_hover_text(t("Agents that need you show up here")).clicked() {
                            self.actions.push(Action::ToggleAttention);
                        }
                    }
                }
                // Agents that can't say when they need the user (started without the hooks).
                if !unconnected.is_empty() {
                    let rem = (left_rect.right() - ui.cursor().min.x - 14.0).max(0.0);
                    if rem >= 72.0 {
                        dot_sep(ui, &p);
                        let text = crate::i18n::count(unconnected.len(), t("1 agent not connected"), t("{} agents not connected"));
                        let r = item(ui, Some(icons::PLUGS_CONNECTED), &text, p.warn, None, None, rem.min(260.0), true);
                        #[cfg(debug_assertions)]
                        crate::devtest::record("unconnected", r.rect);
                        if r.on_hover_text(self.unconnected_tip(&unconnected)).clicked() {
                            self.connect_agents(&unconnected);
                        }
                    }
                }
            },
        );
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
        let on_top = self.dialog.is_none()
            && self.palette.is_none()
            && !self.updater.dialog_open
            && !self.monitor_popover;
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
        let dark = ctx.global_style().visuals.dark_mode;
        let list_w = (ctx.content_rect().width() - 32.0).clamp(320.0, LIST_W);
        let area = egui::Area::new(egui::Id::new("attention-panel")).anchor(Align2::CENTER_TOP, vec2(0.0, 96.0));
        let modal = egui::Modal::new(egui::Id::new("attention-modal"))
            .area(area)
            .backdrop_color(egui::Color32::from_black_alpha(if dark { 90 } else { 40 }))
            .frame(super::dialogs::frame(ctx).inner_margin(egui::Margin::ZERO))
            .show(ctx, |ui| {
                let p = style::pal(ui);
                ui.set_width(list_w);
                ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
                let top = ui.max_rect();
                kit::paint_highlight(ui.painter(), Rect::from_min_size(top.min, vec2(list_w, 1.0)), style::RADIUS_DIALOG, &p);
                // Header: the bell, "Waiting for you", how many; at the right the order and the keys.
                let (head, _) = ui.allocate_exact_size(vec2(list_w, 52.0), Sense::hover());
                let painter = ui.painter();
                let cy = head.center().y;
                status::paint_glyph(painter, pos2(head.left() + 18.0 + 7.0, cy), Status::NeedsInput, &p, 14.0);
                let g = painter.layout_no_wrap(t("Waiting for you").to_string(), style::medium(14.0), p.text);
                let gw = g.size().x;
                painter.galley(pos2(head.left() + 18.0 + 14.0 + 10.0, cy - g.size().y / 2.0), g, p.text);
                if n > 0 {
                    let x = head.left() + 18.0 + 14.0 + 10.0 + gw + 8.0;
                    let w = kit::paint_badge(painter, pos2(10_000.0, cy), &n.to_string(), egui::Color32::TRANSPARENT, egui::Color32::TRANSPARENT, 16.0).width();
                    kit::paint_badge(painter, pos2(x + w, cy), &n.to_string(), p.warn, kit::badge_text(p.warn, &p, dark), 16.0);
                }
                let mut rx = head.right() - 16.0;
                if let Some(k) = kit::keys(&Action::ToggleAttention) {
                    let kw = kit::kbd_width(painter, k);
                    rx -= kw;
                    kit::paint_kbd_text(painter, pos2(rx, cy), k, &p);
                    rx -= 8.0;
                }
                if n > 0 {
                    let all = Rect::from_center_size(pos2(rx - 11.0, cy), vec2(22.0, 22.0));
                    if ui.scope_builder(egui::UiBuilder::new().max_rect(all), |ui| kit::icon_button_ex(ui, icons::CHECK, t("Mark All Seen"), false, 22.0)).inner.clicked() {
                        self.actions.push(Action::MarkAllSeen);
                    }
                    rx = all.left() - 8.0;
                    if list_w >= 460.0 {
                        let g = ui.painter().layout_no_wrap(t("oldest first").to_string(), egui::FontId::proportional(11.5), p.text_faint);
                        ui.painter().galley(pos2(rx - g.size().x, cy - g.size().y / 2.0), g.clone(), p.text_faint);
                    }
                }
                ui.painter().hline(head.x_range(), head.bottom() - 0.5, Stroke::new(1.0, p.border));
                ui.add_space(6.0);
                if queue.is_empty() {
                    let (r, _) = ui.allocate_exact_size(vec2(list_w, 40.0), Sense::hover());
                    ui.painter().text(pos2(r.left() + 18.0, r.center().y), Align2::LEFT_CENTER, t("No agent is waiting for you."), egui::FontId::proportional(13.0), p.text_muted);
                }
                // Two groups, in the queue's order: what needs the user, what finished.
                let max_h = (ctx.content_rect().height() - 240.0).clamp(180.0, 480.0);
                egui::ScrollArea::vertical()
                    .id_salt("attention-modal-scroll")
                    .max_height(max_h)
                    .auto_shrink([false, true])
                    .show(ui, |ui| {
                let mut group: Option<bool> = None;
                for (i, e) in queue.iter().enumerate() {
                    let Some(pane) = self.panes.get(&e.pane) else { continue };
                    let needs = matches!(e.status, Status::NeedsInput | Status::Failed(_));
                    if group != Some(needs) {
                        group = Some(needs);
                        let count = queue.iter().filter(|x| matches!(x.status, Status::NeedsInput | Status::Failed(_)) == needs).count();
                        let name = if needs { t("Needs you") } else { t("Finished") };
                        let (r, _) = ui.allocate_exact_size(vec2(list_w, 28.0), Sense::hover());
                        let g = kit::section_galley(ui.painter(), name, p.text_faint);
                        let gw = g.size().x;
                        ui.painter().galley(pos2(r.left() + 18.0, r.bottom() - 6.0 - g.size().y), g, p.text_faint);
                        ui.painter().text(pos2(r.left() + 18.0 + gw + 6.0, r.bottom() - 6.0), Align2::LEFT_BOTTOM, count.to_string(), style::mono(10.5), p.text_ghost);
                    }
                    let project = self.project_label(pane.project);
                    let (letters, color) = self.project(pane.project).map(|pr| (pr.letters(), pr.color32())).unwrap_or_default();
                    let what = super::sidebar::attention_short(pane);
                    let agent = pane.tracker.agent.filter(|a| super::agent_logo::has(*a));
                    let selected = i == sel;
                    let (full, _) = ui.allocate_exact_size(vec2(list_w, 52.0 + if selected { 36.0 } else { 0.0 }), Sense::hover());
                    let rect = Rect::from_min_size(pos2(full.left() + 8.0, full.top()), vec2(list_w - 16.0, 52.0));
                    let resp = ui.interact(rect, egui::Id::new(("att-row", e.pane)), Sense::click());
                    #[cfg(debug_assertions)]
                    crate::devtest::record(format!("att:{}", e.pane), rect);
                    let painter = ui.painter();
                    if selected {
                        painter.rect(rect, CornerRadius::same(9), if dark { p.subtle } else { egui::Color32::WHITE }, Stroke::new(1.0, if dark { p.border } else { p.border_strong }), egui::StrokeKind::Inside);
                    } else if resp.hovered() {
                        painter.rect_filled(rect, CornerRadius::same(9), p.hover);
                    }
                    let cy = rect.center().y;
                    status::paint_glyph(painter, pos2(rect.left() + 10.0 + 9.0, cy), e.status, &p, 14.0);
                    kit::paint_project_tile(painter, Rect::from_min_size(pos2(rect.left() + 10.0 + 18.0 + 10.0, cy - 12.0), vec2(24.0, 24.0)), &letters, color, dark);
                    let x = rect.left() + 10.0 + 18.0 + 10.0 + 24.0 + 10.0;
                    let right = rect.right() - 10.0;
                    let age = painter.layout_no_wrap(status::short_age(e.since), style::mono(10.5), p.text_faint);
                    let aw = age.size().x;
                    painter.galley(pos2(right - aw, cy - age.size().y / 2.0), age, p.text_faint);
                    let mut job = egui::text::LayoutJob::simple_singleline(project, style::medium(13.0), p.text);
                    job.wrap = egui::text::TextWrapping::truncate_at_width((right - aw - 8.0 - x).max(10.0));
                    let g = painter.layout_job(job);
                    painter.galley(pos2(x, cy - 9.0 - g.size().y / 2.0), g, p.text);
                    let mut wx = x;
                    if let Some(a) = agent {
                        super::agent_logo::paint(painter, Rect::from_center_size(pos2(wx + 6.0, cy + 10.0), vec2(12.0, 12.0)), a, p.text_muted, false);
                        wx += 12.0 + 6.0;
                    }
                    let mut job = egui::text::LayoutJob::simple_singleline(what, egui::FontId::proportional(12.0), p.text_muted);
                    job.wrap = egui::text::TextWrapping::truncate_at_width((right - aw - 8.0 - wx).max(10.0));
                    let g = painter.layout_job(job);
                    painter.galley(pos2(wx, cy + 10.0 - g.size().y / 2.0), g, p.text_muted);
                    if selected {
                        // What can be done with it, under the row.
                        let by = rect.bottom() + 18.0;
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
                            let r = button(ui, kit::Kind::Secondary, Some(icons::GIT_DIFF), t("Review"), Some("R"));
                            #[cfg(debug_assertions)]
                            crate::devtest::record(format!("att-review:{}", e.pane), r.rect);
                            if r.clicked() {
                                chosen = Some((i, Choice::Review));
                            }
                        }
                        if button(ui, kit::Kind::Secondary, None, t("Mark as Seen"), Some("⌫")).clicked() {
                            chosen = Some((i, Choice::Seen));
                        }
                    }
                    if resp.clicked() {
                        chosen = Some((i, Choice::Jump));
                    } else if resp.hovered() && ui.input(|i| i.pointer.delta() != egui::Vec2::ZERO) {
                        self.attention_sel = i;
                    }
                }
                });
                // The keys.
                ui.add_space(8.0);
                let (foot, _) = ui.allocate_exact_size(vec2(list_w, 38.0), Sense::hover());
                ui.painter().hline(foot.x_range(), foot.top() + 0.5, Stroke::new(1.0, p.border));
                let foot_inner = Rect::from_min_max(pos2(foot.left() + 14.0, foot.top()), pos2(foot.right() - 14.0, foot.bottom()));
                let mut hints: Vec<(&[&str], &str)> = vec![
                    (&["↑", "↓"], t("choose")),
                    (&["↵"], t("open")),
                    (&["R"], t("review")),
                    (&["⌫"], t("mark as seen")),
                ];
                let esc_reserve = 78.0;
                while hints.len() > 1 && key_hints_width(ui, &hints) + esc_reserve > foot_inner.width() {
                    hints.pop();
                }
                paint_key_hints(ui, foot_inner, &hints, Some("esc"));
            });
        if let Some(i) = chosen {
            self.attention_choose(&queue, i, &mut close);
        }
        // A click anywhere else closes it, but for what toggles it (that click, queued this
        // frame, closes it already).
        let toggled = self.actions.iter().any(|a| matches!(a, Action::ToggleAttention));
        if modal.should_close() && !toggled {
            close = true;
        }
        if close {
            self.attention_open = false;
        }
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
                self.actions.push(Action::ReviewAgent);
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

/// An item of the status bar: an icon, a text, then quieter extra text and a key hint,
/// highlighted on hover when it can be clicked. Texts longer than `max` are cut with "…".
#[allow(clippy::too_many_arguments)]
fn item(ui: &mut Ui, icon: Option<&str>, text: &str, color: egui::Color32, extra: Option<&str>, key: Option<&str>, max: f32, click: bool) -> egui::Response {
    let p = style::pal(ui);
    let pad = 6.0;
    let gap = 5.0;
    let small = egui::FontId::proportional(style::LABEL);
    let icon_g = icon.map(|i| ui.painter().layout_no_wrap(i.to_string(), style::icon_font(12.0), color));
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

/// A quiet "·" between groups of the status bar.
fn dot_sep(ui: &mut Ui, p: &style::Palette) {
    let (rect, _) = ui.allocate_exact_size(vec2(12.0, 20.0), Sense::hover());
    ui.painter().text(rect.center(), Align2::CENTER_CENTER, "·", egui::FontId::proportional(style::LABEL), p.text_ghost);
}

/// "⌕ Commands ⌘K" at the right end of the status bar.
fn commands_item(ui: &mut Ui, p: &style::Palette, key: &str) -> egui::Response {
    let font = egui::FontId::proportional(style::LABEL);
    let label = ui.painter().layout_no_wrap(t("Commands").to_string(), font, p.text_muted);
    let kw = if key.is_empty() { 0.0 } else { kit::kbd_width(ui.painter(), key).max(16.0) + 6.0 };
    let (rect, resp) = ui.allocate_exact_size(vec2(6.0 + 12.0 + 6.0 + label.size().x + kw + 6.0, 20.0), Sense::click());
    if resp.hovered() {
        ui.painter().rect_filled(rect, CornerRadius::same(5), p.hover);
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    let cy = rect.center().y;
    ui.painter().text(pos2(rect.left() + 6.0 + 6.0, cy), Align2::CENTER_CENTER, icons::MAGNIFYING_GLASS, style::icon_font(12.0), p.text_muted);
    let lw = label.size().x;
    ui.painter().galley(pos2(rect.left() + 24.0, cy - label.size().y / 2.0), label, if resp.hovered() { p.text } else { p.text_muted });
    if !key.is_empty() {
        kit::paint_kbd_sized(ui.painter(), pos2(rect.left() + 24.0 + lw + 6.0, cy), key, p, 16.0);
    }
    resp
}

/// The status bar's lead: the status glyph, the project, what waits there ("Claude Code wants to
/// run `flutter test`"), how many more, and ⌘J (with "Next:" and ⇧⌘J when the pane in view was
/// skipped).
#[allow(clippy::too_many_arguments)]
fn lead(ui: &mut Ui, p: &style::Palette, s: Status, next: bool, project: &str, who: &str, what: &str, code: Option<&str>, more: usize, max: f32) -> egui::Response {
    let font = egui::FontId::proportional(style::LABEL);
    let mut job = egui::text::LayoutJob::default();
    let fmt = |f: egui::FontId, c: egui::Color32| egui::TextFormat::simple(f, c);
    if next {
        job.append(t("Next:"), 0.0, fmt(font.clone(), p.text_faint));
        job.append(project, 6.0, fmt(style::medium(style::LABEL), p.text));
    } else {
        job.append(project, 0.0, fmt(style::medium(style::LABEL), p.text));
    }
    job.append("·", 6.0, fmt(font.clone(), p.text_faint));
    job.append(&format!("{who} {what}"), 6.0, fmt(font.clone(), p.text_muted));
    if let Some(code) = code {
        job.append(code, 5.0, fmt(style::mono(10.5), p.text));
    }
    if more > 0 {
        job.append(&crate::i18n::tf("+{} more", &[&more.to_string()]), 6.0, fmt(font.clone(), p.text_faint));
    }
    let keys = kit::keys(&Action::NextAttention).unwrap_or_default();
    let back = if next { kit::keys(&Action::BackAttention) } else { None };
    let kw = |k: &str| kit::kbd_width(ui.painter(), k).max(16.0);
    let tail = if keys.is_empty() { 0.0 } else { 6.0 + kw(keys) } + back.map_or(0.0, |b| 8.0 + kw(b) + 4.0 + ui.painter().layout_no_wrap(t("back").to_string(), font.clone(), p.text_ghost).size().x);
    job.wrap = egui::text::TextWrapping::truncate_at_width((max - 18.0 - tail - 8.0).max(40.0));
    let g = ui.painter().layout_job(job);
    let (rect, resp) = ui.allocate_exact_size(vec2(4.0 + 12.0 + 6.0 + g.size().x + tail + 6.0, 20.0), Sense::click());
    if resp.hovered() {
        ui.painter().rect_filled(rect, CornerRadius::same(5), p.hover);
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    let cy = rect.center().y;
    super::status::paint_glyph(ui.painter(), pos2(rect.left() + 4.0 + 6.0, cy), s, p, 12.0);
    let mut x = rect.left() + 4.0 + 12.0 + 6.0;
    let gw = g.size().x;
    ui.painter().galley(pos2(x, cy - g.size().y / 2.0), g, p.text);
    x += gw;
    if !keys.is_empty() {
        x += 6.0;
        x += kit::paint_kbd_sized(ui.painter(), pos2(x, cy), keys, p, 16.0);
    }
    if let Some(b) = back {
        x += 8.0;
        x += kit::paint_kbd_sized(ui.painter(), pos2(x, cy), b, p, 16.0) + 4.0;
        ui.painter().text(pos2(x, cy), Align2::LEFT_CENTER, t("back"), font, p.text_ghost);
    }
    resp
}

