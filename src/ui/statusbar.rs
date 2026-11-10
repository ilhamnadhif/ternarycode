//! The status bar (the most urgent waiting pane, ⌘J, counts and memory), the attention list
//! (FR-N7) and toasts.

use std::time::{Duration, Instant};

use eframe::egui::{self, Align2, RichText, Sense, Ui, vec2};
use tc_core::status::Status;

use super::{icons, kit, status, style};
use crate::actions::Action;
use crate::app::App;
use crate::i18n::{t, tf};

pub fn jump_key() -> &'static str {
    if cfg!(target_os = "macos") { "⌘J" } else { "Ctrl+Shift+J" }
}

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
                    let what = pane
                        .tracker
                        .message
                        .clone()
                        .filter(|_| s == Status::NeedsInput)
                        .unwrap_or_else(|| status::text(s));
                    let more = if queue.len() > 1 {
                        tf(" (+{} more)", &[&(queue.len() - 1).to_string()])
                    } else {
                        String::new()
                    };
                    let text = format!("{project} · {}: {what}{more}", pane.label());
                    let room = (ui.available_width() * 0.6).max(120.0);
                    let r = bar_item(ui, Some(status::icon(s)), &text, c, Some(jump_key()), room);
                    #[cfg(debug_assertions)]
                    crate::devtest::record("attention-summary", r.rect);
                    let tip = format!("{}\n{}", status::described(&pane.tracker), t("Show everything that waits for you"));
                    if r.on_hover_text(tip).clicked() {
                        self.actions.push(Action::ToggleAttention);
                    }
                }
                None => {
                    let r = bar_item(ui, Some(icons::CHECK_CIRCLE), t("No agent is waiting for you."), p.text_faint, None, 400.0);
                    if r.on_hover_text(t("Agents that need you show up here")).clicked() {
                        self.actions.push(Action::ToggleAttention);
                    }
                }
            }
            // Right: commands, then what is running and the memory it takes.
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = 2.0;
                let key = kit::keys(&Action::Palette).unwrap_or_default();
                let commands = bar_item(ui, Some(icons::MAGNIFYING_GLASS), t("Commands"), p.text_muted, Some(key), 200.0);
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
                bar_text(ui, None, &counts, p.text_faint, 280.0)
                    .on_hover_text(t("Memory TernaryCode uses (programs in the panes not counted)"));
                if let Some(target) = self.keyboard_target() {
                    dot_sep(ui, &p);
                    bar_text(ui, Some(icons::KEYBOARD), &target, p.accent_text, 260.0)
                        .on_hover_text(t("Where your typing goes"));
                }
                self.update_status_item(ui);
            });
        });
    }

    pub fn attention_panel(&mut self, ctx: &egui::Context) {
        if !self.attention_open {
            return;
        }
        let queue = self.attention_queue();
        let mut close = false;
        let screen = ctx.content_rect();
        let panel = egui::Area::new(egui::Id::new("attention-panel"))
            .order(egui::Order::Foreground)
            .pivot(Align2::LEFT_BOTTOM)
            .fixed_pos(screen.left_bottom() + vec2(8.0, -super::view::STATUS_H - 4.0))
            .show(ctx, |ui| {
                egui::Frame::popup(ui.style())
                    .inner_margin(egui::Margin::same(10))
                    .corner_radius(egui::CornerRadius::same(10))
                    .show(ui, |ui| {
                        ui.set_width(440.0);
                        let p = style::pal(ui);
                        ui.horizontal(|ui| {
                            ui.label(RichText::new(t("Waiting for you")).font(style::semibold(13.0)));
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                if kit::icon_button(ui, icons::X, t("Close")).clicked() {
                                    close = true;
                                }
                            });
                        });
                        ui.add_space(6.0);
                        if queue.is_empty() {
                            ui.label(RichText::new(t("No agent is waiting for you.")).color(p.text_muted));
                        }
                        for e in &queue {
                            let Some(pane) = self.panes.get(&e.pane) else { continue };
                            let project = self.project(pane.project).map(|x| x.name.clone()).unwrap_or_default();
                            let c = status::color(e.status, &p);
                            // The hover fill goes under the row, known only once the row is laid out.
                            let fill = ui.painter().add(egui::Shape::Noop);
                            let r = ui
                                .horizontal(|ui| {
                                    ui.spacing_mut().item_spacing.x = 8.0;
                                    kit::icon(ui, status::icon(e.status), 15.0, c);
                                    ui.vertical(|ui| {
                                        ui.label(RichText::new(format!("{project} · {}", pane.label())).font(style::medium(12.5)));
                                        let detail = pane
                                            .tracker
                                            .message
                                            .clone()
                                            .unwrap_or_else(|| status::text(e.status));
                                        ui.label(
                                            RichText::new(format!("{detail} · {}", status::age(e.since)))
                                                .size(style::SMALL)
                                                .color(p.text_muted),
                                        );
                                    });
                                })
                                .response;
                            let r = ui.interact(r.rect, egui::Id::new(("att", e.pane)), Sense::click());
                            if r.hovered() {
                                ui.painter().set(
                                    fill,
                                    egui::Shape::rect_filled(r.rect.expand(2.0), egui::CornerRadius::same(6), p.faint_row()),
                                );
                            }
                            if r.clicked() {
                                self.actions.push(Action::FocusPane(e.pane));
                                close = true;
                            }
                            ui.add_space(4.0);
                        }
                    });
            })
            .response;
        // A click anywhere else closes it, but for the status bar item that toggles it (that
        // click, queued this frame, closes it already).
        let toggled = self.actions.iter().any(|a| matches!(a, Action::ToggleAttention));
        if panel.clicked_elsewhere() && !toggled {
            close = true;
        }
        // Esc closes it and goes no further. Not while a dialog, the palette or the Review is
        // over it: that Esc is theirs.
        let on_top = self.dialog.is_none() && self.palette.is_none() && self.review.is_none();
        if on_top && ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape)) {
            close = true;
        }
        if close {
            self.attention_open = false;
        }
    }

    pub fn toast_ui(&mut self, ctx: &egui::Context) {
        let Some((text, at)) = self.toast.clone() else { return };
        let shown = at.elapsed();
        if shown > Duration::from_secs(4) {
            self.toast = None;
            return;
        }
        ctx.request_repaint_after(Duration::from_secs(4) - shown);
        let screen = ctx.content_rect();
        egui::Area::new(egui::Id::new("toast"))
            .order(egui::Order::Tooltip)
            .pivot(Align2::CENTER_BOTTOM)
            .fixed_pos(screen.center_bottom() + vec2(0.0, -super::view::STATUS_H - 16.0))
            .interactable(false)
            .show(ctx, |ui| {
                egui::Frame::popup(ui.style())
                    .inner_margin(egui::Margin::symmetric(14, 8))
                    .corner_radius(egui::CornerRadius::same(8))
                    .show(ui, |ui| {
                        ui.label(RichText::new(text).size(style::BODY));
                    });
            });
    }
}

/// A clickable item of the status bar: an icon, a text and a shortcut hint, highlighted on
/// hover. Texts longer than `max` are cut with "…".
fn bar_item(ui: &mut Ui, icon: Option<&str>, text: &str, color: egui::Color32, key: Option<&str>, max: f32) -> egui::Response {
    item(ui, icon, text, color, key, max, true)
}

/// A status bar text that does nothing when clicked.
fn bar_text(ui: &mut Ui, icon: Option<&str>, text: &str, color: egui::Color32, max: f32) -> egui::Response {
    item(ui, icon, text, color, None, max, false)
}

fn item(ui: &mut Ui, icon: Option<&str>, text: &str, color: egui::Color32, key: Option<&str>, max: f32, click: bool) -> egui::Response {
    let p = style::pal(ui);
    let pad = 6.0;
    let gap = 5.0;
    let icon_g = icon.map(|i| ui.painter().layout_no_wrap(i.to_string(), style::icon_font(12.5), color));
    let key_g = key.filter(|k| !k.is_empty()).map(|k| ui.painter().layout_no_wrap(k.to_string(), egui::FontId::proportional(style::SMALL), p.text_faint));
    let fixed = icon_g.as_ref().map_or(0.0, |g| g.size().x + gap) + key_g.as_ref().map_or(0.0, |g| g.size().x + gap) + pad * 2.0;
    let mut job = egui::text::LayoutJob::simple_singleline(text.to_string(), egui::FontId::proportional(style::SMALL), color);
    job.wrap = egui::text::TextWrapping::truncate_at_width((max - fixed).max(24.0));
    let text_g = ui.painter().layout_job(job);
    let size = vec2(fixed + text_g.size().x, 20.0);
    let (rect, resp) = ui.allocate_exact_size(size, if click { Sense::click() } else { Sense::hover() });
    if ui.is_rect_visible(rect) {
        if click && resp.hovered() {
            ui.painter().rect_filled(rect, egui::CornerRadius::same(5), p.hover);
        }
        let mut x = rect.left() + pad;
        let cy = rect.center().y;
        if let Some(g) = icon_g {
            let w = g.size();
            ui.painter().galley(egui::pos2(x, cy - w.y / 2.0), g, color);
            x += w.x + gap;
        }
        let w = text_g.size();
        ui.painter().galley(egui::pos2(x, cy - w.y / 2.0), text_g, color);
        x += w.x + gap;
        if let Some(g) = key_g {
            let w = g.size();
            ui.painter().galley(egui::pos2(x, cy - w.y / 2.0), g, p.text_faint);
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
    ui.painter().circle_filled(rect.center(), 1.3, p.text_faint.gamma_multiply(0.7));
}
